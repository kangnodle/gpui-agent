use anyhow::Result;
use gpui_agent_application::ports::JobRepository;
use gpui_agent_contracts::*;
use gpui_agent_infrastructure::{JobStore,
                                Settings};

fn data() -> PullRequestData {
    PullRequestData {
        snapshot: PullRequestSnapshot {
            title: "Fix".into(),
            head_sha: "1".repeat(40),
            base_sha: "a".repeat(40),
            head_branch: "fix".into(),
            base_branch: "main".into(),
            state: PullRequestState::Open,
            draft: false,
            conflicting: Some(false),
            checks: vec![PullRequestCheck {
                name: "test".into(),
                state: "pending".into(),
            }],
            remarks: vec![],
        },
        description: "Review this".into(),
        diff: Some("+fixed".into()),
        viewer: Some("me".into()),
    }
}
#[tokio::test]
async fn watch_baselines_and_notifications_commit_once_survive_restart_and_rollback_when_the_queue_is_full() -> Result<()> {
    // Given persisted watches for all providers in a real host database.
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let catalog = workspaces.catalog().await?;
    let workspace = &catalog.projects[0].workspaces[0];
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    let target = PullRequestTarget::parse(None, Some("owner/repo"), Some(7)).map_err(anyhow::Error::msg)?;
    let mut scopes = vec![];
    for provider in PROVIDERS {
        let scope = JobScope {
            provider,
            account_id: "default".into(),
            workspace_id: Some(workspace.id),
            workspace_path: workspace.path.clone(),
        };
        store.set_pull_request_watch(&scope, 1, &target, Some(&data()), 100)?;
        scopes.push(scope);
    }
    assert!(store.due_pull_requests(219)?.is_empty());
    assert_eq!(store.due_pull_requests(220)?.len(), 3);
    for link in store.due_pull_requests(220)? {
        let mut snapshot = data().snapshot;
        snapshot.checks[0].state = "failed".into();
        snapshot.conflicting = Some(true);
        let (next, request) = link.observe(snapshot, 220);
        // When competing polls report the same failure, then only one turn is
        // saved.
        let job = store.apply_pull_request_poll(&link, &next, request.as_ref())?.unwrap();
        assert!(store.apply_pull_request_poll(&link, &next, request.as_ref())?.is_none());
        assert_eq!(store.get(job.id)?.request.conversation_id, Some(1));
        assert_eq!(store.get(job.id)?.request.claude_permission_mode, None);
    }
    assert_eq!(store.list()?.len(), 3);
    let scope = &scopes[0];
    // Queue admission failure must leave the baseline and deadline retryable.
    let mut current = data();
    current.snapshot.head_sha = "2".repeat(40);
    store.set_pull_request_watch(scope, 2, &target, Some(&current), 100)?;
    let original = store.pull_request_links(scope, 2)?.remove(0);
    let queued = |store: &JobStore| -> anyhow::Result<Vec<i64>> {
        let mut ids: Vec<_> = store
            .list()?
            .into_iter()
            .filter(|job| job.scope == *scope && job.state == gpui_agent_contracts::JobState::Queued)
            .map(|job| job.id)
            .collect();
        ids.sort();
        Ok(ids)
    };
    while queued(&store)?.len() < 32 {
        store.enqueue(scope, &current.guide_request(&target).map_err(anyhow::Error::msg)?)?;
    }
    current.snapshot.checks[0].state = "failed".into();
    let (next, notification) = original.observe(current.snapshot.clone(), 220);
    assert!(store.apply_pull_request_poll(&original, &next, notification.as_ref()).is_err());
    assert_eq!(store.pull_request_links(scope, 2)?[0].revision, original.revision);
    assert_eq!(store.pull_request_links(scope, 2)?[0].snapshot.as_ref().unwrap().checks[0].state, "pending");
    let first = queued(&store)?[0];
    store.cancel_queued(first)?;
    assert!(store.apply_pull_request_poll(&original, &next, notification.as_ref())?.is_some());
    // A completed read racing with unwatch cannot rearm or enqueue a
    // notification.
    let stale = store.pull_request_links(scope, 2)?.remove(0);
    store.stop_pull_request_watches(scope, 2)?;
    let (next, request) = stale.observe(data().snapshot, 500);
    assert!(store.apply_pull_request_poll(&stale, &next, request.as_ref())?.is_none());
    drop(store);
    let store = JobStore::open(settings, workspaces)?;
    assert!(store.pull_request_links(scope, 2)?[0].watch.is_none());
    assert_eq!(store.pull_request_links(&scopes[1], 1)?[0].snapshot.as_ref().unwrap().checks[0].state, "failed");
    assert_eq!(store.due_pull_requests(500)?.len(), 2);
    // Cancelling a conversation job through the shared queue port also disarms
    // its watch, so the next poll cannot immediately wake it again.
    let cancelled_scope = &scopes[2];
    store.set_pull_request_watch(cancelled_scope, 3, &target, Some(&data()), 100)?;
    let previous = store.pull_request_links(cancelled_scope, 3)?.remove(0);
    let mut changed = data().snapshot;
    changed.checks[0].state = "failed".into();
    let (next, notification) = previous.observe(changed, 500);
    let alert = store.apply_pull_request_poll(&previous, &next, notification.as_ref())?.unwrap();
    store.cancel_queued(alert.id)?;
    assert!(store.pull_request_links(cancelled_scope, 3)?[0].watch.is_none());
    // Rate limits postpone without consuming the consecutive-error budget.
    let original = store.pull_request_links(&scopes[2], 1)?.remove(0);
    let delayed = original.read_failed(500, true);
    store.apply_pull_request_poll(&original, &delayed, None)?;
    let delayed = store.pull_request_links(&scopes[2], 1)?.remove(0);
    assert_eq!(delayed.watch.as_ref().unwrap().failures, 0);
    assert_eq!(delayed.next_poll, Some(1100));
    for n in 0 .. 8 {
        let previous = store.pull_request_links(&scopes[2], 1)?.remove(0);
        store.apply_pull_request_poll(&previous, &previous.read_failed(1100 + n * 120, false), None)?;
    }
    assert!(store.pull_request_links(&scopes[2], 1)?[0].watch.is_none());
    assert!(store.pull_request_links(&scopes[2], 1)?[0].error.as_ref().unwrap().contains("8 consecutive"));
    // A chatty reviewer cannot keep waking an agent indefinitely without
    // progress.
    for n in 0 .. 10 {
        let previous = store.pull_request_links(&scopes[1], 1)?.remove(0);
        let mut current = previous.snapshot.clone().unwrap();
        current.remarks.push(PullRequestRemark {
            id: format!("comment-{n}"),
            at: format!("{n}"),
            author: "reviewer".into(),
            text: "Another remark".into(),
        });
        let (next, request) = previous.observe(current, 600 + n * 120);
        store.apply_pull_request_poll(&previous, &next, request.as_ref())?;
    }
    assert!(store.pull_request_links(&scopes[1], 1)?[0].watch.is_none());
    // Local unlink/delete only removes this conversation's metadata.
    for job in store.list()? {
        if job.scope == *scope && !job.state.terminal() {
            store.cancel_queued(job.id)?;
        }
    }
    store.purge_conversation(scope.provider, &scope.account_id, workspace.id, 1)?;
    assert!(store.pull_request_links(scope, 1)?.is_empty());
    assert_eq!(store.pull_request_links(&scopes[1], 1)?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn guide_jobs_deduplicate_by_scope_and_head_and_recover_their_result() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let workspace = workspaces.catalog().await?.projects[0].workspaces[0].clone();
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    let target = PullRequestTarget::parse(None, Some("owner/repo"), Some(7)).map_err(anyhow::Error::msg)?;
    let mut scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspace.id),
        workspace_path: workspace.path,
    };
    let data = data();
    let request = data.guide_request(&target).map_err(anyhow::Error::msg)?;
    let (run, started) = store.start_pull_request_guide(&scope, &target, &data.snapshot.head_sha, &data.snapshot.base_sha, &request)?;
    assert!(started);
    let retry = store.start_pull_request_guide(&scope, &target, &data.snapshot.head_sha, &data.snapshot.base_sha, &request)?;
    assert!(!retry.1);
    assert_eq!(retry.0.job.id, run.job.id);
    store.append(
        run.job.id,
        &StreamEvent::Completed {
            session_id: "guide".into(),
            text: "Review cancellation first".into(),
        },
    )?;
    scope.provider = Provider::Claude;
    assert!(store.pull_request_guide(&scope, &target)?.is_none());
    let other = store
        .start_pull_request_guide(&scope, &target, &data.snapshot.head_sha, &data.snapshot.base_sha, &request)?
        .0;
    assert_ne!(other.job.id, run.job.id);
    scope.provider = Provider::Codex;
    drop(store);
    let store = JobStore::open(settings, workspaces)?;
    assert_eq!(
        store.pull_request_guide(&scope, &target)?.unwrap().text.as_deref(),
        Some("Review cancellation first")
    );
    assert!(
        !store
            .start_pull_request_guide(&scope, &target, &data.snapshot.head_sha, &data.snapshot.base_sha, &request)?
            .1
    );
    Ok(())
}
