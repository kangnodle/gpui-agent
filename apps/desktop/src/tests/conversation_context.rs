use super::*;

#[gpui_kit::test]
fn given_quick_check_when_a_plugin_is_checked_and_unchecked_then_only_this_conversation_changes(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        // Given installed packages and an empty conversation selection.
        view.update(cx, |s, _| ready(s));
        window.render_frame(cx);
        assert!(window.try_find("shared-guidance-toggle").is_none());
        assert!(window.find("conversation-plugins-toggle").visible());
        assert!(window.try_find("conversation-plugin-repo:review").is_none());
        // When selecting a package from the shared picker.
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        window.click("conversation-plugin-check-repo:review", cx);
        window.render_frame(cx);
        assert_eq!(window.find("conversation-plugin-repo:review").label(), Some("Review tools"));
        // Then removing its header checkbox leaves the installed package
        // enabled.
        window.click("conversation-plugins-close", cx);
        window.render_frame(cx);
        window.click("conversation-plugin-repo:review", cx);
        window.render_frame(cx);
        assert!(window.try_find("conversation-plugin-repo:review").is_none());
        assert!(view.read(cx).plugins.catalog.plugins[0].enabled);
        assert!(view.read(cx).panel.is_none());
    })
    .unwrap();
}

fn ready(s: &mut ChatApp) {
    s.plugins.scope = s.client.guidance_editor_scope();
    s.plugins.loaded = true;
    s.plugins.catalog = serde_json::from_value(serde_json::json!({"plugins":[
        {"id":"repo:review","manifest":{"name":"review","display_name":"Review tools","description":"Review project changes"},"enabled":true,"skills":[]},
        {"id":"user:browser","manifest":{"name":"browser","description":"Browse pages"},"enabled":true,"skills":[]},
        {"id":"user:disabled","manifest":{"name":"disabled","display_name":"Disabled tools","description":"Unavailable"},"enabled":false,"skills":[]}
    ],"diagnostics":[]}))
    .unwrap();
    s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot::default());
}

