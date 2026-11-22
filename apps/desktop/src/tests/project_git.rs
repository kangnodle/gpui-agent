use super::*;

#[gpui_kit::test]
fn given_a_long_worktree_path_when_git_review_is_scrolled_then_approval_remains_reachable(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1100.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.workspaces.apply(project_catalog());
            s.workspaces.catalog.projects[0].workspaces[0].path =
                "C:/Users/mingy/github/mingyuchoo/study-series-for-rust/GUI/gpui/gpui-agent/.dev/demo-git-verification-20261031/worktree".into();
            s.workspaces.error = Some("fixture unavailable".into());
            s.workspaces.git.apply(overview(), None);
            s.open_panel(Panel::Git);
            cx.notify();
        });
        window.render_frame(cx);
        let before = window.find("proposal-approve").bounds();
        window.scroll("chat-tool-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1600.))), cx);
        window.render_frame(cx);
        assert!(
            window.find("proposal-approve").bounds().top() < before.top(),
            "Git content must scroll instead of being clipped"
        );
        assert!(window.find("proposal-approve").visible());
    })
    .unwrap();
}

pub(super) fn overview() -> GitOverview {
    serde_json::from_value(serde_json::json!({
        "workspace_id":42,
        "version":{"head":"head","index":"index","worktree":"tree"},
        "checkpoints":[],
        "files":[
            {"path":"first.rs","original_path":null,"index_status":" ","worktree_status":"M"},
            {"path":"second.rs","original_path":null,"index_status":" ","worktree_status":"M"}
        ],
        "diff":"+reviewed change", "truncated":false, "backup":null,"commit":null,"pr":null,
        "proposal":{"message":"Agent proposal","paths":["first.rs","second.rs"],
            "expected":{"head":"head","index":"index","worktree":"tree"}}
    }))
    .unwrap()
}

#[gpui_kit::test]
fn given_active_agent_work_when_a_proposal_diff_is_opened_then_the_operation_is_preserved(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.workspaces.apply(project_catalog());
            s.workspaces.git.apply(overview(), None);
            s.open_panel(Panel::Git);
            s.busy = true;
            s.operation = Some(99);
            s.background_job = Some(8);
            cx.notify();
        });
        window.render_frame(cx);
        assert_ne!(window.find("proposal-diff-0").disabled(), Some(true));
        window.click("proposal-diff-0", cx);
        assert_eq!(view.read(cx).operation, Some(99));
        assert_eq!(view.read(cx).background_job, Some(8));
        assert!(view.read(cx).busy);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_an_agent_commit_proposal_when_reviewed_then_only_explicitly_selected_files_are_submitted(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.client = api.client.for_account(Provider::Claude, "default".into(), Some(42));
            s.workspaces.apply(project_catalog());
            s.workspaces.catalog.projects[0].git = true;
            s.workspaces.git.apply(overview(), None);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("git-proposal-review").visible());
        window.click("git-proposal-review", cx);
        window.render_frame(cx);
        assert!(window.find("chat-view").visible());
        assert!(window.find("proposal-file-0").visible());
        assert!(!api.requests.try_iter().any(|r| r.contains("commit_selected")));
        window.click("proposal-file-0", cx);
        view.update(cx, |s, _| s.workspaces.git.message = "Operator message".into());
        window.render_frame(cx);
        window.click("proposal-approve", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    let requests: Vec<_> = api.requests.try_iter().collect();
    let commit = requests.iter().find(|r| r.contains("commit_selected")).unwrap();
    assert!(commit.starts_with("post /v1/workspaces/42/git "));
    assert!(commit.contains(r#""paths":["second.rs"]"#), "{commit}");
    assert!(commit.contains(r#""message":"Operator message""#));
    let body: serde_json::Value = serde_json::from_str(commit.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body["expected"],
        serde_json::json!({"branch":null,"head":"head","index":"index","worktree":"tree"})
    );
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).workspaces.git.overview.as_ref().unwrap().commit.as_deref(),
            Some("committed-selection")
        );
    });
}

#[gpui_kit::test]
fn given_changed_files_since_proposal_when_reviewed_then_commit_is_disabled(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.workspaces.apply(project_catalog());
            let mut changed = overview();
            changed.version.worktree = "changed-after-proposal".into();
            s.workspaces.git.apply(changed, None);
            s.open_panel(Panel::Git);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find("proposal-approve").disabled(), Some(true));
        assert!(window.find("git-proposal-stale").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_no_selected_proposal_files_when_reviewed_then_commit_is_disabled(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.workspaces.apply(project_catalog());
            s.workspaces.git.apply(overview(), None);
            s.open_panel(Panel::Git);
            cx.notify();
        });
        window.render_frame(cx);
        window.click("proposal-file-0", cx);
        window.click("proposal-file-1", cx);
        window.render_frame(cx);
        assert_eq!(window.find("proposal-approve").disabled(), Some(true));
    })
    .unwrap();
}
