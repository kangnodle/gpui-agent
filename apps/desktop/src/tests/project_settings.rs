use super::*;
use gpui_kit::test::TestAppContextExt as _;

#[gpui_kit::test]
fn given_another_selected_project_when_a_project_menu_creates_a_worktree_then_the_request_targets_its_owner(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1200.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.client = api.client.clone();
            s.client.select_workspace(Some(42));
            let mut catalog = project_catalog();
            catalog.projects[1].git = true;
            catalog.projects.swap(0, 1);
            s.workspaces.apply(catalog);
            s.workspaces.path = "Keep project draft".into();
            s.conversations[0].input = "Keep chat draft".into();
            s.open_panel(Panel::Workspace);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("workspace-section-tabs").is_none());
        window.hover("project-actions-2", cx);
        window.click("project-menu-2", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).workspaces.project, Some(2));
        assert!(window.find("worktree-create").visible());
        view.update(cx, |s, _| {
            s.workspaces.branch = "feature/second".into();
            s.workspaces.base = "main".into();
        });
        window.render_frame(cx);
        window.click("worktree-create", cx);
        let s = view.read(cx);
        assert_eq!(s.workspaces.path, "Keep project draft");
        assert_eq!(s.conversations[0].input, "Keep chat draft");
        assert_eq!(s.workspaces.selected, Some(42));
    })
    .unwrap();
    let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(request.starts_with("post /v1/projects/2/worktrees "), "{request}");
    let body: serde_json::Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"branch":"feature/second","base":"main"}));
}

#[gpui_kit::test]
async fn given_managed_and_locked_worktrees_when_a_project_menu_removes_one_then_only_explicit_confirmation_deletes_it(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1200.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.client = api.client.clone();
            let mut catalog = project_catalog();
            let project = &mut catalog.projects[0];
            project.git = true;
            for (id, locked) in [(44, false), (45, true)] {
                project.workspaces.push(Workspace {
                    id,
                    project_id: 1,
                    path: format!("/worktree-{id}"),
                    branch: Some(format!("feature-{id}")),
                    main: false,
                    managed: true,
                    locked,
                    missing: false,
                });
            }
            s.workspaces.apply(catalog);
            s.open_panel(Panel::Workspace);
            cx.notify();
        });
        window.render_frame(cx);
        window.hover("project-actions-1", cx);
        window.click("project-menu-1", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(2usize, cx);
        assert!(view.read(cx).workspaces.remove.is_none(), "Locked worktrees cannot be removed");
        window.press("escape", cx);
    })
    .unwrap();
    cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("project-menu-1", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).workspaces.remove.as_ref().unwrap().id, 44);
        assert!(api.requests.try_recv().is_err(), "Opening confirmation must not delete");
        window.click("remove-worktree-cancel", cx);
        assert!(view.read(cx).workspaces.remove.is_none());
        assert!(api.requests.try_recv().is_err(), "Cancelling must not delete");
    })
    .unwrap();
    cx.wait_for(handle.into(), std::time::Duration::from_secs(2), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(handle.into(), |_, window, cx| {
        window.hover("project-actions-1", cx);
        window.click("project-menu-1", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
        window.render_frame(cx);
        window.click("remove-worktree-confirm", cx);
    })
    .unwrap();
    let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(request.starts_with("delete /v1/workspaces/44 "), "{request}");
}

#[gpui_kit::test]
fn given_a_worktree_keyword_when_project_settings_are_searched_then_project_menus_and_the_pending_form_remain_available(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1200.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            let mut catalog = project_catalog();
            catalog.projects[0].git = true;
            s.workspaces.apply(catalog);
            s.open_panel(Panel::Workspace);
            s.workspaces.project = Some(1);
            s.settings_search = "브랜치".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("project-menu-1").visible());
        assert!(window.find("worktree-create").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_reviewed_git_file_when_attached_then_context_uses_its_workspace_and_preserves_the_chat_draft(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    let existing = Attachment::from_bytes("keep.txt".into(), b"Keep attachment").unwrap();
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(1200.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.client = api.client.clone();
            s.client.select_workspace(Some(42));
            s.workspaces.apply(project_catalog());
            let mut overview = project_git::overview();
            overview.proposal = None;
            s.workspaces.git.apply(overview, None);
            s.workspaces.diff = Some(FileDiff {
                workspace_id: 42,
                path: "src/main 한글.rs".into(),
                unstaged: String::new(),
                staged: String::new(),
                untracked: true,
                truncated: false,
            });
            s.conversations[0].input = "Keep draft".into();
            s.conversations[0].attachments = vec![existing.clone()];
            s.open_panel(Panel::Git);
            cx.notify();
        });
        window.render_frame(cx);
        window.scroll("chat-tool-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.find("diff-attach").visible());
        window.click("diff-attach", cx);
    })
    .unwrap();
    let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(
        request.starts_with("post /v1/providers/claude/context ") && request.contains("x-knd-workspace: 42"),
        "{request}"
    );
    let body: serde_json::Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"path":"src/main 한글.rs","start_line":null,"end_line":null}));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let done = cx.update(|cx| {
            view.update(cx, |s, _| {
                s.updates(&s.wake.clone());
                !s.busy
            })
        });
        if done {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Context attachment timed out");
        std::thread::yield_now();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            assert_eq!(s.conversations[0].input, "Keep draft");
            assert_eq!(s.conversations[0].attachments.len(), 2);
            assert_eq!(s.conversations[0].attachments[0], existing);
            assert!(matches!(&s.conversations[0].attachments[1], Attachment::Text { source: Some(source), .. }
                if source.path == "src/main 한글.rs"));
            s.workspaces.diff.as_mut().unwrap().workspace_id = 43;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("diff-attach").is_none(), "A different workspace's diff must not be attached");
    })
    .unwrap();
}
