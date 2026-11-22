use anyhow::Result;
use gpui_agent_application::ports::JobRepository;
use gpui_agent_contracts::{ChatRequest,
                           JobScope,
                           Provider};
use gpui_agent_infrastructure::{JobStore,
                                Settings};

#[tokio::test]
async fn given_scoped_queues_when_reopened_then_order_and_original_receipts_are_preserved() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let foreign_path = root.path().join("another workspace");
    std::fs::create_dir(&foreign_path)?;
    let catalog = workspaces.register(foreign_path.display().to_string()).await?;
    let workspace = catalog
        .projects
        .iter()
        .flat_map(|project| &project.workspaces)
        .find(|workspace| workspace.id == workspaces.default_workspace().unwrap())
        .unwrap();
    let other_workspace = catalog
        .projects
        .iter()
        .flat_map(|project| &project.workspaces)
        .find(|candidate| candidate.id != workspace.id)
        .unwrap();
    let scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspace.id),
        workspace_path: workspace.path.clone(),
    };
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    let request = ChatRequest {
        extensions: None,
        skills: vec![],
        computer_use: false,
        text: "original request".into(),
        model: Some("saved-model".into()),
        session_id: None,
        conversation_id: None,
        attachments: vec![],
        claude_permission_mode: None,
        codex_execution_policy: None,
    };
    let first = store.enqueue_once(&scope, &request, "original-command")?;
    let second = store.enqueue(&scope, &request)?;
    let account = store.create_account(Provider::Codex, "Other account")?;
    let other_scopes = [
        JobScope {
            account_id: account.id,
            ..scope.clone()
        },
        JobScope {
            provider: Provider::Claude,
            ..scope.clone()
        },
        JobScope {
            workspace_id: Some(other_workspace.id),
            workspace_path: other_workspace.path.clone(),
            ..scope.clone()
        },
    ];
    for other in &other_scopes {
        store.enqueue(other, &request)?;
    }
    drop(store);

    let store = JobStore::open(settings, workspaces)?;
    let queued = |scope: &JobScope| -> Result<Vec<i64>> {
        Ok(store
            .list()?
            .into_iter()
            .filter(|job| job.scope == *scope && job.state == gpui_agent_contracts::JobState::Queued)
            .map(|job| job.id)
            .collect())
    };
    assert_eq!(queued(&scope)?.len(), 2);
    for other in &other_scopes {
        assert_eq!(queued(other)?.len(), 1);
    }
    let retry = store.enqueue_once(&scope, &request, "original-command")?;
    assert_eq!(retry.id, first.id);
    let saved = store.get(first.id)?;
    assert_eq!(saved.request.text, "original request");
    assert_eq!(saved.request.model.as_deref(), Some("saved-model"));
    let mut different = saved.request.clone();
    different.text = "different request".into();
    assert!(store.enqueue_once(&scope, &different, "original-command").is_err());
    assert_eq!(store.claim(&scope)?.unwrap().job.id, first.id);
    assert!(store.claim(&scope)?.is_none(), "One account runs one job at a time; {} waits", second.id);
    Ok(())
}