#[gpui_kit::test]
fn given_quick_check_when_a_new_chat_draft_is_reopened_then_all_choices_are_restored(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            let conversation = &mut s.conversations[0];
            conversation.input = "Saved draft".into();
            conversation.guidance.extensions.plugins = vec!["repo:review".into()];
            conversation.guidance.extensions.mcp = vec!["local-tools".into()];
            conversation.guidance.selected = vec![gpui_agent_contracts::SkillReference {
                id: "repo:.agents/skills/check/SKILL.md".into(),
            }];
            conversation.save_draft();
            conversation.new_chat();
        });
        window.render_frame(cx);
        window.click("chat-draft-0-0", cx);
        let conversation = &view.read(cx).conversations[0];
        assert_eq!(conversation.input, "Saved draft");
        assert_eq!(conversation.guidance.extensions.plugins, ["repo:review"]);
        assert_eq!(conversation.guidance.extensions.mcp, ["local-tools"]);
        assert_eq!(conversation.guidance.selected[0].id, "repo:.agents/skills/check/SKILL.md");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_header_skills_when_selecting_standalone_and_plugin_skills_then_the_header_shows_both(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.conversations[0].guidance.snapshot.as_mut().unwrap().skills = vec![
                SkillMetadata::parse("repo:.agents/skills/verify/SKILL.md", "verify", "---\ndescription: Verify\n---\nTests", None)
                    .unwrap()
                    .0,
                SkillMetadata::parse("plugin:repo:review/skills/check/SKILL.md", "check", "---\ndescription: Check\n---\nTests", None)
                    .unwrap()
                    .0,
            ];
        });
        window.render_frame(cx);
        assert!(window.try_find("shared-guidance-toggle").is_none());
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("conversation-skill-check-0").visible());
        assert!(window.find("conversation-skill-check-1").visible());
        window.click("conversation-skill-check-0", cx);
        assert_eq!(view.read(cx).conversations[0].guidance.selected.len(), 1);

        assert!(window.find("conversation-skill-check-1").visible());
        window.click("conversation-skill-check-1", cx);
        assert_eq!(view.read(cx).conversations[0].guidance.selected.len(), 2);
        window.render_frame(cx);
        window.click("conversation-plugins-close", cx);
        window.render_frame(cx);
        assert_eq!(window.find("conversation-skill-0").label(), Some("verify"));
        assert_eq!(window.find("conversation-skill-1").label(), Some("check"));
        assert!(window.within("composer-surface").try_find("conversation-skill-0").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_quick_check_when_mcp_is_selected_filtered_and_removed_then_connection_settings_are_unchanged(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.plugins.conversation_mcp_scope = format!("{}:{:?}:default", s.client.guidance_editor_scope(), PROVIDERS[0]);
            s.plugins.conversation_mcp = serde_json::from_value(serde_json::json!([{"name":"local-tools","url":"https://example.invalid/mcp"}])).unwrap();
            s.plugins.conversation_mcp_loaded = true;
        });
        window.render_frame(cx);
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        window.click("conversation-filter-mcp", cx);
        window.render_frame(cx);
        assert!(window.try_find("conversation-plugin-check-repo:review").is_none());
        window.click("conversation-mcp-check-local-tools", cx);
        window.click("conversation-plugins-close", cx);
        window.render_frame(cx);
        assert_eq!(window.find("conversation-mcp-local-tools").label(), Some("local-tools"));
        assert_eq!(window.find("conversation-mcp-local-tools-chip").label(), Some("MCP · local-tools"));
        window.click("conversation-mcp-local-tools", cx);
        window.render_frame(cx);
        assert!(window.try_find("conversation-mcp-local-tools").is_none());
        assert!(view.read(cx).conversations[0].guidance.extensions.mcp.is_empty());
        assert_eq!(view.read(cx).plugins.conversation_mcp[0].name(), "local-tools");
        assert!(view.read(cx).conversations[1].guidance.extensions.mcp.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_conversation_context_when_rendered_then_real_enabled_plugins_and_setup_share_the_header(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(900.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.conversations[0].guidance.extensions.plugins = vec!["repo:review".into(), "user:browser".into()];
        });
        window.render_frame(cx);
        let header = window.find("chat-header").bounds();
        let context = window.find("conversation-context").bounds();
        assert!(context.top() >= header.top() && context.bottom() <= header.bottom());
        assert!((context.center().x - header.center().x).abs() <= px(2.));
        assert_eq!(window.find("guidance-status").label(), Some("Set instructions"));
        assert_eq!(window.find("conversation-plugin-repo:review").label(), Some("Review tools"));
        assert_eq!(window.find("conversation-plugin-user:browser").label(), Some("browser"));
        assert!(window.try_find("conversation-plugin-user:disabled").is_none());
        assert!(window.within("composer-surface").try_find("guidance-status").is_none());
        assert!(window.within("composer-surface").try_find("conversation-plugin-repo:review").is_none());
        window.click("guidance-status", cx);
        assert_eq!(view.read(cx).panel, Some(Panel::Guidance));
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_conversation_context_when_inspected_during_a_turn_then_details_preserve_the_chat(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.conversations[0].input = "Keep this draft".into();
            s.conversations[0].guidance.snapshot.as_mut().unwrap().instructions.push(InstructionDocument {
                source: "repo:AGENTS.md".into(),
                content: "Project rules".into(),
                truncated: false,
            });
            s.busy = true;
        });
        window.render_frame(cx);
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("conversation-plugins-picker").visible());
        window.click("conversation-plugin-check-repo:review", cx);
        assert!(view.read(cx).conversations[0].guidance.extensions.plugins.is_empty());
        assert!(view.read(cx).panel.is_none());
        assert_eq!(view.read(cx).conversations[0].input, "Keep this draft");
        window.click("conversation-plugins-close", cx);
        window.render_frame(cx);
        window.click("guidance-status", cx);
        window.render_frame(cx);
        assert!(window.find("conversation-instructions").visible());
        assert_eq!(window.find("conversation-instruction-0").label(), Some("repo:AGENTS.md"));
        assert!(window.find("conversation-instruction-content-0").visible());
        window.click("conversation-guidance-manage", cx);
        assert!(view.read(cx).panel.is_none(), "editing must not interrupt a running turn");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_conversation_context_when_catalog_is_empty_failed_or_changes_scope_then_unavailable_names_are_hidden(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.plugins.catalog.plugins.clear();
        });
        window.render_frame(cx);
        assert!(window.find("conversation-plugins-toggle").visible());
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        view.update(cx, |s, _| {
            ready(s);
            s.plugins.error = Some("Cannot load catalog".into());
        });
        window.render_frame(cx);
        assert!(window.find("conversation-plugins-retry").visible());
        assert!(window.try_find("conversation-plugin-repo:review").is_none());
        view.update(cx, |s, _| {
            s.plugins.scope = "old-workspace".into();
        });
        window.render_frame(cx);
        assert!(window.find("conversation-plugins-toggle").visible());
        assert!(window.try_find("conversation-plugin-repo:review").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_conversation_context_when_details_are_long_in_a_short_window_then_close_and_manage_remain_reachable(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1000.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.sidebar_open = false;
            s.plugins.catalog.plugins[0].manifest.description = "Long description line\n".repeat(150);
            s.conversations[0].guidance.snapshot.as_mut().unwrap().instructions.push(InstructionDocument {
                source: "repo:AGENTS.md".into(),
                content: "Long instruction line\n\n".repeat(150),
                truncated: false,
            });
        });
        window.render_frame(cx);
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        for id in ["conversation-plugins-close", "conversation-plugins-manage"] {
            let bounds = window.find(id).bounds();
            assert!(
                window.find(id).visible() && bounds.top() >= px(0.) && bounds.bottom() <= px(360.),
                "{id}: {bounds:?}"
            );
        }
        window.click("conversation-plugins-close", cx);
        window.render_frame(cx);
        window.click("guidance-status", cx);
        window.render_frame(cx);
        for id in ["conversation-guidance-close", "conversation-guidance-manage"] {
            let bounds = window.find(id).bounds();
            assert!(
                window.find(id).visible() && bounds.top() >= px(0.) && bounds.bottom() <= px(360.),
                "{id}: {bounds:?}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_conversation_context_when_refresh_is_requested_then_the_catalog_is_revalidated(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| ready(s));
        window.render_frame(cx);
        window.click("refresh-chat", cx);
        window.render_frame(cx);
        assert!(window.try_find("conversation-plugin-repo:review").is_none());
        assert!(window.find("conversation-plugins-toggle").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_conversation_context_when_names_are_long_and_the_window_is_narrow_then_actions_fit(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, _, cx| {
        view.update(cx, |s, _| {
            ready(s);
            s.sidebar_open = false;
            s.conversations[0].guidance.extensions.plugins = vec!["repo:review".into(), "user:browser".into()];
            s.plugins.catalog.plugins[0].manifest.display_name = Some("Long plugin name ".repeat(20));
            for number in 0 .. 6 {
                let mut plugin = s.plugins.catalog.plugins[0].clone();
                plugin.id = format!("repo:extra-{number}");
                s.plugins.catalog.plugins.push(plugin);
                s.conversations[0].guidance.extensions.plugins.push(format!("repo:extra-{number}"));
            }
        });
    })
    .unwrap();
    for width in [1100., 700., 500.] {
        cx.simulate_window_resize(handle.into(), size(px(width), px(900.)));
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let context = window.find("conversation-context").bounds();
            assert!(context.left() >= px(0.) && context.right() <= px(width));
            for id in [
                "guidance-status",
                "conversation-plugin-repo:review",
                "conversation-plugin-user:browser",
                "conversation-plugin-repo:extra-5",
            ] {
                let bounds = window.find(id).bounds();
                assert!(
                    bounds.left() >= context.left() && bounds.right() <= context.right(),
                    "{id} at {width}: {bounds:?}, context={context:?}"
                );
            }
            assert!(window.find("composer-surface").bounds().top() >= context.bottom());
            let refresh = window.find("refresh-chat").bounds();
            assert!(
                refresh.bottom() <= context.top() || refresh.left() >= context.right(),
                "width={width}, refresh={refresh:?}, context={context:?}"
            );
        })
        .unwrap();
    }
}
