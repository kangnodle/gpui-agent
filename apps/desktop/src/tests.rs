use crate::{app::{ChatApp,
                  HistoryDialog},
            client::ApiClient,
            language::Language,
            ui::{Field,
                 Panel,
                 Wake}};
use gpui::{AppContext,
           Bounds,
           Entity,
           TestAppContext,
           WindowBounds,
           WindowHandle,
           WindowOptions,
           px,
           size,
           test::TestWindowExt};
use gpui_agent_contracts::*;
use gpui_agent_presentation::Pending;

mod conversation_context;
mod project_git;
mod project_settings;

#[gpui_kit::test]
fn given_a_project_when_git_is_opened_then_review_stays_beside_chat_and_out_of_settings(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1100.), px(900.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.workspaces.apply(project_catalog());
            s.workspaces.catalog.projects[0].git = true;
            s.open_panel(Panel::Git);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(view.read(cx).panel.is_none(), "Git belongs to the project, not settings");
        assert_eq!(view.read(cx).chat_tool, Some(Panel::Git));
        assert!(window.find("chat-view").visible());
        assert!(window.find("project-git-scope").visible());
        assert!(window.try_find("settings-sidebar").is_none());
        assert!(!Panel::SETTINGS_GROUPS.iter().any(|(_, panels)| panels.contains(&Panel::Git)));
        window.click("chat-tool-close", cx);
        window.click("chat-git", cx);
        assert_eq!(view.read(cx).chat_tool, Some(Panel::Git));
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_long_skill_path_when_the_list_is_narrow_then_the_label_fits(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Skills);
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.guidance_editor.user = true;
            let id = format!("user:skills/{}/SKILL.md", "long-skill-name".repeat(12));
            s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot {
                skills: vec![
                    SkillMetadata::parse(&id, "verify", "---\ndescription: Verify changes\n---\nTests", None)
                        .unwrap()
                        .0,
                ],
                ..Default::default()
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("skill-label-0").bounds().right() <= px(500.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_narrow_plugin_settings_when_rendered_then_scope_and_actions_do_not_overlap(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Plugins);
            cx.notify();
        });
        window.render_frame(cx);
        let scope = window.within("plugin-scope-tabs").find(1usize).bounds();
        let import = window.find("plugin-import").bounds();
        assert!(scope.bottom() <= import.top() || scope.right() <= import.left());
        assert!(scope.size.width >= px(100.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_long_project_path_when_project_settings_open_then_controls_fit_the_content(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1100.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Workspace);
            s.workspaces.catalog = project_catalog();
            s.workspaces.catalog.projects[0].workspaces[0].path = format!("C:/{}", "long-project-path/".repeat(12));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("settings-content").bounds().right() <= px(1100.));
        assert!(window.find("project-select-42").bounds().right() <= window.find("settings-content").bounds().right());
        assert!(window.find("projects-refresh").bounds().right() <= window.find("settings-content").bounds().right());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_general_and_appearance_on_a_narrow_window_when_opened_then_choices_fit(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(360.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        for (panel, id) in [(Panel::General, "settings-language-group"), (Panel::Appearance, "settings-theme-group")] {
            view.update(cx, |s, cx| {
                s.panel = Some(panel);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find(id).bounds().right() <= window.find("settings-content").bounds().right());
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_guidance_at_any_width_when_opened_then_both_scopes_are_stacked_in_application_order(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    for width in [1200., 500.] {
        cx.simulate_window_resize(handle.into(), size(px(width), px(900.)));
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, cx| {
                s.panel = Some(Panel::Guidance);
                cx.notify();
            });
            window.render_frame(cx);
            let (app, project) = (window.find("guidance-scope-user"), window.find("guidance-scope-project"));
            assert!(app.visible() && project.visible());
            assert!(app.bounds().bottom() <= project.bounds().top(), "app-wide instructions apply first");
            assert!(project.bounds().bottom() <= window.find("guidance-reload").bounds().top());
            assert!(project.bounds().right() <= window.find("settings-content").bounds().right());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn given_project_draft_when_reopening_settings_and_resizing_then_it_is_preserved(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1200.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Workspace);
            s.workspaces.path = "draft path".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("workspace-section-tabs").is_none());
        view.update(cx, |s, _| {
            s.open_panel(Panel::General);
            s.open_panel(Panel::Workspace);
        });
        assert_eq!(view.read(cx).workspaces.path, "draft path");
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("project-register").bounds().right() <= px(500.));
        assert_eq!(view.read(cx).workspaces.path, "draft path");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_project_git_when_resized_then_checkpoint_navigation_stays_above_content(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(900.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.open_panel(Panel::Git);
            s.language = Language::Korean;
            s.workspaces.git.label = "checkpoint draft".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("git-section-changes").bounds().top(),
            window.find("git-section-checkpoints").bounds().top()
        );
        assert_eq!(window.find("git-section-checkpoints").label(), Some("작업 상태 저장 · 복원"));
        assert!(window.try_find("checkpoint-save").is_none());
        window.click("git-section-checkpoints", cx);
        assert!(window.find("git-section-tabs").bounds().bottom() <= window.find("checkpoint-save").bounds().top());
        window.click("git-section-changes", cx);
        window.click("git-section-checkpoints", cx);
        assert_eq!(view.read(cx).workspaces.git.label, "checkpoint draft");
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("git-section-tabs").bounds().bottom() <= window.find("checkpoint-save").bounds().top());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_plugin_settings_when_resized_then_category_tabs_move_above_the_content(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1200.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Plugins);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("plugin-tabs").bounds().right() <= window.find("plugin-scope-tabs").bounds().left());
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("plugin-tabs").bounds().bottom() <= window.find("plugin-scope-tabs").bounds().top());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_the_plugin_button_when_keyboard_focused_then_support_is_shown(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot::default()));
        window.render_frame(cx);
        for _ in 0 .. 100 {
            window.focus_next(cx);
            window.render_frame(cx);
            if window.find("conversation-plugins-toggle").focused() == Some(true) {
                break;
            }
        }
        assert_eq!(window.find("conversation-plugins-toggle").focused(), Some(true));
        window.click("conversation-plugins-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("conversation-plugins-picker").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_settings_when_opened_then_plugins_group_skills_and_mcp_connections(cx: &mut TestAppContext) {
    let (handle, _) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-toggle", cx);
        assert!(window.within("popup-menu").try_find_by_label("Instructions").is_some());
        assert!(window.within("popup-menu").try_find_by_label("Plugins").is_some());
        assert!(window.within("popup-menu").try_find_by_label("Skills").is_none());
        assert!(window.within("popup-menu").try_find_by_label("MCP servers").is_none());
    })
    .unwrap();
}

fn mcp_oauth_fixture(s: &mut ChatApp, accounts: Vec<AccountProfile>) {
    s.panel = Some(Panel::Mcp);
    s.mcp.servers = serde_json::from_value(serde_json::json!([{"name":"remote","url":"https://example.invalid/mcp","oauth":{}}])).unwrap();
    s.mcp.accounts = accounts;
    s.mcp.oauth.insert(
        "remote".into(),
        McpOAuthStatus {
            state: McpOAuthState::Connected,
            authorization_url: None,
            expires_at: None,
            message: None,
        },
    );
}

#[gpui_kit::test]
fn given_mcp_oauth_accounts_when_selected_again_then_the_oauth_status_is_preserved(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            mcp_oauth_fixture(
                s,
                vec![
                    AccountProfile {
                        id: "default".into(),
                        provider: Provider::Claude,
                        label: "Default".into(),
                    },
                    AccountProfile {
                        id: "other".into(),
                        provider: Provider::Claude,
                        label: "Other".into(),
                    },
                ],
            )
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("mcp-account-default", cx);
        assert!(view.read(cx).mcp.oauth.contains_key("remote"));
        window.click("mcp-account-other", cx);
        assert_eq!(view.read(cx).mcp.account, "other");
        assert!(view.read(cx).mcp.oauth.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_one_mcp_oauth_account_when_rendered_then_it_is_shown_without_a_selection_button(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            mcp_oauth_fixture(
                s,
                vec![AccountProfile {
                    id: "default".into(),
                    provider: Provider::Claude,
                    label: "Default".into(),
                }],
            )
        });
        window.render_frame(cx);
        assert!(window.try_find("mcp-account-default").is_none());
        assert!(window.try_find("mcp-account-current").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_common_mcp_registration_when_opened_then_no_provider_is_required_and_drafts_are_preserved(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Mcp);
            s.mcp.provider = Provider::Claude;
            s.mcp.account = "other".into();
        });
        window.render_frame(cx);
        assert!(window.try_find("mcp-provider-group").is_none());
        assert!(window.try_find("mcp-account-current").is_none());
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.find("mcp-load").visible());
        assert!(window.find("mcp-save").visible());
        assert_eq!(view.read(cx).mcp.account, "other");
        assert_eq!(view.read(cx).mcp.provider, Provider::Claude);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_plugin_tabs_when_switching_category_and_scope_then_the_selection_follows(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Plugins);
            s.plugins.loaded = true;
            s.plugins.scope = s.client.guidance_editor_scope();
        });
        window.render_frame(cx);
        assert_eq!(window.find("plugin-category-0").label(), Some("Installed"));
        assert_eq!(window.within("plugin-scope-tabs").find(0usize).label(), Some("App-wide"));
        window.within("plugin-scope-tabs").click(0usize, cx);
        assert!(view.read(cx).plugins.user);
        window.within("plugin-scope-tabs").click(1usize, cx);
        assert!(!view.read(cx).plugins.user);
        if window.try_find("plugin-category-1").is_some() {
            window.click("plugin-category-1", cx);
        } else {
            window.within("plugin-tabs").click(1usize, cx);
        }
        assert_eq!(view.read(cx).panel, Some(Panel::Skills));
        assert_eq!(window.within("skill-scope-tabs").find(0usize).label(), Some("App-wide · 0"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_mixed_skill_scopes_when_switching_tabs_then_lists_counts_and_add_targets_follow_the_scope(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        // Given standalone, packaged and catalog-only skills in both scopes.
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Skills);
            s.workspaces.catalog = project_catalog();
            s.workspaces.selected = Some(42);
            s.guidance_editor.scope = s.guidance_editor_scope();
            let ids = [
                "user:skills/common/SKILL.md",
                "repo:.agents/skills/local/SKILL.md",
                "plugin:user:tools/skills/common/SKILL.md",
                "plugin:repo:tools/skills/local/SKILL.md",
            ];
            s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot {
                skills: ids
                    .iter()
                    .map(|id| {
                        SkillMetadata::parse(id, "verify", "---\ndescription: Verify changes\n---\nTests", None)
                            .unwrap()
                            .0
                    })
                    .collect(),
                ..Default::default()
            });
            s.guidance_editor.catalog.files = ids[.. 2]
                .iter()
                .map(|id| (*id).into())
                .chain([
                    "user:skills/broken/SKILL.md".into(),
                    "repo:.agents/skills/broken/SKILL.md".into(),
                    "repo:AGENTS.md".into(),
                ])
                .collect();
        });
        window.render_frame(cx);
        // When project scope is selected, Then only its skills and files
        // appear.
        assert!(window.try_find("skill-details-0").is_none());
        assert!(window.try_find("skill-details-1").is_some());
        assert!(window.try_find("skill-details-2").is_none());
        assert!(window.try_find("skill-details-3").is_some());
        assert!(window.try_find("guidance-file-2").is_none());
        assert!(window.try_find("guidance-file-3").is_some());
        assert_eq!(window.within("skill-scope-tabs").find(0usize).label(), Some("App-wide · 3"));
        assert_eq!(window.within("skill-scope-tabs").find(1usize).label(), Some("Current project · 3"));
        assert_eq!(window.find("skill-new").label(), Some("Add skill to this project"));
        assert_eq!(window.find("skill-scope-description").label(), Some("Available only in this project."));
        // When switching to app scope, Then the list and creation destination
        // change.
        window.within("skill-scope-tabs").click(0usize, cx);
        assert!(window.try_find("skill-details-0").is_some());
        assert!(window.try_find("skill-details-1").is_none());
        assert!(window.try_find("skill-details-2").is_some());
        assert!(window.try_find("skill-details-3").is_none());
        assert!(window.try_find("guidance-file-2").is_some());
        assert!(window.try_find("guidance-file-3").is_none());
        assert_eq!(window.find("skill-new").label(), Some("Add app-wide skill"));
        assert_eq!(window.find("skill-folder-import").label(), Some("Import folder app-wide…"));
        assert_eq!(window.find("skill-scope-description").label(), Some("Available in all projects."));
        view.update(cx, |s, _| s.guidance_editor.search = "no-match".into());
        window.render_frame(cx);
        assert_eq!(window.within("skill-scope-tabs").find(0usize).label(), Some("App-wide · 3"));
        assert_eq!(window.find("skill-library-empty").label(), Some("No search results."));
        window.within("skill-scope-tabs").click(1usize, cx);
        assert!(view.read(cx).guidance_editor.search.is_empty());
        assert!(window.try_find("skill-details-1").is_some());
        assert!(view.read(cx).conversations[0].guidance.selected.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_no_project_or_dirty_skill_edits_when_browsing_scopes_then_empty_states_and_drafts_are_safe(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Skills);
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot::default());
        });
        window.render_frame(cx);
        assert_eq!(window.within("skill-scope-tabs").find(1usize).label(), Some("Current project · 0"));
        assert_eq!(window.find("skill-library-empty").label(), Some("No skills registered in this project."));
        assert_eq!(window.find("skill-scope-description").label(), Some("Open a project to manage its skills."));
        assert_eq!(window.find("skill-new").disabled(), Some(true));
        assert_eq!(window.find("skill-folder-import").disabled(), Some(true));
        window.within("skill-scope-tabs").click(0usize, cx);
        assert_eq!(window.find("skill-library-empty").label(), Some("No app-wide skills registered."));
        assert_ne!(window.find("skill-new").disabled(), Some(true));
        view.update(cx, |s, _| {
            s.guidance_editor.load(GuidanceFile {
                id: "user:skills/common/SKILL.md".into(),
                text: "Original".into(),
                revision: Some("v1".into()),
            });
            s.guidance_editor.text = "Draft".into();
        });
        window.render_frame(cx);
        assert_eq!(window.within("skill-scope-tabs").find(1usize).disabled(), Some(true));
        window.within("skill-scope-tabs").click(1usize, cx);
        assert_eq!(view.read(cx).guidance_editor.text, "Draft");
        assert!(view.read(cx).guidance_editor.user);
        view.update(cx, |s, _| s.guidance_editor.text = "Original".into());
        window.render_frame(cx);
        window.within("skill-scope-tabs").click(1usize, cx);
        assert!(view.read(cx).guidance_editor.file.is_none());
        assert!(window.try_find("guidance-text").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_an_empty_skill_catalog_when_opened_then_the_composer_reports_no_available_skills(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot::default());
            s.conversations[0].guidance.open = true;
        });
        window.render_frame(cx);
        assert_eq!(window.find("shared-guidance-message").label(), Some("No skills available."));
        view.update(cx, |s, _| {
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.panel = Some(Panel::Skills);
        });
        window.render_frame(cx);
        assert!(window.find("skill-library-empty").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_plugin_and_standalone_skills_when_opened_then_each_group_selects_its_own_skills(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let skill = |id: &str| {
        SkillMetadata::parse(id, "verify", "---\ndescription: Verify changes\n---\nTests", None)
            .unwrap()
            .0
    };
    cx.update_window(handle.into(), |_, window, cx| {
        // Given interleaved skills and equally named packages in different
        // scopes.
        view.update(cx, |s, _| {
            s.plugins.scope = s.client.guidance_editor_scope();
            s.plugins.loaded = true;
            s.plugins.catalog = serde_json::from_value(serde_json::json!({"plugins":[
                {"id":"repo:review","manifest":{"name":"review","display_name":"Review tools","description":"Review"},"enabled":true,"skills":[]},
                {"id":"user:review","manifest":{"name":"review","display_name":"Review tools","description":"Review"},"enabled":true,"skills":[]}
            ],"diagnostics":[]}))
            .unwrap();
            s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot {
                skills: vec![
                    skill("plugin:repo:review/skills/verify/SKILL.md"),
                    skill("repo:.agents/skills/verify/SKILL.md"),
                    skill("plugin:user:review/skills/verify/SKILL.md"),
                    skill("plugin:repo:review/skills/check/SKILL.md"),
                ],
                ..Default::default()
            });
            s.conversations[0].guidance.open = true;
        });
        window.render_frame(cx);
        // When opening the picker, Then groups retain every skill and its
        // identity.
        assert_eq!(window.find("conversation-plugins-toggle").label(), Some("Connect plugins"));
        assert_eq!(window.find("shared-skill-group-repo:review").label(), Some("Review tools · repo:review"));
        assert_eq!(window.find("shared-skill-group-user:review").label(), Some("Review tools · user:review"));
        assert_eq!(window.find("shared-skill-group-").label(), Some("Standalone skills"));
        assert!(window.within("shared-skill-group-repo:review").find("shared-skill-0").visible());
        assert!(window.within("shared-skill-group-repo:review").try_find("shared-skill-3").is_some());
        assert!(window.within("shared-skill-group-user:review").try_find("shared-skill-2").is_some());
        assert!(window.within("shared-skill-group-").try_find("shared-skill-1").is_some());
        window.within("shared-skill-group-repo:review").click("shared-skill-0", cx);
        assert_eq!(
            view.read(cx).conversations[0].guidance.selected[0].id,
            "plugin:repo:review/skills/verify/SKILL.md"
        );
        window.within("shared-skill-group-repo:review").click("shared-skill-0", cx);
        assert!(view.read(cx).conversations[0].guidance.selected.is_empty());
        window.scroll("shared-guidance-list", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.within("shared-skill-group-user:review").find("shared-skill-2").visible());
        window.within("shared-skill-group-user:review").click("shared-skill-2", cx);
        assert_eq!(
            view.read(cx).conversations[0].guidance.selected[0].id,
            "plugin:user:review/skills/verify/SKILL.md"
        );
        view.update(cx, |s, _| s.conversations[0].input = "Use $missing".into());
        window.render_frame(cx);
        assert!(window.try_find("shared-skill-group-repo:review").is_none());
        assert!(window.try_find("shared-skill-group-").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_local_plugin_when_imported_then_catalog_activation_and_confirmed_removal_preserve_drafts(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("gpui-plugin-desktop-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let repo = root.join("repo");
    let source = root.join("source");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(source.join("skills/review")).unwrap();
    std::fs::write(
        source.join("plugin.json"),
        r#"{"name":"review-tools","display_name":"Review tools","description":"Review workflows"}"#,
    )
    .unwrap();
    std::fs::write(
        source.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review public behavior\n---\nCheck behavior",
    )
    .unwrap();
    let settings = gpui_agent_infrastructure::Settings::discover(repo, Some(root.join("home"))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let token = "plugindesktopfixture1234567890123456";
    let api = gpui_agent_api::Api::new(settings.application().unwrap(), token.into()).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = runtime.spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.client = ApiClient::new(url, token.into()).unwrap();
            s.conversations[0].input = "Keep chat draft".into();
            s.mcp.json = "[{unfinished".into();
            s.open_panel(Panel::Plugins);
        });
        window.render_frame(cx);
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| s.plugins.loaded);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("plugins-empty").visible());
        assert!(window.try_find("settings-nav-Skills").is_none());
        assert!(window.try_find("settings-nav-Mcp").is_none());
        window.within("plugin-scope-tabs").click(0usize, cx);
        window.click("plugin-import", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![source.clone()])
    });
    cx.run_until_parked();
    wait_for_plugins(&view, cx, |s| s.plugins.catalog.plugins.len() == 1 && s.plugins.pending.is_none());
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(view.read(cx).plugins.catalog.plugins[0].id, "user:review-tools");
        assert!(!view.read(cx).plugins.catalog.plugins[0].enabled);
        view.update(cx, |s, _| s.plugins.search = "missing".into());
        window.render_frame(cx);
        assert!(window.find("plugins-empty").visible());
        view.update(cx, |s, _| s.plugins.search = "REVIEW".into());
        window.render_frame(cx);
        window.click("plugin-card-0", cx);
        window.click("plugin-enabled", cx);
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| s.plugins.catalog.plugins[0].enabled && !s.busy);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(
            view.read(cx).conversations[0]
                .guidance
                .snapshot
                .as_ref()
                .unwrap()
                .skills
                .iter()
                .any(|skill| skill.name == "review")
        );
        window.render_frame(cx);
        if window.try_find("plugin-category-1").is_some() {
            window.click("plugin-category-1", cx);
        } else {
            window.within("plugin-tabs").click(1usize, cx);
        }
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.within("skill-scope-tabs").click(0usize, cx);
        assert_eq!(window.find("skill-details-0").disabled(), Some(true));
        if window.try_find("plugin-category-0").is_some() {
            window.click("plugin-category-0", cx);
        } else {
            window.within("plugin-tabs").click(0usize, cx);
        }
        window.click("plugin-enabled", cx);
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| !s.plugins.catalog.plugins[0].enabled && !s.busy);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("plugin-remove", cx);
        window.click("plugin-remove-cancel", cx);
        assert_eq!(view.read(cx).plugins.catalog.plugins.len(), 1);
        window.click("plugin-remove", cx);
        window.click("plugin-remove-confirm", cx);
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| s.plugins.catalog.plugins.is_empty() && !s.busy);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("plugins-empty").visible());
        if window.try_find("plugin-category-2").is_some() {
            window.click("plugin-category-2", cx);
        } else {
            window.within("plugin-tabs").click(2usize, cx);
        }
        assert_eq!(view.read(cx).mcp.json, "[{unfinished");
        if window.try_find("plugin-category-0").is_some() {
            window.click("plugin-category-0", cx);
        } else {
            window.within("plugin-tabs").click(0usize, cx);
        }
        assert_eq!(view.read(cx).conversations[0].input, "Keep chat draft");
        assert!(source.join("plugin.json").is_file());
    })
    .unwrap();
    let _ = stop.send(());
    runtime.block_on(host).unwrap().unwrap();
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();
}

fn wait_for_plugins(view: &Entity<ChatApp>, cx: &mut TestAppContext, ready: impl Fn(&ChatApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let done = cx.update(|cx| {
            view.update(cx, |s, _| {
                s.updates(&s.wake.clone());
                s.poll_plugins();
                assert!(s.plugins.error.is_none(), "{:?}", s.plugins.error);
                ready(s)
            })
        });
        if done {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "plugin request timed out");
        std::thread::yield_now();
    }
}

#[gpui_kit::test]
fn given_a_corrupted_enabled_plugin_when_its_diagnostic_is_disabled_then_runtime_recovers(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("gpui-plugin-recovery-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let repo = root.join("repo");
    let home = root.join("home");
    std::fs::create_dir_all(&repo).unwrap();
    let settings = gpui_agent_infrastructure::Settings::discover(repo.clone(), Some(home.clone())).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let token = "pluginrecoverydesktopfixture12345678";
    let client = ApiClient::new(format!("http://{}", listener.local_addr().unwrap()), token.into()).unwrap();
    let api = gpui_agent_api::Api::new(settings.application().unwrap(), token.into()).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = runtime.spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let import = PluginImportRequest {
        scope: "user".into(),
        files: std::collections::BTreeMap::from([(
            "plugin.json".into(),
            br#"{"name":"broken","description":"Recovery fixture","mcp_servers":[{"name":"fixture","url":"https://example.invalid/mcp"}]}"#.to_vec(),
        )]),
    };
    client
        .response(reqwest::Method::POST, "/v1/plugins/import", Some(serde_json::to_value(import).unwrap()))
        .unwrap();
    client
        .response(
            reqwest::Method::PUT,
            "/v1/plugins/enabled",
            Some(serde_json::json!({"id":"user:broken","enabled":true})),
        )
        .unwrap();
    std::fs::write(home.join("plugins/broken/plugin.json"), "invalid JSON").unwrap();
    let plugins = gpui_agent_infrastructure::PluginFiles::new(&home, &repo).unwrap();
    assert!(plugins.mcp_servers().is_err());
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, _, cx| {
        view.update(cx, |s, _| {
            s.client = client;
            s.open_panel(Panel::Plugins);
        });
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| s.plugins.loaded);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            let mut invalid = s.plugins.catalog.diagnostics[0].clone();
            invalid.source = "../../invalid".into();
            s.plugins.catalog.diagnostics.push(invalid);
        });
        window.render_frame(cx);
        assert!(window.try_find("plugin-card-0").is_none());
        assert!(window.try_find("plugin-diagnostic-disable-1").is_none());
        window.click("plugin-diagnostic-disable-0", cx);
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| s.plugins.pending.is_none() && !s.busy);
    assert!(plugins.mcp_servers().unwrap().is_empty());
    let _ = stop.send(());
    runtime.block_on(host).unwrap().unwrap();
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui_kit::test]
fn given_a_skill_installed_by_the_agent_when_its_turn_ends_then_it_is_enabled_and_selected_for_the_conversation(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("gpui-plugin-auto-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let repo = root.join("repo");
    let home = root.join("home");
    std::fs::create_dir_all(&repo).unwrap();
    let install = home.join("claude/plugins/cache/market/review/1.0.0");
    std::fs::create_dir_all(install.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(install.join("skills/check")).unwrap();
    std::fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"review","description":"Review tools"}"#).unwrap();
    std::fs::write(install.join("skills/check/SKILL.md"), "---\ndescription: Check\n---\nCheck it").unwrap();
    std::fs::write(
        home.join("claude/plugins/installed_plugins.json"),
        serde_json::to_vec(&serde_json::json!({"plugins":{"review@market":[{"installPath":install}]}})).unwrap(),
    )
    .unwrap();
    let settings = gpui_agent_infrastructure::Settings::discover(repo.clone(), Some(home.clone())).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let token = "pluginautosyncdesktopfixture123456789";
    let client = ApiClient::new(format!("http://{}", listener.local_addr().unwrap()), token.into()).unwrap();
    let api = gpui_agent_api::Api::new(settings.application().unwrap(), token.into()).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = runtime.spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, _, cx| {
        view.update(cx, |s, _| {
            s.client = client;
            s.auto_sync_claude_plugins();
        });
    })
    .unwrap();
    wait_for_plugins(&view, cx, |s| s.plugins.loaded);
    cx.update_window(handle.into(), |_, _, cx| {
        let s = view.read(cx);
        assert!(s.plugins.catalog.plugins[0].enabled);
        assert_eq!(s.conversations[s.selected].guidance.extensions.plugins, ["user:review"]);
    })
    .unwrap();
    let _ = stop.send(());
    runtime.block_on(host).unwrap().unwrap();
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui_kit::test]
fn given_an_open_plugin_picker_when_workspace_changes_then_late_selection_and_catalog_are_discarded(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.plugins.scope = s.client.guidance_editor_scope();
            s.plugins.loaded = true;
            s.panel = Some(Panel::Plugins);
        });
        window.render_frame(cx);
        window.click("plugin-import", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.update(|cx| {
        view.update(cx, |s, _| {
            let (tx, rx) = std::sync::mpsc::channel();
            s.plugins.pending = Some(rx);
            tx.send(Err("Old workspace response".into())).unwrap();
            s.client.select_workspace(Some(99));
            s.panel = None;
            s.poll_plugins();
            assert!(s.plugins.error.is_none());
            assert!(s.plugins.pending.is_none());
        })
    });
    cx.simulate_path_prompt_response(|_| Some(vec!["/must-not-be-read".into()]));
    cx.run_until_parked();
    cx.update(|cx| {
        let s = view.read(cx);
        assert!(s.plugins.error.is_none());
        assert!(s.plugins.pending.is_none());
        assert!(!s.plugins.loaded);
    });
}

#[gpui_kit::test]
fn given_guidance_scope_cards_when_a_file_is_created_then_scope_changes_and_dirty_drafts_stay_safe(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Guidance);
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.guidance_editor.user = false;
            s.guidance_editor.catalog.files = vec!["repo:AGENTS.md".into()];
        });
        window.render_frame(cx);
        assert_eq!(window.find("guidance-scope-user").label(), Some("App-wide · Checking"));
        assert_eq!(window.find("guidance-status-project").label(), Some("Checking"));
        window.click("guidance-new-agents-user", cx);
        assert!(view.read(cx).guidance_editor.user);
        view.update(cx, |s, _| s.busy = false);
        window.render_frame(cx);
        window.click("guidance-file-0", cx);
        assert!(!view.read(cx).guidance_editor.user);
        view.update(cx, |s, _| s.busy = false);
        view.update(cx, |s, _| {
            s.guidance_editor.load(GuidanceFile {
                id: "repo:AGENTS.md".into(),
                text: "Original".into(),
                revision: Some("v1".into()),
            });
            s.guidance_editor.text = "Draft".into();
        });
        window.render_frame(cx);
        assert_eq!(window.find("guidance-new-agents-user").disabled(), Some(true));
        window.click("guidance-new-agents-user", cx);
        assert!(!view.read(cx).guidance_editor.user);
        assert!(window.find("guidance-editing").visible());
        assert_eq!(view.read(cx).guidance_editor.text, "Draft");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_guidance_draft_when_preview_is_toggled_then_editing_and_draft_are_preserved(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Guidance);
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.guidance_editor.load(GuidanceFile {
                id: "repo:AGENTS.md".into(),
                text: "Original".into(),
                revision: Some("rev".into()),
            });
            s.guidance_editor.text = "# Draft".into();
        });
        window.render_frame(cx);
        assert!(window.try_find("guidance-preview-content").is_none());
        window.click("guidance-preview-toggle", cx);
        window.render_frame(cx);
        assert!(window.try_find("guidance-preview-content").is_some());
        assert!(window.find("guidance-text").visible());
        assert_eq!(view.read(cx).guidance_editor.text, "# Draft");
        window.click("guidance-preview-toggle", cx);
        window.render_frame(cx);
        assert!(window.try_find("guidance-preview-content").is_none());
        assert_eq!(view.read(cx).guidance_editor.text, "# Draft");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_guidance_and_skills_when_composing_or_editing_then_each_surface_keeps_its_role(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let skill = SkillMetadata::parse(
        "repo:.agents/skills/verify/SKILL.md",
        "verify",
        "---\ndescription: Verify changes\n---\nTests",
        None,
    )
    .unwrap()
    .0;
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].guidance.snapshot = Some(GuidanceSnapshot {
                instructions: vec![InstructionDocument {
                    source: "repo:AGENTS.md".into(),
                    content: "Project rules".into(),
                    truncated: false,
                }],
                skills: vec![skill],
                ..Default::default()
            });
            s.conversations[0].guidance.open = true;
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.guidance_editor.catalog.files = vec!["repo:AGENTS.md".into(), "repo:.agents/skills/verify/SKILL.md".into()];
        });
        window.render_frame(cx);
        assert!(window.find("guidance-status").visible());
        assert_eq!(window.find("guidance-status").label(), Some("Instructions · 1"));
        assert_eq!(window.find("conversation-plugins-toggle").label(), Some("Connect plugins"));
        assert_eq!(
            window.find("shared-guidance-message").label(),
            Some("No skills selected. Select skills to apply to this turn.")
        );
        for id in ["shared-guidance-manage", "shared-guidance-refresh", "shared-skill-enabled-0"] {
            assert!(window.try_find(id).is_none());
        }
        view.update(cx, |s, _| {
            s.conversations[0].guidance.open = false;
            s.conversations[0].input = "Review $ver".into();
        });
        window.render_frame(cx);
        assert!(window.find("shared-skill-0").visible());
        window.click("shared-skill-0", cx);
        assert_eq!(view.read(cx).conversations[0].input, "Review ");
        view.update(cx, |s, _| s.conversations[0].input = "Again $ver".into());
        window.render_frame(cx);
        window.click("shared-skill-0", cx);
        assert_eq!(view.read(cx).conversations[0].guidance.selected.len(), 1);
        assert_eq!(view.read(cx).conversations[0].input, "Again ");
        view.update(cx, |s, _| s.conversations[0].guidance.open = true);

        assert_eq!(view.read(cx).conversations[0].guidance.selected.len(), 1);
        window.render_frame(cx);
        assert_eq!(
            window.find("shared-guidance-message").label(),
            Some("Selected skills also apply to subsequent turns in this conversation.")
        );
        assert!(window.find("conversation-skill-0").visible());
        window.click("conversation-skill-0", cx);
        assert!(view.read(cx).conversations[0].guidance.selected.is_empty());
        view.update(cx, |s, _| s.panel = Some(Panel::Guidance));
        window.render_frame(cx);
        assert!(window.find("guidance-new-agents-user").visible());
        assert!(window.try_find("guidance-new-agents-project").is_none());
        assert!(window.find("guidance-file-0").visible());
        assert_eq!(window.find("guidance-file-0").label(), Some("AGENTS.md"));
        assert_eq!(window.find("guidance-status-user").label(), Some("No file"));
        assert_eq!(window.find("guidance-status-project").label(), Some("Applied"));
        assert!(window.try_find("guidance-new-skill").is_none());
        assert!(window.try_find("guidance-file-1").is_none());
        view.update(cx, |s, _| s.panel = Some(Panel::Skills));
        window.render_frame(cx);
        assert_eq!(window.find("skill-description-0").label(), Some("Verify changes"));
        assert!(window.find("skill-details-0").visible());
        assert!(window.find("skill-folder-import").visible());
        view.update(cx, |s, _| s.guidance_editor.search = "no-match".into());
        window.render_frame(cx);
        assert!(window.try_find("skill-details-0").is_none());
        view.update(cx, |s, _| s.guidance_editor.search = "VERIFY".into());
        window.render_frame(cx);
        assert!(window.find("skill-details-0").visible());
        view.update(cx, |s, _| s.guidance_editor.search.clear());
        window.render_frame(cx);
        assert!(window.find("skill-new").visible());
        assert!(window.try_find("guidance-name").is_none());
        assert!(window.try_find("guidance-file-1").is_none());
        assert!(window.find("shared-skill-enabled-0").visible());
        assert!(window.try_find("guidance-new-agents-user").is_none());
        assert!(window.try_find("guidance-file-0").is_none());
        view.update(cx, |s, _| {
            s.guidance_editor.load(GuidanceFile {
                id: "repo:.agents/skills/verify/SKILL.md".into(),
                text: "Original".into(),
                revision: Some("v1".into()),
            });
            s.guidance_editor.text = "Unsaved".into();
            s.open_panel(Panel::Guidance);
            assert_eq!(s.panel, Some(Panel::Skills));
            assert_eq!(s.guidance_editor.text, "Unsaved");
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_an_agent_skill_proposal_when_project_skills_are_open_then_the_operator_reviews_and_decides_it(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.guidance_editor.scope = s.guidance_editor_scope();
            s.guidance_editor.proposals = vec![gpui_agent_contracts::SkillProposal {
                id: "sp1".into(),
                skill_id: "repo:.agents/skills/review/SKILL.md".into(),
                name: "review".into(),
                content: "---\ndescription: Review a change\n---\nRun public tests".into(),
                resources: [("references/checks.md".to_owned(), "Checks".to_owned())].into(),
                requested_during: Some("Please review my change".into()),
                status: gpui_agent_contracts::SkillProposalStatus::Pending,
                message: None,
                expires_at: 1,
            }];
            s.panel = Some(Panel::Skills);
        });
        window.render_frame(cx);
        assert_eq!(window.find("skill-proposal-0").label(), Some("Skill proposal · review"));
        assert!(window.find("skill-proposal-files-0").visible());
        assert!(window.try_find("skill-proposal-request-0").is_some());
        assert!(window.find("skill-proposal-approve-0").visible());
        assert!(window.find("skill-proposal-reject-0").visible());
        view.update(cx, |s, _| s.busy = true);
        window.render_frame(cx);
        assert_eq!(window.find("skill-proposal-approve-0").disabled(), Some(true));
        assert_eq!(window.find("skill-proposal-reject-0").disabled(), Some(true));
        view.update(cx, |s, _| {
            s.busy = false;
            s.guidance_editor.mcp_proposals = vec![gpui_agent_contracts::McpProposal {
                id: "mp1".into(),
                plugin_id: "repo:mcp-fixture-tools".into(),
                server: gpui_agent_contracts::McpServer::Stdio {
                    name: "fixture_tools".into(),
                    command: "/usr/bin/tool".into(),
                    args: vec!["--serve".into()],
                },
                rationale: "Inspect fixtures".into(),
                environment: vec![],
                requested_during: Some("Please inspect the fixtures".into()),
                status: gpui_agent_contracts::ProposalStatus::Pending,
                message: None,
                expires_at: 1,
            }];
        });
        window.render_frame(cx);
        assert_eq!(window.find("mcp-proposal-0").label(), Some("MCP server proposal · fixture_tools"));
        assert!(window.try_find("mcp-proposal-facts-0").is_some());
        assert!(window.try_find("mcp-proposal-approve-0").is_some());
        assert!(window.try_find("mcp-proposal-reject-0").is_some());
        view.update(cx, |s, _| s.busy = true);
        window.render_frame(cx);
        assert_eq!(window.find("mcp-proposal-approve-0").disabled(), Some(true));
        view.update(cx, |s, _| s.busy = false);
        // Proposals install into the project, so they are not offered for
        // app-wide skills.
        view.update(cx, |s, _| {
            s.busy = false;
            s.guidance_editor.user = true;
        });
        window.render_frame(cx);
        assert!(window.try_find("skill-proposal-0").is_none());
        assert!(window.try_find("mcp-proposal-0").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_guidance_settings_when_a_manifest_is_imported_and_saved_then_the_composer_can_select_it(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("gpui-guidance-editor-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let upload = root.join("uploaded.md");
    let manifest = "---\nname: uploaded\ndescription: Verify public behavior\n---\nRun public tests 한글";
    std::fs::write(&upload, manifest).unwrap();
    let settings = gpui_agent_infrastructure::Settings::discover(repo, Some(root.join("home"))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let token = "guidanceeditordesktopfixture1234567890";
    let api = gpui_agent_api::Api::new(settings.application().unwrap(), token.into()).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let host = runtime.spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = ApiClient::new(url, token.into()).unwrap();
            s.conversations[0].input = "Keep my chat draft".into();
            s.conversations[0].guidance.open = true;
        });
        window.render_frame(cx);
        window.click("settings-toggle", cx);
        window.within("popup-menu").click(5usize, cx);
        if window.try_find("plugin-category-1").is_some() {
            window.click("plugin-category-1", cx);
        } else {
            window.within("plugin-tabs").click(1usize, cx);
        }
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.within("skill-scope-tabs").click(0usize, cx);
        view.update(cx, |s, _| {
            s.guidance_editor.name = "uploaded".into();
            s.guidance_editor.description = "Verify uploaded files".into();
        });
        window.render_frame(cx);
        window.click("skill-new", cx);
        window.render_frame(cx);
        window.click("guidance-new-skill", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let manifest = view.read(cx).guidance_editor.text.clone();
        let metadata = SkillMetadata::parse("user:skills/uploaded/SKILL.md", "uploaded", &manifest, None).unwrap().0;
        assert_eq!(metadata.name, "uploaded");
        assert_eq!(metadata.description, "Verify uploaded files");
        view.update(cx, |s, _| s.guidance_editor.text = "invalid frontmatter".into());
        window.render_frame(cx);
        assert_eq!(window.find("guidance-save").disabled(), Some(true));
        view.update(cx, |s, _| s.guidance_editor.text = manifest);
        window.render_frame(cx);
        assert!(window.find("guidance-save").visible());
        window.click("guidance-import", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.multiple);
        Some(vec![upload.clone()])
    });
    cx.run_until_parked();
    wait_for_chat_state(&view, cx, |s| !s.busy && s.guidance_editor.text == manifest);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("guidance-save", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        let state = view.read(cx);
        assert!(state.workspaces.error.is_none(), "{:?}", state.workspaces.error);
        assert!(!state.guidance_editor.dirty());
        assert!(state.guidance_editor.file.as_ref().unwrap().revision.is_some());
        window.click("panel-close", cx);
        window.render_frame(cx);
        window.click("shared-skill-0", cx);
        assert_eq!(view.read(cx).conversations[0].guidance.selected[0].id, "user:skills/uploaded/SKILL.md");
        assert_eq!(view.read(cx).conversations[0].input, "Keep my chat draft");
    })
    .unwrap();
    let _ = stop.send(());
    runtime.block_on(host).unwrap().unwrap();
    drop(runtime);
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui_kit::test]
fn given_an_edited_question_when_only_its_skill_selection_changes_then_the_composer_can_submit(cx: &mut TestAppContext) {
    let (_, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |state, _| {
            state.busy = false;
            state.selected = 0;
            let conversation = &mut state.conversations[0];
            conversation.id = Some(7);
            conversation.status = Some(AuthStatus {
                authenticated: true,
                account: None,
                method: None,
            });
            conversation.messages = serde_json::from_value(serde_json::json!([
                {"id":1,"user":true,"text":"Same question","incomplete":false},
                {"id":2,"user":false,"text":"Completed answer","incomplete":false}
            ]))
            .unwrap();
            conversation.editing = Some("Same question".into());
            conversation.guidance.selected = vec![SkillReference {
                id: "repo:.agents/skills/verify/SKILL.md".into(),
            }];
            assert!(state.can_send());
            state.conversations[0].guidance.selected.clear();
            assert!(!state.can_send());
            state.conversations[0].guidance.extensions.plugins.push("repo:review".into());
            assert!(state.can_send(), "changing only the package selection is an editable revision");
            state.conversations[0].guidance.extensions.plugins.clear();
            state.conversations[0].guidance.extensions.mcp.push("local-tools".into());
            assert!(state.can_send(), "changing only the MCP selection is an editable revision");
            state.conversations[0].guidance.extensions.mcp.clear();
            assert!(!state.can_send());
        });
    });
}

fn fixture(cx: &mut TestAppContext) -> (WindowHandle<gpui::base::Root>, Entity<ChatApp>) {
    cx.update(|cx| {
        gpui::init(cx);
        crate::ui::apply_theme(cx);
    });
    cx.update(|cx| {
        let (window, view) = gpui::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(Default::default(), size(px(1100.), px(780.))))),
                ..crate::titlebar::TitlebarMode::default().window_options()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let (tx, _rx) = async_channel::unbounded();
                    let mut view = ChatApp::new(
                        ApiClient::new("http://127.0.0.1:1".into(), "fixture".into()).unwrap(),
                        "/fixture".into(),
                        None,
                        &Wake(tx),
                        cx.focus_handle(),
                    );
                    // Isolate tests from the failed startup refresh.
                    (view.tx, view.rx) = std::sync::mpsc::sync_channel(512);
                    view.busy = false;
                    view.language = Language::English;
                    view.conversations[0].status = Some(AuthStatus {
                        authenticated: true,
                        account: None,
                        method: None,
                    });
                    view.focus.focus(window, cx);
                    view
                })
            },
        )
        .unwrap();
        (window.downcast::<gpui::base::Root>().unwrap(), view)
    })
}

fn wait_for_computer(view: &Entity<ChatApp>, cx: &mut TestAppContext, ready: impl Fn(&ChatApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if cx.update(|cx| {
            view.update(cx, |s, _| {
                s.poll();
                ready(s)
            })
        }) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "computer UI did not reach its expected state: {:?}",
            cx.update(|cx| {
                let s = view.read(cx);
                (
                    s.selected,
                    s.background_job,
                    s.conversations[s.selected].pending.as_ref().map(|p| p.request_id),
                    s.computer.status.as_ref().map(|s| s.job_id),
                    s.computer.pending.is_some(),
                    s.job_monitor.jobs.len(),
                    s.conversations[s.selected].error.clone(),
                )
            })
        );
        std::thread::yield_now();
    }
}

#[gpui_kit::test]
fn given_automatic_access_before_first_capture_when_native_confirmation_remains_then_accept_is_available(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = pending_computer_task(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.computer.status = Some(ComputerStatus {
                available: true,
                automatic: true,
                ..Default::default()
            });
            s.computer.pending = None;
        });
        window.render_frame(cx);
        assert!(window.find("mcp-submit").visible());
        assert_ne!(window.find("mcp-submit").disabled(), Some(true));
        view.update(cx, |s, _| s.computer.status.as_mut().unwrap().job_id = Some(999));
        window.render_frame(cx);
        assert_eq!(window.find("mcp-submit").disabled(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn workspace_file_picker_selects_only_a_remote_path_and_preserves_inputs_until_explicit_attachment(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let existing = Attachment::from_bytes("keep.txt".into(), b"Keep existing attachment").unwrap();
    let (handle, view) = fixture(cx);
    {
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| {
                (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
                s.client = api.client.clone();
                s.client.select_workspace(Some(42));
                s.conversations[0].input = "Main draft".into();
                s.conversations[0].attachments = vec![existing.clone()];
                s.attachment_ui.workspace = true;
                s.attachment_ui.path = "previous.txt".into();
                s.attachment_ui.start = "2".into();
                s.attachment_ui.end = "2".into();
                s.open_panel(Panel::Attach);
            });
            window.render_frame(cx);
            // Given an existing draft, attachment and range, open the host's
            // file list.
            window.click("attach-browse", cx);
        })
        .unwrap();
        wait_for_context_entry(handle, "context-entry-0", cx);
        assert!(
            !cx.did_prompt_for_paths(),
            "Workspace browsing must use the host API, never the local OS dialog"
        );
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("context-cancel", cx);
            let s = view.read(cx);
            assert_eq!(s.attachment_ui.path, "previous.txt");
            assert_eq!(s.attachment_ui.start, "2");
            assert_eq!(s.attachment_ui.end, "2");
            assert_eq!(s.conversations[0].input, "Main draft");
            assert_eq!(s.conversations[0].attachments, vec![existing.clone()]);
            window.click("attach-browse", cx);
        })
        .unwrap();
        wait_for_context_entry(handle, "context-entry-0", cx);
        cx.update_window(handle.into(), |_, window, cx| window.click("context-entry-0", cx)).unwrap();
        wait_for_context_entry(handle, "context-entry-0", cx);
        cx.update_window(handle.into(), |_, window, cx| {
            // When a remote file is chosen, only its relative path changes.
            window.click("context-entry-1", cx);
            let s = view.read(cx);
            assert_eq!(s.attachment_ui.path, "src/main 한글.rs");
            assert_eq!(s.attachment_ui.start, "2");
            assert_eq!(s.attachment_ui.end, "2");
            assert_eq!(s.conversations[0].attachments, vec![existing.clone()]);
            assert_eq!(s.chat_tool, Some(Panel::Attach));
            let requests: Vec<_> = api.requests.try_iter().collect();
            assert!(!requests.iter().any(|request| request.starts_with("post ") && request.contains("/context ")));
            assert!(requests.iter().filter(|r| r.contains("/files?")).all(|r| r.contains("x-knd-workspace: 42")));
            window.click("attach-add", cx);
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        cx.update(|cx| {
            let s = view.read(cx);
            let attachments = &s.conversations[0].attachments;
            assert_eq!(attachments.len(), 2);
            assert!(matches!(&attachments[1], Attachment::Text { text, source: Some(source), .. }
                if text == "second\n" && source.path == "src/main 한글.rs" && source.start_line == Some(2) && source.end_line == Some(2)));
            assert_eq!(s.conversations[0].input, "Main draft");
        });
        let requests: Vec<_> = api
            .requests
            .try_iter()
            .filter(|request| request.starts_with("post ") && request.contains("/context "))
            .collect();
        assert_eq!(requests.len(), 1, "Only explicit Add snapshots file context");
        assert!(requests[0].contains("\"start_line\":2") && requests[0].contains("\"end_line\":2"));
    }
}

fn wait_for_context_entry(handle: WindowHandle<gpui::base::Root>, id: &str, cx: &mut TestAppContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        let ready = cx
            .update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.try_find(id.to_owned()).is_some()
            })
            .unwrap();
        if ready {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "Workspace file list did not finish loading");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[gpui_kit::test]
fn workspace_file_picker_can_retry_navigate_empty_folders_and_cancel_in_flight_without_late_changes(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering;
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    let existing = Attachment::from_bytes("keep.txt".into(), b"Keep").unwrap();
    api.history_mode.store(5, Ordering::Relaxed);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.attachment_ui.workspace = true;
            s.attachment_ui.path = "keep.rs".into();
            s.attachment_ui.start = "3".into();
            s.attachment_ui.end = "9".into();
            s.conversations[0].input = "Unsent draft".into();
            s.conversations[0].attachments = vec![existing.clone()];
            s.open_panel(Panel::Attach);
        });
        window.render_frame(cx);
        window.click("attach-browse", cx);
    })
    .unwrap();
    // Given a failed remote listing, retry without replacing any attachment
    // input.
    wait_for_context_entry(handle, "context-retry", cx);
    api.history_mode.store(0, Ordering::Relaxed);
    cx.update_window(handle.into(), |_, window, cx| window.click("context-retry", cx)).unwrap();
    wait_for_context_entry(handle, "context-entry-0", cx);
    cx.update_window(handle.into(), |_, window, cx| window.click("context-entry-0", cx)).unwrap();
    wait_for_context_entry(handle, "context-entry-0", cx);
    cx.update_window(handle.into(), |_, window, cx| window.click("context-entry-0", cx)).unwrap();
    wait_for_context_entry(handle, "context-empty", cx);
    cx.update_window(handle.into(), |_, window, cx| window.click("context-parent", cx)).unwrap();
    wait_for_context_entry(handle, "context-entry-0", cx);
    cx.update_window(handle.into(), |_, window, cx| window.click("context-root", cx)).unwrap();
    wait_for_context_entry(handle, "context-entry-0", cx);
    cx.update_window(handle.into(), |_, window, cx| window.click("context-cancel", cx)).unwrap();
    for close_tool in [false, true] {
        // When cancelled or closed during loading, then a late host reply
        // cannot reopen the picker or replace the draft.
        api.requests.try_iter().for_each(drop);
        api.history_mode.store(6, Ordering::Relaxed);
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| s.open_panel(Panel::Attach));
            window.render_frame(cx);
            window.click("attach-browse", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let pending = api.requests.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert!(pending.contains("/files?"), "The host must have received the in-flight listing");
        cx.update_window(handle.into(), |_, window, cx| {
            window.click(if close_tool { "chat-tool-close" } else { "context-cancel" }, cx);
        })
        .unwrap();
        api.history_mode.store(0, Ordering::Relaxed);
        // A second response fences completion of the held request on this
        // fixture server.
        api.client.workspace_files(Provider::Claude, "").unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let s = view.read(cx);
            assert!(s.attachment_ui.picker.is_none());
            assert_eq!(s.chat_tool, if close_tool { None } else { Some(Panel::Attach) });
            assert_eq!(s.attachment_ui.path, "keep.rs");
            assert_eq!(s.attachment_ui.start, "3");
            assert_eq!(s.attachment_ui.end, "9");
            assert_eq!(s.conversations[0].input, "Unsent draft");
            assert_eq!(s.conversations[0].attachments, vec![existing.clone()]);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn local_file_picker_groups_actions_below_the_path_in_chat(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    for width in [1100., 640.] {
        cx.simulate_window_resize(handle.into(), size(px(width), px(780.)));
        cx.update_window(handle.into(), |_, window, cx| {
            {
                // Given the chat entry point, when local file attachment is
                // opened, then both actions share a row below
                // the path and align with the form.
                view.update(cx, |s, _| {
                    s.attachment_ui.workspace = false;
                    s.open_panel(Panel::Attach);
                });
                window.render_frame(cx);
                let path_id = match &view.read(cx).inputs["attachment-path"] {
                    | Field::Line {
                        state, ..
                    } => state.entity_id(),
                    | _ => panic!("attachment path must be a line input"),
                };
                let path = window.find(("input", path_id)).bounds();
                let browse = window.find("attach-browse").bounds();
                let add = window.find("attach-add").bounds();
                assert!(browse.origin.y >= path.bottom(), "File selection belongs below the path: {browse:?}, {path:?}");
                assert_eq!(browse.origin.y, add.origin.y);
                assert_eq!(browse.origin.x, window.find("attach-local").bounds().origin.x);
                assert!(add.origin.x >= browse.right());
                assert!(path.right() > add.right());
                assert!(window.find("attach-browse").visible() && window.find("attach-add").visible());
                // Both tabs keep the same lower action row.
                window.click("attach-context", cx);
                window.render_frame(cx);
                assert!(window.find("attach-browse").visible());
                assert_eq!(window.find("attach-browse").bounds().origin.y, window.find("attach-add").bounds().origin.y);
                assert!(window.find("attach-add").visible());
                window.click("chat-tool-close", cx);
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn local_file_picker_preserves_manual_paths_and_external_file_drops(cx: &mut TestAppContext) {
    use gpui::{ExternalPaths,
               FileDropEvent,
               InputEvent,
               point};

    let path = std::env::temp_dir().join(format!("gpui-path-{}.txt", gpui_agent_infrastructure::new_token().unwrap()));
    std::fs::write(&path, "Manual and dropped file").unwrap();
    let (handle, view) = fixture(cx);
    // Given the existing path input, when its Add button is pressed, then the
    // file is attached.
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.conversations[0].input = "Keep draft".into();
            s.attachment_ui.path = path.to_string_lossy().into_owned();
            s.open_panel(Panel::Attach);
        });
        window.render_frame(cx);
        window.click("attach-add", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(view.read(cx).conversations[0].attachments.len(), 1);
        view.update(cx, |s, _| s.open_panel(Panel::Attach));
        window.render_frame(cx);
        // When an external file is dropped onto the attachment view, then it
        // uses the same draft.
        let position = point(px(700.), px(400.));
        window.dispatch_event(
            FileDropEvent::Entered {
                position,
                paths: ExternalPaths([path.clone()].into_iter().collect()),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            FileDropEvent::Submit {
                position,
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let c = &view.read(cx).conversations[0];
        assert_eq!(c.attachments.len(), 2);
        assert_eq!(c.input, "Keep draft");
        assert!(c.error.is_none());
    });
    std::fs::remove_file(path).unwrap();
}

#[gpui_kit::test]
fn local_file_picker_cancels_without_changes_and_attaches_multiple_files(cx: &mut TestAppContext) {
    let folder = std::env::temp_dir().join(format!("gpui-attach-{}", gpui_agent_infrastructure::new_token().unwrap()));
    std::fs::create_dir(&folder).unwrap();
    let first = folder.join("first 한글.txt");
    let second = folder.join("second.md");
    std::fs::write(&first, "First file 한글").unwrap();
    std::fs::write(&second, "Second file").unwrap();
    let existing = Attachment::from_bytes("existing.txt".into(), b"Existing attachment").unwrap();
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        // Given a draft, an attachment and a manually entered path.
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.conversations[0].input = "Draft".into();
            s.conversations[0].attachments = vec![existing.clone()];
            s.attachment_ui.path = "Keep this typed path".into();
            s.open_panel(Panel::Attach);
        });
        window.render_frame(cx);
        window.click("attach-browse", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |s, _| {
            assert!(!s.busy, "The native dialog must not block processing UI updates");
            s.conversations[0].input = "Updated draft".into();
        })
    });
    // When the native file dialog is cancelled, then all draft state stays
    // intact.
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        None
    });
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let s = view.read(cx);
        assert_eq!(s.conversations[0].input, "Updated draft");
        assert_eq!(s.conversations[0].attachments, vec![existing.clone()]);
        assert_eq!(s.attachment_ui.path, "Keep this typed path");
        assert_eq!(s.chat_tool, Some(Panel::Attach));
        assert!(!s.busy);
        window.render_frame(cx);
        window.click("attach-browse", cx);
    })
    .unwrap();
    // When multiple files are selected, then they append to the same draft
    // atomically.
    cx.simulate_path_prompt_response(|_| Some(vec![first.clone(), second.clone()]));
    cx.run_until_parked();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let s = view.read(cx);
        let attachments = &s.conversations[0].attachments;
        assert_eq!(attachments.len(), 3);
        assert_eq!(attachments[0], existing);
        assert_eq!(
            attachments[1],
            Attachment::from_bytes("first 한글.txt".into(), "First file 한글".as_bytes()).unwrap()
        );
        assert_eq!(attachments[2].name(), "second.md");
        assert_eq!(s.conversations[0].input, "Updated draft");
        assert!(s.chat_tool.is_none() && s.panel.is_none());
    });
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
    std::fs::remove_dir(folder).unwrap();
}

#[gpui_kit::test]
fn local_file_picker_reuses_validation_and_never_partially_appends_rejected_selections(cx: &mut TestAppContext) {
    let folder = std::env::temp_dir().join(format!("gpui-attach-validation-{}", gpui_agent_infrastructure::new_token().unwrap()));
    std::fs::create_dir(&folder).unwrap();
    let text = folder.join("valid.txt");
    let pdf = folder.join("invalid.pdf");
    let large_text = folder.join("large.txt");
    let first_image = folder.join("first.png");
    let second_image = folder.join("second.png");
    std::fs::write(&text, "Valid text").unwrap();
    std::fs::write(&pdf, b"%PDF-invalid").unwrap();
    std::fs::write(&large_text, vec![b'a'; 256 * 1024 + 1]).unwrap();
    for path in [&first_image, &second_image] {
        image::RgbaImage::new(1, 1).save(path).unwrap();
        std::fs::OpenOptions::new().write(true).open(path).unwrap().set_len(2 * 1024 * 1024).unwrap();
    }
    // Given existing attachments, reject the whole batch for format, file size,
    // count or combined size.
    for (paths, existing_count, expected) in [
        (vec![text.clone(), pdf.clone()], 1, "PDF attachments are not supported"),
        (vec![large_text.clone()], 1, "Text file exceeds 256 KiB"),
        (vec![text.clone(); 9], 1, "At most 8 attachments"),
        (vec![text.clone(); 2], 7, "At most 8 attachments"),
        (vec![first_image.clone(), second_image.clone()], 1, "4 MiB in total"),
    ] {
        let (handle, view) = fixture(cx);
        let existing: Vec<_> = (0 .. existing_count)
            .map(|index| Attachment::from_bytes(format!("existing-{index}.txt"), b"Keep").unwrap())
            .collect();
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| {
                (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
                s.conversations[0].input = "Keep draft".into();
                s.conversations[0].attachments = existing.clone();
                s.open_panel(Panel::Attach);
            });
            window.render_frame(cx);
            window.click("attach-browse", cx);
        })
        .unwrap();
        cx.simulate_path_prompt_response(|_| Some(paths));
        cx.run_until_parked();
        wait_for_chat(&view, cx);
        cx.update(|cx| {
            let c = &view.read(cx).conversations[0];
            assert!(c.error.as_deref().unwrap_or_default().contains(expected), "{:?}", c.error);
            assert_eq!(c.attachments, existing);
            assert_eq!(c.input, "Keep draft");
        });
    }
    for path in [text, pdf, large_text, first_image, second_image] {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(folder).unwrap();
}

#[gpui_kit::test]
fn project_picker_keeps_updates_running_and_only_opens_a_selected_folder(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.open_panel(Panel::Workspace);
            s.workspaces.path = "/previous".into();
        });
        window.render_frame(cx);
        window.click("project-browse", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |s, cx| {
            assert!(!s.busy);
            s.conversations[0].input = "Updated while choosing a folder".into();
            cx.notify();
        });
    });
    cx.simulate_path_prompt_response(|options| {
        assert!(!options.files && options.directories && !options.multiple);
        None
    });
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(view.read(cx).workspaces.path, "/previous");
        assert!(!view.read(cx).busy);
        assert!(api.requests.try_recv().is_err());
        window.render_frame(cx);
        window.click("project-browse", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec!["/selected project".into()]));
    cx.run_until_parked();
    cx.update(|cx| {
        let s = view.read(cx);
        assert_eq!(s.workspaces.path, "/selected project");
        assert_eq!(s.conversations[0].input, "Updated while choosing a folder");
        assert!(s.busy);
    });
    let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(request.starts_with("post /v1/projects "), "{request}");
    assert!(request.contains(r#""path":"/selected project""#), "{request}");
}

#[gpui_kit::test]
fn background_chat_submission_stays_in_the_conversation(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.providers = vec![ProviderInfo {
                provider: Provider::Claude,
                name: "Claude".into(),
                available: true,
                issue: None,
                capabilities: ProviderCapabilities {
                    background_jobs: true,
                    ..Default::default()
                },
            }];
            s.conversations[0].input = "Analyze this project".into();
        });
        window.render_frame(cx);
        window.click("send", cx);
        let s = view.read(cx);
        assert!(s.panel.is_none());
        assert_eq!(s.conversations[0].messages[0].text, "Analyze this project");
        assert!(s.busy);
        window.render_frame(cx);
        assert!(window.find("chat-view").visible());
        assert!(window.try_find("settings-sidebar").is_none());
    })
    .unwrap();
}

#[test]
fn titlebar_options_match_native_and_gpui_chrome() {
    use crate::titlebar::TitlebarMode;
    for (mode, custom) in [(TitlebarMode::Native, false), (TitlebarMode::Gpui, true)] {
        let options = mode.window_options();
        let titlebar = options.titlebar.unwrap();
        assert_eq!(titlebar.title.as_deref(), Some("Gpui Agent"));
        assert_eq!(titlebar.appears_transparent, custom);
        assert_eq!(options.app_owns_titlebar_drag, custom);
        assert_eq!(
            options.window_decorations,
            Some(if custom {
                gpui::WindowDecorations::Client
            } else {
                gpui::WindowDecorations::Server
            })
        );
    }
}

fn project_catalog() -> ProjectCatalog {
    serde_json::from_value(serde_json::json!({
        "default_workspace_id":42,
        "projects":[
            {"id":1,"name":"First","path":"/first","git":false,"workspaces":[
                {"id":42,"project_id":1,"path":"/first","main":true,"managed":false,"locked":false,"missing":false}
            ]},
            {"id":2,"name":"Opened","path":"/opened","git":false,"workspaces":[
                {"id":43,"project_id":2,"path":"/opened","main":true,"managed":false,"locked":false,"missing":false}
            ]}
        ]
    }))
    .unwrap()
}

fn wait_for_chat(view: &Entity<ChatApp>, cx: &mut TestAppContext) { wait_for_chat_state(view, cx, |s| !s.busy); }

fn wait_for_chat_state(view: &Entity<ChatApp>, cx: &mut TestAppContext, ready: impl Fn(&ChatApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let done = cx.update(|cx| {
            view.update(cx, |s, _| {
                s.updates(&s.wake.clone());
                ready(s)
            })
        });
        if done {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "chat request timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn wait_until(view: &Entity<ChatApp>, cx: &mut TestAppContext, ready: impl Fn(&ChatApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let done = cx.update(|cx| {
            view.update(cx, |s, _| {
                s.poll();
                ready(s)
            })
        });
        if done {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "UI state timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn background_fixture(cx: &mut TestAppContext, api: &AccountApi) -> (WindowHandle<gpui::base::Root>, Entity<ChatApp>) {
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.client.select_workspace(Some(42));
            s.workspaces.catalog = project_catalog();
            s.workspaces.loaded = true;
            s.workspaces.selected = Some(42);
            s.providers = vec![ProviderInfo {
                provider: Provider::Claude,
                name: "Claude".into(),
                available: true,
                issue: None,
                capabilities: ProviderCapabilities {
                    background_jobs: true,
                    ..Default::default()
                },
            }];
        })
    });
    (handle, view)
}

#[gpui_kit::test]
fn given_a_chat_without_manual_controls_when_a_job_runs_then_the_job_list_tracks_it(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = pending_computer_task(cx, &api);
    cx.simulate_window_resize(handle.into(), size(px(1440.), px(1000.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("chat-control").is_none());
    })
    .unwrap();
    wait_for_computer(&view, cx, |s| s.job_monitor.jobs.iter().any(|job| job.id == 5));
}

#[gpui_kit::test]
fn background_chat_streams_tools_approval_results_and_followups_without_navigation(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = background_fixture(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].input = "Analyze this project".into());
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.conversations[0].pending.is_some());
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.poll());
        window.render_frame(cx);
        let s = view.read(cx);
        assert!(s.busy);
        assert!(s.panel.is_none());
        assert_eq!(s.background_job, Some(5));
        assert_eq!(s.conversations[0].id, Some(7));
        assert!(
            s.conversations[0]
                .timeline
                .iter()
                .any(|entry| matches!(&entry.event, Event::Tool {call} if call.title.as_deref() == Some("Read project")))
        );
        assert!(window.find("chat-view").visible());
        assert!(window.find("allow-once").visible());
        assert!(window.try_find("settings-sidebar").is_none());
        window.click("allow-once", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        let s = view.read(cx);
        assert!(s.panel.is_none());
        assert!(s.background_job.is_none());
        assert!(s.conversations[0].pending.is_none());
        assert_eq!(s.conversations[0].messages.len(), 2);
        assert_eq!(s.conversations[0].messages[1].text, "Project analysis");
        assert!(!s.conversations[0].messages[1].incomplete);
        assert_eq!(s.conversations[0].session_id.as_deref(), Some("chat-session"));
        view.update(cx, |s, _| s.conversations[0].input = "Explain more".into());
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let s = view.read(cx);
        assert!(s.panel.is_none());
        assert_eq!(s.conversations[0].messages.len(), 4);
        assert_eq!(s.conversations[0].messages[0].text, "Analyze this project");
        assert_eq!(s.conversations[0].messages[1].text, "Project analysis");
        assert_eq!(s.conversations[0].messages[2].text, "Explain more");
        assert_eq!(s.conversations[0].messages[3].text, "Follow-up answer");
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    let submissions: Vec<_> = requests
        .iter()
        .filter(|request| request.starts_with("post /v1/providers/claude/jobs "))
        .collect();
    assert_eq!(submissions.len(), 2);
    assert!(submissions[1].contains(r#""conversation_id":7"#));
    assert!(submissions[1].contains(r#""session_id":"chat-session""#));
    assert!(submissions.iter().all(|request| request.contains("x-knd-workspace: 42")));
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("post /v1/jobs/5/interactions/777 ") && request.contains(r#""allow":true"#))
    );
    assert!(requests.iter().any(|request| request.starts_with("get /v1/jobs/5/events?after=5 ")));
    assert!(!requests.iter().any(|request| request.starts_with("post /v1/providers/claude/chat ")));
}

#[gpui_kit::test]
fn given_a_computer_task_when_submitted_then_grant_native_approval_and_result_stay_in_the_chat(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = background_fixture(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.selected = 1;
            s.conversations[1].status = s.conversations[0].status.clone();
            s.providers[0].provider = Provider::Codex;
            s.conversations[1].input = "Type Korean in the test editor".into();
        });
        window.render_frame(cx);
        window.click("computer-submit", cx);
    })
    .unwrap();
    wait_for_computer(&view, cx, |s| {
        s.conversations[1].pending.is_some() && s.computer.status.is_some() && s.computer.pending.is_none() && !s.job_monitor.jobs.is_empty()
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("computer-start").disabled(), Some(true));
        window.click("computer-grant", cx);
        window.render_frame(cx);
        window.click("computer-cancel", cx);
        assert_eq!(view.read(cx).background_job, Some(5));
        window.click("computer-grant", cx);
        window.render_frame(cx);
        window.click("computer-confirm", cx);
    })
    .unwrap();
    wait_for_computer(&view, cx, |s| {
        s.computer.pending.is_none() && s.computer.status.as_ref().is_some_and(|v| v.job_id == Some(5))
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("computer-start", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.conversations[1].pending.as_ref().is_some_and(|p| p.request_id == 778));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mcp-content-778").is_none());
        assert!(window.find("computer-pending-computer-input").visible());
        window.click("mcp-submit", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).conversations[1].pending.is_none());
        assert!(!view.read(cx).conversations[1].messages.last().unwrap().incomplete);
        window.click("chat-Some(42)-1-1--2-computer-input-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("computer-observation-computer-input").visible());
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/providers/codex/jobs ") && r.contains("\"computer_use\":true"))
    );
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/jobs/5/interactions/778 ") && r.contains("\"content\":{}"))
    );
}

#[gpui_kit::test]
fn background_chat_can_cancel_a_queued_job_and_displays_failures_in_place(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = background_fixture(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].input = "Queue this".into());
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.background_job.is_some());
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).operation.is_none());
        assert!(window.find("stop-operation").visible());
        window.click("stop-operation", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(view.read(cx).panel.is_none());
        assert!(view.read(cx).conversations[0].error.is_some());
        view.update(cx, |s, _| {
            s.conversations[0].new_chat();
            s.conversations[0].input = "Fail this".into();
        });
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let s = view.read(cx);
        assert!(s.panel.is_none());
        assert_eq!(s.conversations[0].error.as_deref(), Some("Job failed"));
        assert_eq!(s.conversations[0].messages[0].text, "Fail this");
        assert!(window.find("chat-view").visible());
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    assert!(requests.iter().any(|request| request.starts_with("delete /v1/jobs/5 ")));
    assert!(!requests.iter().any(|request| request.starts_with("delete /v1/operations/")));
}

fn pending_computer_task(cx: &mut TestAppContext, api: &AccountApi) -> (WindowHandle<gpui::base::Root>, Entity<ChatApp>) {
    let (handle, view) = background_fixture(cx, api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.selected = 1;
            s.conversations[1].status = s.conversations[0].status.clone();
            s.providers[0].provider = Provider::Codex;
            s.conversations[1].input = "Computer input awaiting approval".into();
        });
        window.render_frame(cx);
        window.click("computer-submit", cx);
    })
    .unwrap();
    wait_for_computer(&view, cx, |s| {
        s.computer_gate().is_some() && s.computer.pending.is_none() && s.computer.status.is_some() && !s.job_monitor.jobs.is_empty()
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("computer-grant", cx);
        window.render_frame(cx);
        window.click("computer-confirm", cx);
    })
    .unwrap();
    wait_for_computer(&view, cx, |s| {
        s.computer.pending.is_none() && s.computer.status.as_ref().is_some_and(|status| status.job_id == Some(5))
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("computer-start", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.conversations[1].pending.as_ref().is_some_and(|p| p.request_id == 778));
    (handle, view)
}

#[gpui_kit::test]
fn given_native_computer_approval_when_permission_is_revoked_then_accept_is_blocked_and_decline_remains_available(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = pending_computer_task(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("computer-revoke", cx);
        window.render_frame(cx);
        assert_eq!(window.find("mcp-submit").disabled(), Some(true));
    })
    .unwrap();
    wait_for_computer(&view, cx, |s| {
        s.computer.pending.is_none() && s.computer.status.as_ref().is_some_and(|status| status.job_id.is_none())
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("mcp-submit").disabled(), Some(true));
        assert_ne!(window.find("mcp-decline").disabled(), Some(true));
        window.click("mcp-decline", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    let requests: Vec<_> = api.requests.try_iter().collect();
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/control/computer/grant ") && r.contains("\"enabled\":false"))
    );
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/jobs/5/interactions/778 ") && r.contains("\"action\":\"decline\""))
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.starts_with("post /v1/jobs/5/interactions/778 ") && r.contains("\"action\":\"accept\""))
    );
}

#[gpui_kit::test]
fn given_native_computer_approval_when_the_task_is_cancelled_then_no_approval_is_sent_and_the_turn_is_incomplete(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = pending_computer_task(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("computer-job-cancel", cx);
        window.render_frame(cx);
        assert_eq!(window.find("mcp-submit").disabled(), Some(true));
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let s = view.read(cx);
        assert!(s.conversations[1].pending.is_none());
        assert!(s.conversations[1].messages.last().unwrap().incomplete);
    });
    let requests: Vec<_> = api.requests.try_iter().collect();
    assert!(requests.iter().any(|r| r.starts_with("delete /v1/jobs/5 ")));
    assert!(!requests.iter().any(|r| r.starts_with("post /v1/jobs/5/interactions/778 ")));
}

#[gpui_kit::test]
fn given_native_computer_approval_when_declined_then_the_failure_is_visible_without_an_input_observation(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = pending_computer_task(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("mcp-decline", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("chat-Some(42)-1-1--2-computer-input-toggle")
                .label()
                .is_some_and(|label| label.starts_with("failed"))
        );
        window.click("chat-Some(42)-1-1--2-computer-input-toggle", cx);
        window.render_frame(cx);
        assert!(window.try_find("computer-observation-computer-input").is_none());
        assert!(!view.read(cx).conversations[1].messages.last().unwrap().incomplete);
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/jobs/5/interactions/778 ") && r.contains("\"action\":\"decline\""))
    );
    assert!(
        !requests
            .iter()
            .any(|r| r.starts_with("post /v1/jobs/5/interactions/778 ") && r.contains("\"action\":\"accept\""))
    );
}

#[gpui_kit::test]
fn background_chat_reports_a_failed_cancellation_without_navigation(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (_, view) = background_fixture(cx, &api);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            s.background_job = Some(999);
            s.stop_chat();
        })
    });
    wait_for_chat_state(&view, cx, |s| s.conversations[0].error.is_some());
    cx.update(|cx| {
        let s = view.read(cx);
        assert!(s.panel.is_none());
        assert!(s.conversations[0].error.is_some());
    });
}

#[gpui_kit::test]
fn edited_question_preserves_the_draft_and_switches_between_saved_versions(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = background_fixture(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].input = "Analyze this project".into());
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.conversations[0].pending.is_some());
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("allow-once", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].input = "Keep my unsent draft".into());
        window.render_frame(cx);
        window.hover("chat-Some(42)-0-0-bubble", cx);
        window.click("chat-Some(42)-0-0-edit", cx);
        assert_eq!(view.read(cx).conversations[0].editing.as_deref(), Some("Analyze this project"));
        window.click("edit-question-cancel", cx);
        assert_eq!(view.read(cx).conversations[0].input, "Keep my unsent draft");
        window.hover("chat-Some(42)-0-0-bubble", cx);
        window.click("chat-Some(42)-0-0-edit", cx);
        window.render_frame(cx);
        let state = match &view.read(cx).inputs["composer-Some(42)-0"] {
            | Field::Text {
                state, ..
            } => state.clone(),
            | _ => panic!("composer"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.press("secondary-a", cx);
        window.input("Edited question", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(view.read(cx).conversations[0].editing.as_deref(), Some("Edited question"));
        assert_eq!(view.read(cx).conversations[0].input, "Keep my unsent draft");
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let s = view.read(cx);
        assert!(s.panel.is_none());
        assert_eq!(s.conversations[0].id, Some(8));
        assert_eq!(s.conversations[0].messages[0].text, "Edited question");
        assert_eq!(s.conversations[0].messages[1].text, "Edited answer");
        assert_eq!(s.conversations[0].versions.len(), 2);
        window.click("chat-Some(42)-0-0-version--1", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let s = view.read(cx);
        assert_eq!(s.conversations[0].id, Some(7));
        assert_eq!(s.conversations[0].messages[0].text, "Analyze this project");
        assert_eq!(s.conversations[0].messages[1].text, "Project analysis");
        assert_eq!(s.conversations[0].input, "Keep my unsent draft");
        window.click("chat-Some(42)-0-0-version-1", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let s = view.read(cx);
        assert_eq!(s.conversations[0].id, Some(8));
        assert_eq!(s.conversations[0].messages[1].text, "Edited answer");
        assert!(s.conversations[0].editing.is_none());
    });
    let requests: Vec<_> = api.requests.try_iter().collect();
    assert!(
        requests
            .iter()
            .any(|request| request.starts_with("post /v1/providers/claude/conversations/7/revisions "))
    );
    let revised = requests
        .iter()
        .find(|request| request.starts_with("post /v1/providers/claude/jobs ") && request.contains("Edited question"))
        .unwrap();
    assert!(revised.contains(r#""conversation_id":8"#) && revised.contains(r#""session_id":null"#));
}

#[gpui_kit::test]
fn opening_a_project_starts_a_new_chat_and_preserves_existing_conversations_and_drafts(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    for previous in [Some(42), Some(43), None] {
        let (handle, view) = fixture(cx);
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| {
                (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
                s.client = api.client.clone();
                s.client.select_workspace(previous);
                s.workspaces.catalog = project_catalog();
                s.workspaces.loaded = true;
                s.workspaces.selected = previous;
                s.conversations[0].id = Some(7);
                s.conversations[0].session_id = Some("original-session".into());
                s.conversations[0].input = "Existing draft".into();
                s.conversations[0].messages = vec![SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(1),
                    user: true,
                    text: "Existing message".into(),
                    attachments: vec![],
                    incomplete: false,
                }];
                if previous != Some(43) {
                    let mut target = std::array::from_fn(|_| gpui_agent_presentation::Conversation::default());
                    target[0].id = Some(7);
                    target[0].input = "Target draft".into();
                    target[0].attachments = vec![Attachment::Text {
                        name: "context.txt".into(),
                        text: "Keep this attachment".into(),
                        source: None,
                    }];
                    s.workspaces.cached.insert(Some(43), target);
                }
                s.open_panel(Panel::Workspace);
                s.workspaces.path = "/opened".into();
            });
            window.render_frame(cx);
            window.click("project-register", cx);
            assert_eq!(view.read(cx).panel, Some(Panel::Workspace));
        })
        .unwrap();
        wait_for_chat(&view, cx);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let s = view.read(cx);
            assert!(s.panel.is_none());
            assert_eq!(s.workspaces.selected, Some(43), "previous={previous:?}, error={:?}", s.workspaces.error);
            assert_eq!(s.client.workspace_id(), Some(43), "previous={previous:?}");
            assert_eq!(s.cwd, "/opened");
            let conversation = &s.conversations[0];
            assert!(conversation.id.is_none());
            assert!(conversation.session_id.is_none());
            assert!(conversation.messages.is_empty());
            assert!(conversation.input.is_empty());
            assert!(conversation.attachments.is_empty());
            assert_eq!(conversation.drafts.len(), 1);
            if previous != Some(43) {
                assert_eq!(s.workspaces.cached[&previous][0].input, "Existing draft");
                assert_eq!(s.workspaces.cached[&previous][0].messages[0].text, "Existing message");
            }
            assert!(window.find("chat-view").visible());
            assert!(window.try_find("settings-sidebar").is_none());
            window.click("chat-draft-0-0", cx);
        })
        .unwrap();
        wait_for_chat(&view, cx);
        cx.update(|cx| {
            let s = view.read(cx);
            assert_eq!(s.conversations[0].id, Some(7));
            assert_eq!(s.conversations[0].input, if previous == Some(43) { "Existing draft" } else { "Target draft" });
            if previous != Some(43) {
                assert_eq!(s.conversations[0].attachments.len(), 1);
            }
            assert!(s.conversations[0].drafts.is_empty());
        });
        if previous == Some(42) {
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("workspace-42", cx);
            })
            .unwrap();
            wait_for_chat(&view, cx);
            cx.update(|cx| {
                let s = view.read(cx);
                assert_eq!(s.conversations[0].id, Some(7));
                assert_eq!(s.conversations[0].input, "Existing draft");
                assert_eq!(s.conversations[0].messages[0].text, "Existing message");
            });
        }
    }
}

#[gpui_kit::test]
fn reopening_a_project_preserves_unsent_drafts_and_a_failed_open_keeps_settings(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.client.select_workspace(Some(43));
            s.workspaces.catalog = project_catalog();
            s.workspaces.loaded = true;
            s.workspaces.selected = Some(43);
            s.conversations[0].input = "Unsent draft".into();
            s.conversations[1].input = "Other provider draft".into();
            s.open_panel(Panel::Workspace);
            s.workspaces.path = "/opened".into();
        });
        window.render_frame(cx);
        window.click("project-register", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).conversations[0].input.is_empty());
        window.click("chat-draft-0-0", cx);
        assert_eq!(view.read(cx).conversations[0].input, "Unsent draft");
        assert_eq!(view.read(cx).conversations[1].input, "Other provider draft");
        view.update(cx, |s, _| {
            s.open_panel(Panel::Workspace);
            s.workspaces.path = "/missing".into();
        });
        window.render_frame(cx);
        window.click("project-register", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let s = view.read(cx);
        assert_eq!(s.panel, Some(Panel::Workspace));
        assert_eq!(s.workspaces.selected, Some(43));
        assert_eq!(s.conversations[0].input, "Unsent draft");
        assert!(s.workspaces.error.is_some());
    });
}

#[gpui_kit::test]
fn empty_project_opener_is_below_sidebar_search_and_the_body_only_explains_how_to_start(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        // Given no registered projects or conversations and an expanded
        // sidebar.
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.workspaces.loaded = true;
            s.workspaces.catalog = ProjectCatalog::default();
            s.workspaces.selected = None;
            s.sidebar_open = true;
        });
        window.render_frame(cx);
        // Then the opener belongs to the sidebar, directly below search,
        // outside the body.
        let sidebar = window.find("app-sidebar").bounds();
        let search = window.find("search").bounds();
        let opener = window.find("empty-open-project").bounds();
        assert!(opener.origin.x >= sidebar.origin.x && opener.right() <= sidebar.right());
        assert!(opener.origin.y >= search.bottom());
        assert!(
            window.try_find("project-add").is_none(),
            "Only one project-opening action is needed in the empty sidebar"
        );
        assert!(window.find("empty-project-state").visible());
        // When the button is clicked, then the existing project settings open.
        window.within("app-sidebar").click("empty-open-project", cx);
        assert_eq!(view.read(cx).panel, Some(Panel::Workspace));
    })
    .unwrap();
}

#[gpui_kit::test]
fn empty_project_opener_moves_to_the_header_when_the_sidebar_is_collapsed(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.workspaces.loaded = true;
            s.workspaces.catalog = ProjectCatalog::default();
            s.workspaces.selected = None;
            s.sidebar_open = false;
        })
    });
    for width in [1100., 500.] {
        cx.simulate_window_resize(handle.into(), size(px(width), px(780.)));
        cx.update_window(handle.into(), |_, window, cx| {
            // Given an empty app and collapsed sidebar at normal or minimum
            // window width.
            window.render_frame(cx);
            let header = window.find("chat-header").bounds();
            let opener = window.find("empty-open-project").bounds();
            assert!(window.find("empty-open-project").visible());
            assert!(opener.origin.y >= header.origin.y && opener.bottom() <= header.bottom());
            assert!(opener.right() <= header.right());
            assert!(window.try_find("app-sidebar").is_none());
            assert!(window.find("empty-project-state").visible());
            // When its header button is clicked, then project settings open.
            window.within("chat-header").click("empty-open-project", cx);
            assert_eq!(view.read(cx).panel, Some(Panel::Workspace));
            view.update(cx, |s, _| {
                s.close_panel();
                s.busy = false;
            });
            window.render_frame(cx);
        })
        .unwrap();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        // Reopening the sidebar moves the same action back below search.
        window.click("sidebar-toggle", cx);
        assert!(window.find("app-sidebar").visible());
        assert!(window.find("empty-open-project").bounds().right() <= window.find("app-sidebar").bounds().right());
        assert!(window.try_find("chat-header").is_none());
        // Existing projects retain their usual add-project icon and
        // conversation screen.
        view.update(cx, |s, _| {
            s.workspaces.catalog = project_catalog();
            s.workspaces.selected = Some(42);
        });
        window.render_frame(cx);
        assert!(window.try_find("empty-open-project").is_none());
        assert!(window.find("project-add").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn project_menu_removes_only_registration_and_preserves_the_draft(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    let catalog = ProjectCatalog {
        default_workspace_id: Some(42),
        registered_workspace_id: None,
        projects: vec![Project {
            verification: Default::default(),
            id: 7,
            name: "Project".into(),
            path: "/fixture".into(),
            git: false,
            workspaces: vec![Workspace {
                id: 42,
                project_id: 7,
                path: "/fixture".into(),
                branch: None,
                main: true,
                managed: false,
                locked: false,
                missing: false,
            }],
        }],
    };
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.client = api.client.clone();
            s.client.select_workspace(Some(42));
            s.workspaces.apply(catalog.clone());
            s.conversations[0].input = "Keep this draft".into();
            cx.notify();
        });
        window.render_frame(cx);
        window.hover("workspace-42", cx);
        window.click("project-menu-7", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(request.starts_with("delete /v1/projects/7 "), "{request}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let done = cx.update(|cx| {
            view.update(cx, |s, _| {
                let wake = s.wake.clone();
                s.updates(&wake);
                s.workspaces.selected.is_none() && !s.busy
            })
        });
        if done {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Removal timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let s = view.read(cx);
        assert!(s.client.workspace_id().is_none());
        assert_eq!(s.workspaces.cached[&Some(42)][0].input, "Keep this draft");
        assert!(window.find("empty-open-project").visible());
        assert!(!s.can_send());
        assert!(window.try_find("workspace-42").is_none());
    })
    .unwrap();
}

#[test]
fn removing_the_selected_project_chooses_an_available_workspace() {
    let mut ui = crate::workspace::WorkspaceUi {
        selected: Some(42),
        loaded: true,
        ..Default::default()
    };
    ui.apply(ProjectCatalog {
        default_workspace_id: Some(43),
        ..Default::default()
    });
    assert_eq!(ui.next_selection, Some(Some(43)));
    ui.apply(ProjectCatalog::default());
    assert_eq!(ui.next_selection, Some(None));
}

#[gpui_kit::test]
fn titlebar_switch_preserves_drafts_operations_and_rebinds_inputs(cx: &mut TestAppContext) {
    use crate::titlebar::TitlebarMode;
    let (handle, view) = fixture(cx);
    let preferences = std::env::temp_dir().join(format!("gpui-titlebar-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let preference = preferences.join("ui-titlebar");
    assert_eq!(TitlebarMode::load(&preference), TitlebarMode::Gpui);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.language_path = Some(preferences.join("ui-language"));
            s.conversations[0].input = "한글 draft".into();
            s.busy = true;
            s.operation = Some(42);
            s.open_panel(Panel::Appearance);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("title-bar").visible());
    })
    .unwrap();
    for (mode, button, saved) in [
        (TitlebarMode::Native, "titlebar-Native", "native"),
        (TitlebarMode::Gpui, "titlebar-Gpui", "gpui"),
    ] {
        let old = cx.update(|cx| cx.windows()[0]);
        cx.update_window(old, |_, window, cx| {
            window.render_frame(cx);
            window.click(button, cx);
        })
        .unwrap();
        let new = cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1);
            cx.windows()[0]
        });
        assert_ne!(old, new);
        assert_eq!(std::fs::read_to_string(&preference).unwrap(), saved);
        assert_eq!(TitlebarMode::load(&preference), mode);
        cx.update_window(new, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).titlebar_mode, mode);
            assert_eq!(view.read(cx).panel, Some(Panel::Appearance));
            assert!(view.read(cx).busy);
            assert_eq!(view.read(cx).operation, Some(42));
            assert_eq!(window.try_find("title-bar").is_some(), mode == TitlebarMode::Gpui);
            window.click("panel-close", cx);
            window.render_frame(cx);
            let state = match &view.read(cx).inputs["composer-None-0"] {
                | Field::Text {
                    state, ..
                } => state.clone(),
                | _ => panic!("composer"),
            };
            assert_eq!(state.read(cx).value().as_ref(), "한글 draft");
            state.update(cx, |s, cx| s.focus(window, cx));
            window.input("!", cx);
            window.render_frame(cx);
            assert_eq!(state.read(cx).value().as_ref(), "!한글 draft");
        })
        .unwrap();
        cx.update_window(new, |_, _, cx| {
            assert_eq!(view.read(cx).conversations[0].input, "!한글 draft");
            view.update(cx, |s, cx| {
                s.conversations[0].input = "한글 draft".into();
                s.open_panel(Panel::Appearance);
                cx.notify();
            });
        })
        .unwrap();
    }
    std::fs::write(&preference, "invalid").unwrap();
    assert_eq!(TitlebarMode::load(&preference), TitlebarMode::Gpui);
    std::fs::remove_file(preference).unwrap();
    std::fs::remove_dir(preferences).unwrap();
}

#[gpui_kit::test]
fn native_input_survives_redraws_language_and_provider_switches(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = match &view.read(cx).inputs["composer-None-0"] {
            | Field::Text {
                state, ..
            } => state.clone(),
            | _ => panic!("multiline composer"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.render_frame(cx);
        window.input("한글 🦀 draft", cx);
        window.render_frame(cx);
        assert_eq!(state.read(cx).value().as_ref(), "한글 🦀 draft");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(view.read(cx).conversations[0].input, "한글 🦀 draft");
        window.click("language", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).conversations[0].input, "한글 🦀 draft");
        view.update(cx, |v, cx| {
            v.selected = 1;
            cx.notify();
        });
        window.render_frame(cx);
        view.update(cx, |v, cx| {
            v.selected = 0;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(view.read(cx).conversations[0].input, "한글 🦀 draft");
        assert!(view.read(cx).inputs.contains_key("composer-None-0"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_native_panel_renders_at_the_minimum_window_size(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    for panel in [
        Panel::General,
        Panel::Appearance,
        Panel::Accounts,
        Panel::Guidance,
        Panel::Skills,
        Panel::Plugins,
        Panel::Workspace,
        Panel::Git,
        Panel::Pr,
        Panel::Mcp,
        Panel::Attach,
    ] {
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |v, cx| {
                v.sidebar_open = false;
                v.open_panel(panel);
                cx.notify();
            });
            window.render_frame(cx);
            let close = if panel.is_chat_tool() { "chat-tool-close" } else { "panel-close" };
            assert!(window.find(close).visible(), "panel {:?}", panel.title());
            window.click(close, cx);
            assert!(view.read(cx).panel.is_none());
            assert!(view.read(cx).chat_tool.is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn chat_tools_preserve_attachment_and_pr_drafts(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1440.), px(900.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].input = "Keep my chat draft".into();
            s.workspaces.selected = Some(7);
        });
        window.render_frame(cx);
        window.click("attach", cx);
        assert_eq!(view.read(cx).chat_tool, Some(Panel::Attach));
        assert!(view.read(cx).panel.is_none());
        assert!(window.find("chat-view").visible());
        assert!(window.find("chat-tool-panel").visible());
        assert!(window.try_find("settings-sidebar").is_none());
        window.click("attach-context", cx);
        view.update(cx, |s, _| {
            s.attachment_ui.path = "src/main.rs".into();
            s.attachment_ui.start = "invalid".into();
        });
        window.render_frame(cx);
        window.click("attach-add", cx);
        assert!(view.read(cx).attachment_ui.error.is_some());
        assert_eq!(view.read(cx).chat_tool, Some(Panel::Attach));
        assert!(!view.read(cx).busy);
        window.click("chat-pr", cx);
        let title = match &view.read(cx).inputs["pr-title"] {
            | Field::Line {
                state, ..
            } => state.clone(),
            | _ => panic!("PR title"),
        };
        title.update(cx, |s, cx| s.focus(window, cx));
        window.render_frame(cx);
        window.input("Keep my PR title", cx);
        assert_eq!(title.read(cx).value().as_ref(), "Keep my PR title");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _window, cx| {
        assert_eq!(view.read(cx).workspaces.git.pr.title, "Keep my PR title");
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("chat-header").visible());
        assert!(window.find("chat-tool-close").visible());
        assert!(window.try_find("settings-header").is_none());
        window.press("escape", cx);
        assert!(view.read(cx).chat_tool.is_none());
        view.update(cx, |s, _| s.sidebar_open = false);
        window.render_frame(cx);
        assert!(window.find("composer-surface").visible());
        assert_eq!(view.read(cx).conversations[0].input, "Keep my chat draft");
        window.click("chat-pr", cx);
        assert_eq!(view.read(cx).workspaces.git.pr.title, "Keep my PR title");
        assert!(view.read(cx).workspaces.git.pr.draft);
        assert!(!view.read(cx).workspaces.git.pr.push);
        window.click("chat-tool-close", cx);
        window.click("attach", cx);
        assert_eq!(view.read(cx).attachment_ui.path, "src/main.rs");
        assert_eq!(view.read(cx).attachment_ui.start, "invalid");
        window.click("chat-tool-close", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_missing_runtime_when_opening_setup_from_chat_then_the_matching_provider_is_selected(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.selected = provider_index(Provider::Antigravity);
            s.providers = vec![ProviderInfo {
                provider: Provider::Antigravity,
                name: "Gemini / Antigravity".into(),
                available: false,
                issue: Some("Runtime missing".into()),
                capabilities: Default::default(),
            }];
        });
        window.render_frame(cx);
        assert!(window.try_find("chat-login").is_none(), "Install the runtime before offering login");
        window.click("chat-runtime-settings", cx);
        assert_eq!(view.read(cx).panel, Some(Panel::Accounts));
        assert_eq!(view.read(cx).accounts.selected, provider_index(Provider::Antigravity));
        assert!(!view.read(cx).busy, "Opening setup must not start login or installation");
        window.render_frame(cx);
        assert!(window.try_find("runtime-Antigravity").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_missing_runtime_when_checking_installation_then_a_review_is_offered_before_installing(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.panel = Some(Panel::Accounts);
            s.accounts.selected = provider_index(Provider::Antigravity);
            s.maintenance.apply(RuntimeInfo {
                provider: Provider::Antigravity,
                active_path: None,
                managed_version: None,
                override_active: false,
                release: None,
                releases: vec![],
                issue: Some("Runtime missing".into()),
            });
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert_eq!(window.find("runtime-Antigravity-업데이트 확인").label(), Some("Find a version to install"));
        window.click("runtime-Antigravity-업데이트 확인", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(view.read(cx).maintenance.error.is_none(), "{:?}", view.read(cx).maintenance.error);
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-review-Antigravity", cx);
        assert_eq!(view.read(cx).maintenance.confirm.as_ref().unwrap().0, Provider::Antigravity);
        assert!(!view.read(cx).busy);
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-cancel", cx);
        assert!(view.read(cx).maintenance.confirm.is_none());
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    let checks: Vec<_> = requests.iter().filter(|r| r.starts_with("post /v1/runtimes/")).collect();
    assert_eq!(checks.len(), 1);
    assert!(checks[0].starts_with("post /v1/runtimes/antigravity ") && checks[0].contains(r#""type":"check""#));
}

#[gpui_kit::test]
fn given_old_and_new_antigravity_versions_when_either_is_selected_then_its_reviewed_version_is_installed(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    let (handle, view) = fixture(cx);
    for (version, checksum, key) in [("1.4.1", "ab", "down"), ("1.4.5", "cd", "up")] {
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| {
                s.client = api.client.clone();
                s.panel = Some(Panel::Accounts);
                s.accounts.selected = provider_index(Provider::Antigravity);
                s.maintenance.apply(
                    serde_json::from_value(serde_json::json!({
                        "provider":"antigravity", "active_path":null, "managed_version":null,
                        "override_active":false, "release":null, "issue":null,
                        "releases":[
                            {"version":"1.4.5","url":"https://dl.google.com/new.zip","sha256":"cd".repeat(32),"size":null},
                            {"version":"1.4.1","url":"https://dl.google.com/old.zip","sha256":"ab".repeat(32),"size":null}
                        ]
                    }))
                    .unwrap(),
                );
            });
            window.render_frame(cx);
            window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
            assert!(window.try_find("runtime-review-Antigravity-1.4.5").is_none());
            assert!(window.try_find("runtime-review-Antigravity-1.4.1").is_none());
            window.click("runtime-versions-Antigravity", cx);
            window.press(key, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            assert!(view.read(cx).maintenance.confirm.is_none(), "Selecting must not review or install");
            window.click("runtime-review-Antigravity", cx);
            let release = &view.read(cx).maintenance.confirm.as_ref().unwrap().1;
            assert_eq!(release.version, version);
            assert_eq!(release.sha256, checksum.repeat(32));
            assert!(!view.read(cx).busy, "Reviewing must not install");
            window.render_frame(cx);
            window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
            window.click("runtime-install", cx);
        })
        .unwrap();
        wait_for_chat(&view, cx);
        let requests: Vec<_> = api.requests.try_iter().collect();
        let install = requests.iter().find(|r| r.starts_with("post /v1/runtimes/antigravity ")).unwrap();
        let body: serde_json::Value = serde_json::from_str(install.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"type":"install","version":version,"sha256":checksum.repeat(32)}));
    }
}

#[gpui_kit::test]
fn given_a_selected_runtime_version_when_versions_refresh_then_selection_is_preserved_or_falls_back_to_the_first_available(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let info = |versions: &[&str], checksum: &str| RuntimeInfo {
        provider: Provider::Antigravity,
        active_path: None,
        managed_version: None,
        override_active: false,
        release: None,
        issue: None,
        releases: versions
            .iter()
            .map(|version| RuntimeRelease {
                version: (*version).into(),
                url: "https://dl.google.com/fixture.zip".into(),
                sha256: checksum.repeat(32),
                size: None,
            })
            .collect(),
    };
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Accounts);
            s.accounts.selected = provider_index(Provider::Antigravity);
            s.maintenance.apply(info(&["1.4.5", "1.4.1"], "ab"));
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-versions-Antigravity", cx);
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    for (versions, expected) in [(vec!["1.5.0", "1.4.5", "1.4.1"], "1.4.1"), (vec!["1.5.0", "1.4.5"], "1.5.0")] {
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| s.maintenance.apply(info(&versions, "cd")));
            window.render_frame(cx);
            window.click("runtime-review-Antigravity", cx);
            let release = &view.read(cx).maintenance.confirm.as_ref().unwrap().1;
            assert_eq!(release.version, expected);
            assert_eq!(release.sha256, "cd".repeat(32), "Review must use refreshed metadata");
            window.render_frame(cx);
            window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
            window.click("runtime-cancel", cx);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn given_a_failed_runtime_lookup_when_settings_are_open_then_the_error_is_visible_and_retry_is_enabled(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    api.history_mode.store(12, std::sync::atomic::Ordering::Relaxed);
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.panel = Some(Panel::Accounts);
            s.accounts.selected = provider_index(Provider::Antigravity);
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-Antigravity-업데이트 확인", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("runtime-error-Antigravity").is_some());
        assert_ne!(window.find("runtime-Antigravity-업데이트 확인").disabled(), Some(true));
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(10000.))), cx);
        window.click("accounts-provider-claude", cx);
        window.render_frame(cx);
        assert!(window.try_find("runtime-error-Antigravity").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_reviewed_runtime_when_installed_then_accounts_refresh_and_login_is_enabled(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.panel = Some(Panel::Accounts);
            s.accounts.selected = provider_index(Provider::Antigravity);
            s.accounts.providers[2].accounts[0].auth.error = Some("Runtime missing".into());
            s.maintenance.confirm = Some((
                Provider::Antigravity,
                RuntimeRelease {
                    version: "1.4.1".into(),
                    url: "https://example.com/runtime".into(),
                    sha256: "ab".repeat(32),
                    size: None,
                },
            ));
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-install", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    wait_until(&view, cx, |s| {
        let state = &s.accounts.providers[2];
        !state.loading && !state.accounts[0].busy && state.accounts[0].auth.status.is_some()
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(10000.))), cx);
        let account = &view.read(cx).accounts.providers[2].accounts[0];
        assert!(account.auth.error.is_none());
        assert!(!account.auth.status.as_ref().unwrap().authenticated);
        assert_ne!(window.find("account-login-antigravity-default").disabled(), Some(true));
        assert!(
            view.read(cx).conversations[0].status.as_ref().unwrap().authenticated,
            "Installing another provider must preserve the chat account"
        );
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    let install = requests.iter().find(|r| r.starts_with("post /v1/runtimes/antigravity ")).unwrap();
    let body: serde_json::Value = serde_json::from_str(install.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body, serde_json::json!({"type":"install","version":"1.4.1","sha256":"ab".repeat(32)}));
    assert!(
        !requests.iter().any(|r| r.starts_with("post ") && r.contains("/login ")),
        "Installation must not start OAuth automatically"
    );
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("account-login-antigravity-default", cx);
    })
    .unwrap();
    wait_until(&view, cx, |s| {
        let account = &s.accounts.providers[2].accounts[0];
        !account.busy && account.auth.status.as_ref().is_some_and(|status| status.authenticated)
    });
    assert!(api.requests.try_iter().any(|r| r.starts_with("post /v1/providers/antigravity/login ")));
}

#[gpui_kit::test]
fn account_settings_scope_runtime_installation_to_the_selected_provider(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.panel = Some(Panel::Accounts);
            s.maintenance.apply(RuntimeInfo {
                provider: Provider::Claude,
                active_path: None,
                managed_version: None,
                override_active: false,
                release: Some(RuntimeRelease {
                    version: "1.2.3".into(),
                    url: "https://example.com/runtime".into(),
                    sha256: "ab".repeat(32),
                    size: None,
                }),
                releases: vec![],
                issue: None,
            });
        });
        window.render_frame(cx);
        for panel in ["Runtime", "Attach", "Pr", "Control"] {
            assert!(window.try_find(format!("settings-nav-{panel}")).is_none());
        }
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-review-Claude", cx);
        assert_eq!(view.read(cx).maintenance.confirm.as_ref().unwrap().0, Provider::Claude);
        assert!(!view.read(cx).busy, "Review must not install the runtime");
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.click("runtime-cancel", cx);
        assert!(view.read(cx).maintenance.confirm.is_none());
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(10000.))), cx);
        window.click("accounts-provider-codex", cx);
        window.render_frame(cx);
        assert!(window.try_find("runtime-Claude").is_none());
        assert!(window.try_find("runtime-Codex").is_some());
        assert!(view.read(cx).maintenance.confirm.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_delete_dialog_when_tabbing_then_focus_stays_inside_until_cancelled(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        // Given a destructive confirmation over an editable conversation.
        view.update(cx, |s, _| {
            s.dialog = Some(HistoryDialog::Delete {
                provider: 0,
                id: 7,
                title: "Keep me".into(),
            });
        });
        window.render_frame(cx);
        // When navigating forward and backward, Then only dialog actions
        // receive focus.
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("history-cancel").focused(), Some(true));
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("delete-confirm").focused(), Some(true));
        window.press("tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("history-cancel").focused(), Some(true));
        window.press("shift-tab", cx);
        window.render_frame(cx);
        assert_eq!(window.find("delete-confirm").focused(), Some(true));
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("delete-confirm").is_none());
        assert!(!view.read(cx).busy);
    })
    .unwrap();
}

#[gpui_kit::test]
fn delete_cancel_keeps_the_conversation_and_approval_needs_a_click(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |v, cx| {
            v.conversations[0].id = Some(7);
            v.dialog = Some(HistoryDialog::Delete {
                provider: 0,
                id: 7,
                title: "Keep me".into(),
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert!(!view.read(cx).busy);
        window.click("history-cancel", cx);
        assert_eq!(view.read(cx).conversations[0].id, Some(7));
        assert!(!view.read(cx).busy);
        view.update(cx, |v, cx| {
            v.conversations[0].pending = Some(Pending::new(
                42,
                777,
                Event::Permission {
                    method: "claude/can_use_tool".into(),
                    details: serde_json::json!({"input":{"command":"echo test"}}),
                },
            ));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(!view.read(cx).conversations[0].pending.as_ref().unwrap().sending);
        assert!(window.find("allow-once").visible());
        window.click("deny", cx);
        assert!(view.read(cx).conversations[0].pending.as_ref().unwrap().sending);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_an_mcp_form_the_native_panel_requires_valid_input_and_has_separate_decline(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |v, cx| {
            v.conversations[0].pending = Some(Pending::new(
                42,
                777,
                Event::Elicitation {
                    server_name: "fixture".into(),
                    message: "Choose count".into(),
                    url: None,
                    schema: Some(serde_json::json!({"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]})),
                },
            ));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("mcp-submit").visible());
        assert!(window.find("mcp-decline").visible());
        window.click("mcp-submit", cx);
        assert!(!view.read(cx).conversations[0].pending.as_ref().unwrap().sending);
        let state = match &view.read(cx).inputs["mcp-content-777"] {
            | Field::Text {
                state, ..
            } => state.clone(),
            | _ => panic!("MCP JSON input"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.input("{\"count\":2}", cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).conversations[0].pending.as_ref().unwrap().response().is_some());
        window.click("mcp-submit", cx);
        assert!(view.read(cx).conversations[0].pending.as_ref().unwrap().sending);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_language_choices_when_changed_then_native_toggle_state_tracks_selection(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.open_panel(Panel::General));
        window.render_frame(cx);
        assert_eq!(window.find("settings-language-English").checked(), Some(true));
        assert_eq!(window.find("settings-language-Korean").checked(), Some(false));
        window.click("settings-language-Korean", cx);
        window.render_frame(cx);
        assert_eq!(window.find("settings-language-Korean").checked(), Some(true));
        assert_eq!(window.find("settings-language-English").checked(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn native_question_requires_an_answer_and_keeps_multiple_choices(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |v, cx| {
            v.conversations[0].pending = Some(Pending::new(
                42,
                777,
                Event::UserInput {
                    questions: vec![Question {
                        id: "q".into(),
                        question: "Choose".into(),
                        options: vec!["A".into(), "B".into()],
                        multi_select: true,
                    }],
                },
            ));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(view.read(cx).conversations[0].pending.as_ref().unwrap().response().is_none());
        assert_eq!(window.find("question-777-q-0").role(), Some(gpui::Role::CheckBox));
        window.click("question-777-q-0", cx);
        window.click("question-777-q-1", cx);
        let p = view.read(cx).conversations[0].pending.as_ref().unwrap();
        assert_eq!(p.answers["q"], vec!["A", "B"]);
        assert!(!p.sending);
        window.click("send-answers", cx);
        assert!(view.read(cx).conversations[0].pending.as_ref().unwrap().sending);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_codex_chat_when_execution_policies_are_selected_then_the_submitted_job_keeps_both_choices(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = background_fixture(cx, &api);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.selected = 1;
            s.conversations[1].status = Some(AuthStatus {
                authenticated: true,
                account: None,
                method: None,
            });
            s.providers.push(ProviderInfo {
                provider: Provider::Codex,
                name: "Codex".into(),
                available: true,
                issue: None,
                capabilities: ProviderCapabilities {
                    background_jobs: true,
                    ..Default::default()
                },
            });
            s.conversations[1].input = "Scoped follow up".into();
        });
        window.render_frame(cx);
        window.click("codex-chat-approval-picker", cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("codex-chat-sandbox-picker", cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.background_job.is_some());
    let requests: Vec<_> = api.requests.try_iter().collect();
    let request = requests
        .iter()
        .find(|request| request.starts_with("post /v1/providers/codex/jobs "))
        .expect("Codex job must be submitted");
    assert!(request.contains("\"codex_execution_policy\":{\"approval_policy\":\"on-request\",\"sandbox\":\"workspace-write\"}"));
}

#[gpui_kit::test]
fn native_markdown_search_and_copy_keep_the_message_source(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let source = "# Rust\n\n- native UI\n\n```rust\nfn main() {}\n```\n\n![blocked](data:image/png;base64,invalid)";
    cx.update_window(handle.into(), |_, window, cx| {
        // Toast entrance animations use wall-clock time, not the test executor.
        cx.set_reduce_motion(true);
        view.update(cx, |v, cx| {
            v.conversations[0].messages.push(SavedMessage {
                skills: vec![],
                guidance: None,
                id: Some(1),
                user: false,
                text: source.into(),
                attachments: vec![],
                incomplete: false,
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("chat-None-0-0-copy").visible());
        window.hover("chat-None-0-0-bubble", cx);
        window.click("chat-None-0-0-copy", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some(source));
        window.render_frame(cx);
        assert!(window.find("clipboard-feedback").visible());
        view.update(cx, |v, cx| {
            v.conversations[0].view_query = "Rust".into();
            cx.notify();
        });
        window.render_frame(cx);
        window.click("chat-None-0-0-copy", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some(source));
    })
    .unwrap();
}

#[gpui_kit::test]
fn compact_chat_keeps_send_visible_with_a_large_model_catalog(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |v, cx| {
            v.sidebar_open = false;
            v.conversations[0].models = (0 .. 100)
                .map(|i| Model {
                    id: format!("model-{i}"),
                    name: format!("Model {i}"),
                })
                .collect();
            cx.notify();
        });
        window.render_frame(cx);
        let send = window.find("send");
        assert!(send.visible());
        assert!(send.bounds().bottom() <= gpui::px(360.), "send bounds: {:?}", send.bounds());
        assert!(window.try_find("composer-context").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_closes_a_reopened_panel_after_its_input_was_unmounted(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.accounts.providers[0].managing = true);
        window.render_frame(cx);
        window.click("settings-toggle", cx);
        window.within("popup-menu").click(1usize, cx);
        let state = match &view.read(cx).inputs["account-label-claude"] {
            | Field::Line {
                state, ..
            } => state.clone(),
            | _ => panic!("account label input"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.render_frame(cx);
        window.click("panel-close", cx);
        window.click("settings-toggle", cx);
        window.within("popup-menu").click(1usize, cx);
        assert!(view.read(cx).panel.is_some());
        window.press("escape", cx);
        assert!(view.read(cx).panel.is_none());
        window.click("settings-toggle", cx);
        window.within("popup-menu").click(1usize, cx);
        window.press("escape", cx);
        assert!(view.read(cx).panel.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn theme_switch_preserves_the_draft_and_compact_send_action(cx: &mut TestAppContext) {
    use gpui::component::{ActiveTheme,
                          Theme,
                          ThemeMode};
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.sidebar_open = false;
            s.conversations[0].input = "Keep this draft".into();
            cx.notify();
        });
        Theme::change(ThemeMode::Light, Some(window), cx);
        window.render_frame(cx);
        for expected in [ThemeMode::Dark, ThemeMode::Light] {
            window.click("theme-toggle", cx);
            window.render_frame(cx);
            assert_eq!(cx.theme().mode, expected);
            assert_eq!(view.read(cx).conversations[0].input, "Keep this draft");
            assert!(window.find("send").visible());
            assert!(window.find("send").bounds().bottom() <= px(340.));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_menu_matches_sidebar_names_groups_and_order(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    for language in [Language::Korean, Language::English] {
        let expected_git = if language == Language::Korean { "Git 상태" } else { "Git status" };
        assert_eq!(language.text(Panel::Git.title()), expected_git);
        assert_eq!(language.text(Panel::Git.nav_title()), expected_git);
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, cx| {
                s.language = language;
                cx.notify();
            });
            window.render_frame(cx);
            // Given the chat, When opening settings, Then popup and sidebar use
            // the same setup-first groups, names and order.
            let settings = window.within("app-sidebar").find("settings-toggle");
            assert!(settings.visible());
            assert_eq!(settings.label(), Some(if language == Language::Korean { "설정" } else { "Settings" }));
            assert_eq!(settings.bounds().origin.x, window.find("archived").bounds().origin.x);
            assert!(settings.bounds().center().x < window.find("app-sidebar").bounds().center().x);
            assert!(settings.bounds().top() >= window.find("archived").bounds().bottom());
            assert!(window.try_find("tools-toggle").is_none());
            assert!(window.try_find("accounts-open").is_none());
            window.click("settings-toggle", cx);
            let labels: Vec<_> = window
                .within("popup-menu")
                .elements()
                .iter()
                .filter_map(|element| element.label().map(str::to_owned))
                .collect();
            let expected = match language {
                | Language::Korean => vec!["작업 환경", "계정", "프로젝트", "에이전트 구성", "지침", "플러그인", "앱 환경", "일반", "모양"],
                | Language::English => vec![
                    "Work environment",
                    "Account",
                    "Projects",
                    "Agent configuration",
                    "Instructions",
                    "Plugins",
                    "App preferences",
                    "General",
                    "Appearance",
                ],
            };
            assert_eq!(labels, expected);
            for (index, panel) in [
                (1, Panel::Accounts),
                (2, Panel::Workspace),
                (4, Panel::Guidance),
                (5, Panel::Plugins),
                (7, Panel::General),
                (8, Panel::Appearance),
            ] {
                if index != 1 {
                    window.click("settings-toggle", cx);
                }
                if !window.within("popup-menu").find(index).visible() {
                    window.scroll("popup-menu", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
                }
                window.within("popup-menu").click(index, cx);
                assert_eq!(view.read(cx).panel, Some(panel));
                let sidebar_labels: Vec<_> = window
                    .within("settings-sidebar")
                    .elements()
                    .iter()
                    .filter_map(|element| element.label().map(str::to_owned))
                    .filter(|label| expected.contains(&label.as_str()))
                    .collect();
                assert_eq!(sidebar_labels, expected);
                assert_eq!(window.within("settings-sidebar").find_by_label(expected[index]).label(), Some(expected[index]));
                if index >= 13 {
                    window.scroll("settings-nav-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
                    assert!(window.within("settings-sidebar").find_by_label(expected[index]).visible());
                }
                window.click("panel-close", cx);
                assert!(view.read(cx).panel.is_none(), "return from {panel:?}");
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn compact_chat_can_switch_providers_without_losing_the_other_draft(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.sidebar_open = false;
            s.conversations[0].input = "Claude draft".into();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("provider-picker").visible());
        window.click("provider-picker", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
        assert_eq!(view.read(cx).selected, 1);
        assert_eq!(view.read(cx).conversations[0].input, "Claude draft");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_long_claude_approval_in_a_small_window_when_scrolled_then_approval_and_cancel_are_reachable(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(640.), px(480.)));
    for action in ["allow-once", "cancel-request"] {
        cx.update_window(handle.into(), |_, window, cx| {
            view.update(cx, |s, _| {
                s.conversations[0].pending = Some(Pending::new(
                    42,
                    777,
                    Event::Permission {
                        method: "claude/can_use_tool".into(),
                        details: serde_json::json!({
                            "input":{"command":"fixture-command"},
                            "permission_suggestions":[
                                {"behavior":"allow","destination":"session","rules":[{"toolName":"Write"}],"type":"addRules"},
                                {"behavior":"allow","destination":"localSettings","rules":[{"toolName":"Bash","ruleContent":"cargo test:*"}],"type":"addRules"}
                            ]
                        }),
                    },
                ));
            });
            window.render_frame(cx);
            window.scroll("confirmation-dialog", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(10000.))), cx);
            window.render_frame(cx);
            let inside = |window: &gpui::Window| {
                let modal = window.find("confirmation-dialog").bounds();
                let button = window.find(action).bounds();
                button.top() >= modal.top() && button.bottom() <= modal.bottom()
            };
            for _ in 0 .. 80 {
                if inside(window) {
                    break;
                }
                window.scroll("confirmation-dialog", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-60.))), cx);
                window.render_frame(cx);
            }
            assert!(inside(window), "{action} must be reachable by scrolling inside the small modal");
            window.click(action, cx);
            assert!(view.read(cx).conversations[0].pending.as_ref().unwrap().sending);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn long_tool_forms_scroll_to_the_last_action_while_the_header_stays_visible(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Accounts);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(view.read(cx).panel_scroll.max_offset().y > px(0.), "form must overflow");
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.render_frame(cx);
        assert!(view.read(cx).panel_scroll.offset().y < px(0.), "wheel must scroll the form");
        assert!(window.find("panel-close").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_accounts_with_the_settings_sidebar_closed_when_space_is_available_then_provider_tabs_are_on_the_left(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1000.), px(700.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Accounts);
            s.sidebar_open = false;
            cx.notify();
        });
        window.render_frame(cx);
        let tab = window.find("accounts-provider-claude").bounds();
        let card = window.find("accounts-claude").bounds();
        assert!(tab.right() <= card.left(), "provider tabs should be beside the account card");
    })
    .unwrap();
}

#[gpui_kit::test]
fn accounts_show_all_providers_and_manage_only_the_clicked_provider(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Accounts);
            s.selected = 1;
            s.conversations[1].input = "keep my draft".into();
            cx.notify();
        });
        window.render_frame(cx);
        for provider in PROVIDERS {
            let key = provider_name(provider);
            window.click(format!("accounts-provider-{key}"), cx);
            assert!(window.find(format!("accounts-{key}")).visible());
            assert!(window.find(format!("account-login-{key}-default")).visible());
            assert!(window.find(format!("account-logout-{key}-default")).visible());
        }
        window.click("accounts-provider-codex", cx);
        window.click("accounts-manage-codex", cx);
        window.render_frame(cx);
        assert!(view.read(cx).accounts.providers[1].managing);
        assert!(!view.read(cx).accounts.providers[0].managing);
        assert!(!view.read(cx).accounts.providers[2].managing);
        assert!(view.read(cx).inputs.contains_key("account-label-codex"));
        assert_eq!(view.read(cx).selected, 1);
        assert_eq!(view.read(cx).conversations[1].input, "keep my draft");
        window.click("accounts-provider-antigravity", cx);
        window.click("settings-nav-General", cx);
        window.click("settings-nav-Accounts", cx);
        assert_eq!(view.read(cx).accounts.selected, 2);
        assert_eq!(view.read(cx).selected, 1);
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("accounts-provider-claude", cx);
        assert!(window.find("accounts-claude").visible());
        window.click("accounts-provider-antigravity", cx);
        window.render_frame(cx);
        assert!(window.find("accounts-antigravity").visible());
        assert!(window.find("panel-close").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_host_verification_receipts_when_chat_is_opened_then_command_results_and_the_exact_version_are_in_the_work_panel(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(),|_,window,cx| {
        view.update(cx,|s,_| {
            s.conversations[0].messages=vec![SavedMessage{ skills: vec![], guidance: None, id:Some(2),user:false,text:"Implemented".into(),attachments:vec![],incomplete:false}];
            let report:VerificationReport=serde_json::from_value(serde_json::json!({"job_id":7,"attempt":2,"revision":{"head":"abc123","fingerprint":"working-tree-456"},"finished_revision":{"head":"abc123","fingerprint":"working-tree-456"},"checks":[{"command":["bun","test","server"],"status":"passed","exit_code":0,"output":"2 passed"}]})).unwrap();
            s.conversations[0].timeline=vec![TimelineEntry {message_id:2,event:Event::Verification {report}}];
        });
        window.render_frame(cx);
        let panel=window.find("work-panel").bounds();
        let receipt=window.find("verification-run-7-2").bounds();
        assert!(receipt.left()>=panel.left());
        assert!(window.find("verification-check-7-2-0-toggle").visible());
        window.click("verification-check-7-2-0-toggle",cx);
        window.render_frame(cx);
        assert!(window.find("verification-check-7-2-0-details").visible());
        window.hover("work-panel-close",cx);
        window.render_frame(cx);
        window.click("work-panel-close",cx);
        window.render_frame(cx);
        assert!(!view.read(cx).right_panels.work_open);
        assert!(window.try_find("verification-run-7-2").is_none());
        assert!(window.find("chat-None-0-0-bubble").visible());
        window.click("work-toggle",cx);
        window.render_frame(cx);
        window.click("verification-7-2-copy-version",cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(),Some("HEAD: abc123\nSHA-256: working-tree-456"));
    }).unwrap();
}

#[gpui_kit::test]
fn given_execution_activity_when_chat_is_rendered_then_work_is_separate_and_attention_stays_in_the_center(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].messages = vec![SavedMessage {
                skills: vec![],
                guidance: None,
                id: Some(2),
                user: false,
                text: "Working on it".into(),
                attachments: vec![],
                incomplete: true,
            }];
            s.conversations[0].timeline = serde_json::from_value(serde_json::json!([
                {"message_id":2,"event":{"type":"tool","call":{"id":"read","title":"Read file","status":"in_progress"}}},
                {"message_id":2,"event":{"type":"runtime_status","status":"error","message":"Provider disconnected","code":null}},
                {"message_id":2,"event":{"type":"interaction_resolved","request_id":1,"response":{"type":"approval","allow":true}}}
            ]))
            .unwrap();
            s.conversations[0].error = Some("Task failed".into());
        });
        window.render_frame(cx);
        let chat = window.find("chat-view").bounds();
        let work = window.find("work-panel").bounds();
        assert_eq!(
            window.find("work-toggle").bounds().size.width,
            window.find("refresh-chat").bounds().size.width,
            "The activity control uses the same compact icon button size as its neighbors"
        );
        assert!(work.left() >= chat.right(), "Execution must occupy the right panel");
        window.click("work-panel-maximize", cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").bounds().size.width > work.size.width);
        assert!(window.try_find("chat-view").is_none());
        window.click("work-panel-maximize", cx);
        window.render_frame(cx);
        assert_eq!(window.find("work-panel").bounds().size.width, work.size.width);
        assert!(window.find("chat-None-0-0-2-read-toggle").bounds().left() >= work.left());
        assert!(window.find("chat-None-0-0-2-status-0").bounds().right() <= chat.right());
        assert!(window.find("chat-None-0-0-2-confirmation-1").bounds().right() <= chat.right());
        window.click("work-panel-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("work-panel").is_none());
        assert!(window.try_find("chat-None-0-0-2-read-toggle").is_none());
        assert!(window.find("dismiss-error").visible());
        assert!(window.find("chat-None-0-0-2-status-0").visible());
        for (event, action) in [
            (
                Event::Permission {
                    method: "fixture".into(),
                    details: serde_json::json!({}),
                },
                "allow-once",
            ),
            (
                Event::UserInput {
                    questions: vec![Question {
                        id: "choice".into(),
                        question: "Which file?".into(),
                        options: vec!["A".into()],
                        multi_select: false,
                    }],
                },
                "send-answers",
            ),
        ] {
            view.update(cx, |s, _| s.conversations[0].pending = Some(Pending::new(42, 9, event)));
            window.render_frame(cx);
            assert!(
                window.find(action).visible(),
                "Approvals and questions remain accessible with the work panel collapsed"
            );
        }
        view.update(cx, |s, _| s.conversations[0].pending = None);
        window.render_frame(cx);
        window.click("work-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").visible());
        assert_eq!(view.read(cx).conversations[0].timeline.len(), 3);
        window.click("reference-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").visible());
        assert!(window.find("reference-panel").visible());
        window.click("work-toggle", cx);
        window.render_frame(cx);
        assert!(window.try_find("work-panel").is_none());
        assert!(window.find("reference-panel").visible());
        window.click("work-toggle", cx);
        window.click("close-reference", cx);
        window.render_frame(cx);
        assert!(
            window.find("work-panel").visible(),
            "Activity can be reopened after viewing a reference conversation"
        );
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.sidebar_open = false);
        window.render_frame(cx);
        assert!(window.find("work-panel").bounds().top() >= window.find("chat-view").bounds().bottom());
        assert!(window.find("send").visible());
        window.click("work-panel-close", cx);
        window.render_frame(cx);
        assert!(window.find("dismiss-error").visible());
        window.click("new-chat", cx);
        window.render_frame(cx);
        assert!(window.try_find("work-toggle").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_recorded_tool_activity_when_details_open_then_progress_and_terminal_input_are_visible(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].messages = vec![SavedMessage { skills: vec![], guidance: None, id:Some(2),user:false,text:"Done".into(),attachments:vec![],incomplete:false}];
            s.conversations[0].timeline = serde_json::from_value(serde_json::json!([
                {"message_id":2,"event":{"type":"tool","call":{"id":"mcp","title":"lookup","status":"in_progress","input":null,"output":null,"output_delta":null,"activity":[{"type":"progress","message":"자료 검색 중"}]}}},
                {"message_id":2,"event":{"type":"tool","call":{"id":"mcp","title":null,"status":"completed","input":null,"output":"Final result","output_delta":null,"activity":[{"type":"progress","message":"자료 검증 중"}]}}},
                {"message_id":2,"event":{"type":"tool","call":{"id":"terminal","title":"fixture-terminal","status":"completed","input":null,"output":"actual stdout","output_delta":null,"activity":[{"type":"terminal_input","process_id":"process-42","stdin":"계속\nnext line\n"}]}}}
            ])).unwrap();
        });
        window.render_frame(cx);
        window.click("chat-None-0-0-2-mcp-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("chat-None-0-0-2-mcp-activity-0").visible());
        assert!(window.find("chat-None-0-0-2-mcp-activity-1").visible());
        assert!(window.try_find("chat-None-0-0-2-mcp-activity-2").is_none(), "The first activity must not be repeated during projection");
        window.click("chat-None-0-0-2-mcp-toggle", cx);
        window.click("chat-None-0-0-2-terminal-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("chat-None-0-0-2-terminal-activity-0").visible());
    }).unwrap();
}

#[gpui_kit::test]
fn message_bubbles_and_folded_tools_keep_their_content_and_copy_action(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.conversations[0].messages = vec![
                SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(1),
                    user: true,
                    text: "User request".into(),
                    attachments: vec![],
                    incomplete: false,
                },
                SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(2),
                    user: false,
                    text: "Done".into(),
                    attachments: vec![],
                    incomplete: false,
                },
            ];
            s.conversations[0].timeline = vec![TimelineEntry {
                message_id: 2,
                event: Event::Tool {
                    call: ToolCall {
                        id: "tool-one".into(),
                        title: Some("Read file".into()),
                        status: Some("completed".into()),
                        input: Some(serde_json::json!({"path":"fixture.rs"})),
                        output: Some(serde_json::json!("File contents")),
                        output_delta: None,
                        activity: vec![],
                    },
                },
            }];
            cx.notify();
        });
        window.render_frame(cx);
        let user = window.find("chat-None-0-0-bubble").bounds();
        let assistant = window.find("chat-None-0-1-bubble").bounds();
        assert!(user.size.width <= assistant.size.width * 0.8 + px(1.));
        assert!(window.try_find("chat-None-0-1-2-tool-one-details").is_none());
        window.click("chat-None-0-1-2-tool-one-toggle", cx);
        assert!(window.find("chat-None-0-1-2-tool-one-details").visible());
        window.click("chat-None-0-1-2-tool-one-toggle", cx);
        assert!(window.try_find("chat-None-0-1-2-tool-one-details").is_none());
        assert!(matches!(&view.read(cx).conversations[0].timeline[0].event, Event::Tool { call } if call.output == Some(serde_json::json!("File contents"))));
        window.hover("chat-None-0-0-bubble", cx);
        window.click("chat-None-0-0-copy", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("User request"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_search_wraps_keyboard_selection_and_ignores_ime_confirmation(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-toggle", cx);
        window.scroll("popup-menu", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.within("popup-menu").click(8usize, cx);
        assert!(window.try_find("settings-search-clear").is_none());
        let state = match &view.read(cx).inputs["settings-search"] {
            | Field::Line {
                state, ..
            } => state.clone(),
            | _ => panic!("settings search"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.input("추가", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let matches = view.read(cx).matching_settings();
        assert!(matches.len() > 1);
        window.press("up", cx);
        assert_eq!(view.read(cx).settings_result, matches.len() - 1);
        window.press("down", cx);
        assert_eq!(view.read(cx).settings_result, 0);
        window.press("down", cx);
        assert_eq!(view.read(cx).settings_result, 1);
        let selected_page = view.read(cx).panel;
        let state = match &view.read(cx).inputs["settings-search"] {
            | Field::Line {
                state, ..
            } => state.clone(),
            | _ => panic!("settings search"),
        };
        state.update(cx, |s, cx| s.replace_and_mark_text_in_range(None, "한", None, window, cx));
        window.press("enter", cx);
        assert_eq!(view.read(cx).panel, selected_page);
        state.update(cx, |s, cx| s.unmark_text(window, cx));
        view.update(cx, |s, _| {
            s.settings_search = "추가".into();
            s.settings_result = 1;
        });
        window.render_frame(cx);
        window.press("enter", cx);
        assert_eq!(view.read(cx).panel, Some(matches[1]));
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_drag_clamps_width_and_keeps_the_composer_accessible(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let preferences = std::env::temp_dir().join(format!("gpui-sidebar-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let width_file = preferences.join("ui-sidebar-width");
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.language_path = Some(preferences.join("ui-language")));
        window.render_frame(cx);
        let origin = window.find("sidebar-resize").bounds().center();
        window.drag(origin, gpui::point(px(350.), origin.y), cx);
        assert_eq!(window.find("app-sidebar").bounds().size.width, px(350.));
        let origin = window.find("sidebar-resize").bounds().center();
        window.drag(origin, gpui::point(px(900.), origin.y), cx);
        assert_eq!(view.read(cx).sidebar_width_for(1100.), 460.);
        assert_eq!(window.find("app-sidebar").bounds().size.width, px(460.));
        assert!(window.find("send").visible());
        let origin = window.find("sidebar-resize").bounds().center();
        window.drag(origin, gpui::point(px(100.), origin.y), cx);
        assert_eq!(window.find("app-sidebar").bounds().size.width, px(208.));
        assert_eq!(std::fs::read_to_string(&width_file).unwrap(), "208");
        let restored = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(restored.sidebar_width, 208.);
        std::fs::write(&width_file, "NaN").unwrap();
        let restored = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(restored.sidebar_width, 256.);
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar-resize").is_none());
        assert_eq!(window.find("app-sidebar").bounds().size.width, px(500.));
    })
    .unwrap();
    std::fs::remove_file(width_file).unwrap();
    std::fs::remove_dir(preferences).unwrap();
}

#[gpui_kit::test]
fn given_activity_and_reference_when_panels_are_toggled_then_they_stack_and_close_independently(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].input = "Keep this draft".into();
            s.conversations[0].timeline = serde_json::from_value(serde_json::json!([
                {"message_id":2,"event":{"type":"tool","call":{"id":"check","title":"cargo test","status":"completed"}}}
            ]))
            .unwrap();
        });
        window.render_frame(cx);
        window.click("reference-toggle", cx);
        window.render_frame(cx);
        let work = window.find("work-panel").bounds();
        let reference = window.find("reference-panel").bounds();
        assert_eq!(work.left(), reference.left());
        assert_eq!(work.size.width, reference.size.width);
        assert!(work.bottom() <= reference.top());
        assert!(window.find("send").visible());
        window.click("work-panel-close", cx);
        window.render_frame(cx);
        assert!(window.try_find("work-panel").is_none());
        assert!(window.find("reference-panel").visible());
        window.click("work-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").visible());
        assert!(window.find("reference-panel").visible());
        window.click("close-reference", cx);
        window.render_frame(cx);
        assert!(window.try_find("reference-panel").is_none());
        assert!(window.find("work-panel").visible());
        window.click("reference-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").visible());
        assert!(window.find("reference-panel").visible());
        assert_eq!(view.read(cx).conversations[0].input, "Keep this draft");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_stacked_panels_when_resized_and_reopened_then_sizes_and_visibility_survive_restart(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let preferences = std::env::temp_dir().join(format!("gpui-panels-{}", gpui_agent_infrastructure::new_token().unwrap()));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.language_path = Some(preferences.join("ui-language"));
            s.conversations[0].timeline = serde_json::from_value(serde_json::json!([
                {"message_id":2,"event":{"type":"tool","call":{"id":"check","title":"cargo test","status":"completed"}}}
            ]))
            .unwrap();
        });
        window.render_frame(cx);
        window.click("reference-toggle", cx);
        window.render_frame(cx);
        let dock = window.find("right-panels").bounds();
        let origin = window.find("right-panel-split").bounds().center();
        window.drag(origin, gpui::point(origin.x, dock.top() + dock.size.height * 0.65), cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").bounds().size.height > window.find("reference-panel").bounds().size.height);
        let ratio = view.read(cx).right_panels.work_ratio;
        assert!((0.6 .. 0.7).contains(&ratio));
        let origin = window.find("reference-resize").bounds().center();
        window.drag(origin, gpui::point(px(750.), origin.y), cx);
        window.render_frame(cx);
        assert_eq!(window.find("work-panel").bounds().size.width, px(350.));
        assert_eq!(window.find("reference-panel").bounds().size.width, px(350.));
        let resized = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(
            resized.right_panels,
            view.read(cx).right_panels,
            "Drag release must persist without closing a panel"
        );
        window.click("close-reference", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).right_panels.work_ratio, ratio);
        let restored = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(restored.right_panels, view.read(cx).right_panels);
        assert!(restored.right_panels.work_open);
        assert!(!restored.right_panels.reference_open);
        window.click("reference-toggle", cx);
        window.render_frame(cx);
        assert!(window.find("work-panel").bounds().size.height > window.find("reference-panel").bounds().size.height);
        window.click("work-panel-close", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("reference-panel").bounds().size.height,
            window.find("right-panels").bounds().size.height
        );
        let restored = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(restored.right_panels, view.read(cx).right_panels);
        assert!(!restored.right_panels.work_open);
        assert!(restored.right_panels.reference_open);
    })
    .unwrap();
    std::fs::remove_file(preferences.join("ui-right-panels.json")).unwrap();
    std::fs::remove_dir(preferences).unwrap();
}

#[gpui_kit::test]
fn reference_drag_preserves_minimum_widths_and_restores_the_saved_preference(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let preferences = std::env::temp_dir().join(format!("gpui-reference-{}", gpui_agent_infrastructure::new_token().unwrap()));
    let width_file = preferences.join("ui-right-panels.json");
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.language_path = Some(preferences.join("ui-language"));
            s.right_panels.reference_open = true;
            s.conversations[0].input = "Unsent draft".into();
        });
        window.render_frame(cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(320.));
        let origin = window.find("reference-resize").bounds().center();
        window.drag(origin, gpui::point(px(1000.), origin.y), cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(280.));
        let origin = window.find("reference-resize").bounds().center();
        window.drag(origin, gpui::point(px(400.), origin.y), cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(364.));
        assert_eq!(window.find("chat-view").bounds().size.width, px(480.));
        assert!(window.find("send").visible());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&width_file).unwrap()).unwrap()["width"],
            364.
        );
        let restored = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(restored.right_panels.width, 364.);
        std::fs::write(&width_file, "NaN").unwrap();
        let restored = ChatApp::new(
            view.read(cx).client.clone(),
            "/fixture".into(),
            Some(preferences.join("ui-language")),
            &view.read(cx).wake,
            view.read(cx).focus.clone(),
        );
        assert_eq!(restored.right_panels.width, 320.);
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(1500.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let origin = window.find("reference-resize").bounds().center();
        window.drag(origin, gpui::point(px(900.), origin.y), cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(600.));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&width_file).unwrap()).unwrap()["width"],
            600.
        );
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(1100.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(364.));
        assert_eq!(window.find("chat-view").bounds().size.width, px(480.));
        window.click("close-reference", cx);
        window.click("reference-toggle", cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(364.));
        assert_eq!(view.read(cx).right_panels.width, 600.);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&width_file).unwrap()).unwrap()["width"],
            600.
        );
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(500.));
        assert!(window.try_find("reference-resize").is_none());
        assert!(window.try_find("chat-view").is_none());
        assert!(window.find("close-reference").visible());
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(1500.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(600.));
        view.update(cx, |s, _| s.sidebar_width = 600.);
        window.render_frame(cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(420.));
        assert_eq!(window.find("chat-view").bounds().size.width, px(480.));
        view.update(cx, |s, _| s.sidebar_open = false);
        window.render_frame(cx);
        assert_eq!(window.find("reference-panel").bounds().size.width, px(600.));
        assert_eq!(window.find("chat-view").bounds().size.width, px(900.));
        assert_eq!(view.read(cx).conversations[0].input, "Unsent draft");
    })
    .unwrap();
    std::fs::remove_file(width_file).unwrap();
    std::fs::remove_dir(preferences).unwrap();
}

#[gpui_kit::test]
fn live_search_keeps_the_latest_input_when_an_older_request_finishes(cx: &mut TestAppContext) {
    use gpui_agent_presentation::Update;
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.client.select_workspace(Some(42));
            s.conversations[0].id = Some(7);
            s.conversations[0].title = Some("Existing thread".into());
        });
        window.render_frame(cx);
        assert!(window.try_find("clear-search").is_none());
        let state = match &view.read(cx).inputs["search-0"] {
            | Field::Line {
                state, ..
            } => state.clone(),
            | _ => panic!("conversation search"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.input("latest", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _window, cx| {
        view.update(cx, |s, _| {
            assert!(s.search_changed_at.is_some());
            s.tx.send((
                0,
                Update::History {
                    query: "old".into(),
                    conversations: vec![],
                },
            ))
            .unwrap();
            s.updates(&s.wake.clone());
            assert_eq!(s.conversations[0].search, "latest");
            assert_ne!(s.conversations[0].query, "old");
            assert!(s.conversations.iter().all(|c| c.search == "latest"));
            s.search_changed_at = Some(std::time::Instant::now() - std::time::Duration::from_millis(250));
            s.busy = true;
            s.poll();
            assert!(s.search_changed_at.is_some(), "Search must wait for the active operation");
            s.busy = false;
            s.poll();
            assert!(s.search_changed_at.is_none());
            assert!(s.busy, "Search should start without Enter");
        });
    })
    .unwrap();
    for _ in PROVIDERS {
        let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(request.contains("q=latest"), "{request}");
        assert!(request.contains("x-knd-workspace: 42"), "{request}");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while cx.update(|cx| {
        view.update(cx, |s, _| {
            s.updates(&s.wake.clone());
            s.busy
        })
    }) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(view.read(cx).conversations.iter().all(|c| c.query == "latest"));
        assert_eq!(view.read(cx).conversations[0].title.as_deref(), Some("Existing thread"));
        assert!(
            view.read(cx).conversations[0].view_query.is_empty(),
            "Sidebar search must not change the open message rendering"
        );
        window.render_frame(cx);
        window.click("clear-search", cx);
        assert!(view.read(cx).conversations.iter().all(|c| c.search.is_empty()));
        assert!(view.read(cx).search_changed_at.is_none());
        assert!(view.read(cx).busy);
        view.update(cx, |s, _| {
            s.conversations[0].new_chat();
            assert!(s.conversations[0].title.is_none());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_mcp_json_when_editing_then_invalid_source_is_reported_before_save(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.open_panel(Panel::Mcp);
            s.mcp.loaded_servers = true;
            s.mcp.json = "[invalid".into();
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.find("mcp-json-error").visible());
        assert_eq!(window.find("mcp-json-error").role(), Some(gpui::Role::Alert));
        assert_eq!(window.find("mcp-save").disabled(), Some(true));
        view.update(cx, |s, _| s.mcp.json = "[]".into());
        window.render_frame(cx);
        assert!(window.try_find("mcp-json-error").is_none());
        assert_eq!(window.find("mcp-save").disabled(), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_git_patch_when_reviewing_then_native_diff_rows_show_both_versions(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1400.), px(1800.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.workspaces.apply(project_catalog());
            let mut overview = project_git::overview();
            overview.proposal = None;
            s.workspaces.git.apply(overview, None);
            s.open_panel(Panel::Git);
            s.workspaces.diff = Some(FileDiff {
                workspace_id: 42,
                path: "test.rs".into(),
                unstaged: "diff --git a/test.rs b/test.rs\n--- a/test.rs\n+++ b/test.rs\n@@ -1 +1 @@\n-old_value\n+new_value\n".into(),
                staged: String::new(),
                untracked: false,
                truncated: false,
            });
        });
        window.render_frame(cx);
        assert!(window.within("git-file-unstaged").within("old-gutter").find(("line", 0usize)).visible());
        assert!(window.find("git-file-staged-empty").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_settings_items_when_searching_then_matching_controls_replace_unrelated_content(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.open_panel(Panel::General));
        window.render_frame(cx);
        let Field::Line {
            state, ..
        } = &view.read(cx).inputs["settings-search"]
        else {
            panic!("search input")
        };
        state.clone().update(cx, |s, cx| s.focus(window, cx));
        window.input("Enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-nav-General").visible());
        assert!(window.try_find("settings-language-English").is_none_or(|item| !item.visible()));
        window.click("settings-search-clear", cx);
        window.render_frame(cx);
        assert!(window.find("settings-language-English").visible());
        let Field::Line {
            state, ..
        } = &view.read(cx).inputs["settings-search"]
        else {
            panic!("search input")
        };
        state.clone().update(cx, |s, cx| s.focus(window, cx));
        window.input("no-such-setting", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-no-results").visible());
        assert!(window.try_find("settings-language-English").is_none());
        window.click("settings-search-clear", cx);
        window.render_frame(cx);
        assert!(window.find("settings-language-English").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_search_and_sidebar_open_the_mcp_page_without_losing_drafts(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].input = "Keep the chat draft".into();
            s.conversations[0].session_id = Some("keep-session".into());
            s.mcp.json = "[{\"name\":\"unfinished\"".into();
            s.mcp.loaded_servers = true;
        });
        window.render_frame(cx);
        window.click("settings-toggle", cx);
        window.scroll("popup-menu", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.within("popup-menu").click(8usize, cx);
        assert!(window.find("settings-sidebar").visible());
        assert!(window.find("settings-header").visible());
        assert!(window.try_find("chat-header").is_none());
        assert!(window.find("settings-content").bounds().size.width <= px(896.));
        let search = match &view.read(cx).inputs["settings-search"] {
            | Field::Line {
                state, ..
            } => state.clone(),
            | _ => panic!("settings search"),
        };
        search.update(cx, |s, cx| s.focus(window, cx));
        window.input("MCP", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("settings-nav-Plugins").visible());
        assert!(window.try_find("settings-nav-General").is_none());
        window.press("enter", cx);
        assert_eq!(view.read(cx).panel, Some(Panel::Plugins));
        if window.try_find("plugin-category-2").is_some() {
            window.click("plugin-category-2", cx);
        } else {
            window.within("plugin-tabs").click(2usize, cx);
        }
        // Given unsaved editors, When navigating through search and the
        // sidebar, Then each page has its own title and contents, no
        // duplicate tabs, and retains both drafts.
        assert!(window.try_find("tool-tab-Mcp").is_none());
        assert_eq!(Panel::Mcp.title(), "MCP 서버");
        assert!(view.read(cx).settings_search.is_empty());
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.find("mcp-load").visible());
        assert!(window.find("mcp-save").visible());
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(10000.))), cx);
        window.click("settings-nav-General", cx);
        assert_eq!(view.read(cx).panel, Some(Panel::General));
        assert!(window.try_find("tool-tab-Mcp").is_none());
        assert!(window.try_find("mcp-load").is_none());
        assert!(window.try_find("settings-nav-Automation").is_none());
        window.click("settings-nav-Plugins", cx);
        if window.try_find("plugin-category-2").is_some() {
            window.click("plugin-category-2", cx);
        } else {
            window.within("plugin-tabs").click(2usize, cx);
        }
        assert_eq!(view.read(cx).mcp.json, "[{\"name\":\"unfinished\"");
        window.click("panel-close", cx);
        assert_eq!(view.read(cx).conversations[0].input, "Keep the chat draft");
        assert_eq!(view.read(cx).conversations[0].session_id.as_deref(), Some("keep-session"));
        assert!(window.find("chat-header").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_saved_oauth_servers_when_mcp_settings_open_then_login_controls_require_a_saved_profile_and_keep_chat_drafts(cx: &mut TestAppContext) {
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.mcp.loaded = true;
            s.conversations[0].input = "Keep OAuth chat draft".into();
            s.mcp.json = serde_json::json!([{"name":"remote","url":"https://example.invalid/mcp","oauth":{}}]).to_string();
            s.mcp.loaded_servers = true;
            s.mcp.servers = serde_json::from_str(&s.mcp.json).unwrap();
            s.open_panel(Panel::Mcp);
        });
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.find("mcp-oauth-login-remote").visible());
        assert!(window.try_find("mcp-oauth-cancel-remote").is_none());
        window.click("mcp-oauth-login-remote", cx);
    })
    .unwrap();
    let wait = |cx: &mut TestAppContext, state: McpOAuthState| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let ready = cx.update(|cx| {
                view.update(cx, |s, _| {
                    s.poll();
                    s.mcp.oauth.get("remote").is_some_and(|status| status.state == state)
                })
            });
            if ready {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "OAuth UI did not reach expected state");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    wait(cx, McpOAuthState::Pending);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("mcp-oauth-cancel-remote").visible());
        assert!(window.find("mcp-oauth-browser-remote").visible());
        assert!(window.try_find("mcp-oauth-login-remote").is_none());
        window.click("mcp-oauth-cancel-remote", cx);
    })
    .unwrap();
    wait(cx, McpOAuthState::Cancelled);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("mcp-oauth-login-remote", cx);
    })
    .unwrap();
    wait(cx, McpOAuthState::Connected);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("mcp-oauth-logout-remote").visible());
        window.click("mcp-oauth-logout-remote", cx);
    })
    .unwrap();
    wait(cx, McpOAuthState::SignedOut);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        window.within("mcp-provider-group").click(1usize, cx);
        window.scroll("panel-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))), cx);
        assert!(window.find("mcp-oauth-login-remote").visible());
        assert_eq!(view.read(cx).mcp.servers.len(), 1);
        assert!(view.read(cx).mcp.loaded_servers);
        assert!(view.read(cx).mcp.oauth.is_empty());
        assert_eq!(view.read(cx).conversations[0].input, "Keep OAuth chat draft");
    })
    .unwrap();
    let requests: Vec<_> = api.requests.try_iter().collect();
    for command in ["login", "cancel", "logout"] {
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("post /v1/mcp/claude/remote/oauth ") && request.contains(&format!("\"{command}\"")))
        );
    }
}

#[gpui_kit::test]
fn given_general_settings_when_selecting_language_then_the_group_has_one_selected_value(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.panel = Some(Panel::General));
        window.render_frame(cx);
        assert!(window.find("settings-language-group").visible());
        window.click("settings-language-Korean", cx);
        assert_eq!(view.read(cx).language, Language::Korean);
        window.click("settings-language-English", cx);
        assert_eq!(view.read(cx).language, Language::English);
        window.click("settings-language-English", cx);
        assert_eq!(view.read(cx).language, Language::English);
    })
    .unwrap();
}

struct AccountApi {
    client: ApiClient,
    requests: std::sync::mpsc::Receiver<String>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    history_mode: std::sync::Arc<std::sync::atomic::AtomicU8>,
}

#[gpui_kit::test]
fn startup_shows_each_projects_history_once_and_opens_it_in_the_main_chat(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.start(gpui_agent_presentation::Action::Refresh, &s.wake.clone());
        })
    });
    // Given a restarted app and two projects with persisted chats and linked
    // jobs.
    wait_for_chat(&view, cx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let ready = cx
            .update_window(handle.into(), |_, window, cx| {
                view.update(cx, |s, _| s.poll());
                window.render_frame(cx);
                window.try_find("project-history-43-claude-default-chat-17").is_some()
            })
            .unwrap();
        if ready || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    cx.update_window(handle.into(), |_, window, cx| {
        // Then both projects have records without opening settings or the
        // queue.
        assert!(window.find("project-history-42-claude-default-chat-7").visible());
        assert!(window.find("project-history-43-claude-default-chat-17").visible());
        assert!(window.try_find("project-history-42-claude-default-job-107").is_none());
        assert!(window.try_find("settings-sidebar").is_none());
        view.update(cx, |s, _| s.conversations[0].input = "Keep my startup draft".into());
        // When a record in the other project is selected, it opens in the main
        // chat.
        window.click("project-history-43-claude-default-chat-17", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let s = view.read(cx);
        assert_eq!(s.workspaces.selected, Some(43));
        assert_eq!(s.conversations[0].id, Some(17));
        assert_eq!(s.conversations[0].messages[1].text, "Previous answer");
        assert_eq!(s.workspaces.cached[&Some(42)][0].input, "Keep my startup draft");
        assert!(s.panel.is_none());
        assert!(window.find("chat-view").visible());
        assert!(!s.can_send(), "Empty follow-up should not send");
        view.update(cx, |s, _| s.conversations[0].input = "Follow up".into());
        window.render_frame(cx);
        assert!(view.read(cx).can_send());
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).conversations[0].messages.last().unwrap().text,
            "Follow-up answer",
            "{:?}",
            view.read(cx).conversations[0].error
        )
    });
    let requests: Vec<_> = api.requests.try_iter().collect();
    let request = requests
        .iter()
        .find(|request| request.starts_with("post /v1/providers/claude/jobs ") && request.contains("Follow up"))
        .unwrap();
    assert!(request.contains("x-knd-workspace: 43"));
    assert!(request.contains(r#""conversation_id":17"#) && request.contains(r#""session_id":"saved-session""#));
}

fn wait_for_sidebar_element(handle: WindowHandle<gpui::base::Root>, view: &Entity<ChatApp>, id: &str, cx: &mut TestAppContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let ready = cx
            .update_window(handle.into(), |_, window, cx| {
                view.update(cx, |s, _| s.poll());
                window.render_frame(cx);
                window.try_find(id.to_owned()).is_some()
            })
            .unwrap();
        if ready {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "Missing sidebar element {id}");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[gpui_kit::test]
fn restarted_job_replays_progress_and_approval_in_the_main_chat(cx: &mut TestAppContext) {
    // Given a job left running by an earlier app process.
    let api = AccountApi::new();
    let _: Job = api
        .client
        .response(
            reqwest::Method::POST,
            "/v1/providers/claude/jobs",
            Some(serde_json::json!({
                "text":"Analyze this project","session_id":null,"conversation_id":null,"model":null
            })),
        )
        .unwrap()
        .json()
        .unwrap();
    let (handle, view) = background_fixture(cx, &api);
    cx.update(|cx| view.update(cx, |s, _| s.command(gpui_agent_presentation::Action::Projects)));
    wait_for_sidebar_element(handle, &view, "project-history-42-claude-default-job-5", cx);
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        // When its sidebar record is opened, approval stays in the
        // conversation.
        window.render_frame(cx);
        window.click("project-history-42-claude-default-job-5", cx);
    })
    .unwrap();
    wait_for_chat_state(&view, cx, |s| s.conversations[0].pending.is_some() || !s.busy);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).conversations[0].pending.is_some(), "{:?}", view.read(cx).conversations[0].error);
        assert!(window.find("chat-view").visible());
        assert!(view.read(cx).panel.is_none());
        assert!(window.find("allow-once").visible());
        window.click("allow-once", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let c = &view.read(cx).conversations[0];
        assert_eq!(c.messages.len(), 2);
        assert_eq!(c.messages[0].text, "Analyze this project");
        assert_eq!(c.messages[1].text, "Project analysis");
        assert!(c.pending.is_none());
    });
}

#[gpui_kit::test]
fn project_history_distinguishes_loading_failure_and_empty_and_can_retry(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::Relaxed;
    let api = AccountApi::with_project_history(true);
    api.history_mode.store(1, Relaxed);
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.command(gpui_agent_presentation::Action::Refresh);
        })
    });
    // Given a slow history service, the UI must not claim that there are no
    // records.
    wait_for_sidebar_element(handle, &view, "history-loading-42", cx);
    cx.update_window(handle.into(), |_, window, _| {
        assert!(window.try_find("history-empty-42").is_none());
        assert!(window.try_find("history-error-42").is_none());
    })
    .unwrap();
    // When history fails, show an error and retry, including for the inactive
    // project.
    api.history_mode.store(2, Relaxed);
    wait_for_sidebar_element(handle, &view, "history-error-43", cx);
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("history-empty-42").is_none());
        assert!(window.try_find("history-loading-42").is_none());
        assert!(window.find("history-retry-42").visible());
        api.history_mode.store(3, Relaxed);
        window.click("history-retry-42", cx);
    })
    .unwrap();
    // Then a successful empty response is the only case that displays 'no
    // records'.
    wait_for_sidebar_element(handle, &view, "history-empty-43", cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("history-empty-42").visible());
        assert!(window.try_find("history-error-42").is_none());
        assert!(window.try_find("history-loading-42").is_none());
        assert!(view.read(cx).panel.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn archiving_a_chat_updates_the_project_list_without_resurrecting_its_job(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.command(gpui_agent_presentation::Action::Refresh);
        })
    });
    wait_for_sidebar_element(handle, &view, "project-history-42-claude-default-chat-7", cx);
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("history-menu-7", cx);
        window.within("popup-menu").click(2usize, cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    wait_for_sidebar_element(handle, &view, "history-empty-42", cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("project-history-42-claude-default-chat-7").is_none());
        assert!(window.try_find("project-history-42-claude-default-job-107").is_none());
        window.click("archived", cx);
        assert!(window.find("project-history-42-claude-default-chat-7").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn equal_conversation_ids_in_different_accounts_keep_their_scope_and_drafts(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    api.history_mode.store(4, std::sync::atomic::Ordering::Relaxed);
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.conversations[0].input = "Default account draft".into();
            s.command(gpui_agent_presentation::Action::Refresh);
        })
    });
    let work = "project-history-42-claude-11111111111111111111111111111111-chat-7";
    wait_for_sidebar_element(handle, &view, work, cx);
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("project-history-42-claude-default-chat-7").visible());
        window.click(work, cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].input = "Scoped follow up".into());
        window.render_frame(cx);
        window.click("send", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    let requests: Vec<_> = api.requests.try_iter().collect();
    let followup = requests
        .iter()
        .find(|request| request.starts_with("post /v1/providers/claude/jobs ") && request.contains("Scoped follow up"))
        .unwrap();
    assert!(followup.contains("x-knd-account: 11111111111111111111111111111111") && followup.contains("x-knd-workspace: 42"));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.conversations[0].input = "Work account draft".into());
        window.render_frame(cx);
        window.click("project-history-42-claude-default-chat-7", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).conversations[0].drafts.iter().any(|draft| draft.input == "Default account draft"));
        window.click(work, cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| assert_eq!(view.read(cx).conversations[0].input, "Work account draft"));
}

#[test]
fn sidebar_search_reads_all_providers_in_the_selected_workspace_and_load_refreshes_auth() {
    use gpui_agent_presentation::{Action,
                                  Update};
    let api = AccountApi::new();
    let mut client = api.client.clone();
    client.select_workspace(Some(42));
    let (tx, rx) = std::sync::mpsc::sync_channel(32);
    let (wake, _events) = async_channel::unbounded();
    client.start(Provider::Codex, Action::Search("layout 한글".into()), Default::default(), tx, Wake(wake));
    let mut seen = Vec::new();
    loop {
        let (index, update) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        match update {
            | Update::History {
                query, ..
            } => {
                assert_eq!(query, "layout 한글");
                seen.push(index);
            },
            | Update::Finished => break,
            | Update::Error(error) => panic!("{error}"),
            | _ => panic!("Unexpected search update"),
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, (0 .. PROVIDERS.len()).collect::<Vec<_>>());
    for _ in PROVIDERS {
        let request = api.requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(request.contains("x-knd-workspace: 42"), "{request}");
        assert!(request.contains("q=layout+%ed%95%9c%ea%b8%80"), "{request}");
    }
    let updates = std::cell::RefCell::new(Vec::new());
    client
        .run(
            Provider::Codex,
            Action::Load {
                id: 7,
                message_id: None,
            },
            "",
            &|update| {
                updates.borrow_mut().push(update);
                true
            },
        )
        .unwrap();
    let captured = updates.borrow();
    assert!(
        matches!(&captured[..], [Update::Loaded {conversation, ..}, Update::Versions(_), Update::Status(status), Update::Models(_)] if status.authenticated && conversation.id == 7 && conversation.session_id.as_deref() == Some("saved-session"))
    );
    drop(captured);
    updates.borrow_mut().clear();
    client.select_workspace(Some(43));
    assert!(
        client
            .run(
                Provider::Codex,
                Action::Load {
                    id: 7,
                    message_id: None
                },
                "",
                &|update| {
                    updates.borrow_mut().push(update);
                    true
                }
            )
            .is_err()
    );
    assert!(
        matches!(&updates.borrow()[..], [Update::Loaded {conversation,..}, Update::Versions(_)] if conversation.id == 7),
        "Saved messages must load even when status fails"
    );
}

#[gpui_kit::test]
fn unified_sidebar_menu_preserves_the_active_provider_and_requires_delete_confirmation(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.conversations[0].input = "Keep my draft".into();
            s.conversations[1].history = vec![serde_json::from_value(serde_json::json!({"id":7,"title":"Codex thread","updated_at":1})).unwrap()];
            cx.notify();
        });
        window.render_frame(cx);
        window.click("history-menu-7", cx);
        window.within("popup-menu").click(0usize, cx);
        assert!(matches!(
            view.read(cx).dialog,
            Some(HistoryDialog::Rename {
                provider: 1,
                id: 7,
                ..
            })
        ));
        assert_eq!(view.read(cx).selected, 0);
        window.press("escape", cx);
        assert!(view.read(cx).dialog.is_none());
        assert_eq!(view.read(cx).conversations[1].history.len(), 1);
        assert!(!view.read(cx).busy);
        window.click("history-menu-7", cx);
        window.within("popup-menu").click(5usize, cx);
        assert!(matches!(
            view.read(cx).dialog,
            Some(HistoryDialog::Delete {
                provider: 1,
                id: 7,
                ..
            })
        ));
        assert!(!view.read(cx).busy);
        window.click("history-cancel", cx);
        assert_eq!(view.read(cx).conversations[1].history.len(), 1);
        assert_eq!(view.read(cx).conversations[0].input, "Keep my draft");
    })
    .unwrap();
}

#[gpui_kit::test]
fn chat_layout_keeps_the_composer_below_the_scrolling_messages_and_can_return_from_the_narrow_sidebar(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.conversations[0].messages = vec![SavedMessage {
                skills: vec![],
                guidance: None,
                id: Some(1),
                user: false,
                text: "A long response\n".repeat(150),
                attachments: vec![],
                incomplete: false,
            }];
            s.conversations[0].input = "Keep typing".into();
            cx.notify();
        });
        window.render_frame(cx);
        let composer = window.find("composer-surface").bounds();
        let header = window.find("chat-header").bounds();
        assert_eq!(window.find("app-sidebar").bounds().size.width, px(256.));
        assert!(composer.size.width <= px(736.));
        assert!(header.bottom() <= window.find("messages-scroll").bounds().top());
        assert!(window.find("messages-scroll").bounds().bottom() <= composer.top());
        assert!(window.find("send").bounds().bottom() <= composer.bottom());
        window.scroll("messages-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-500.))), cx);
        assert_eq!(window.find("composer-surface").bounds(), composer);
    })
    .unwrap();
    cx.simulate_window_resize(handle.into(), size(px(500.), px(360.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("app-sidebar").bounds().size.width, px(500.));
        window.click("sidebar-toggle", cx);
        assert!(window.find("send").visible());
        assert_eq!(view.read(cx).conversations[0].input, "Keep typing");
        window.click("sidebar-toggle", cx);
        assert!(window.find("new-chat").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn korean_syllable_commit_does_not_overwrite_the_next_composition(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (handle, view) = fixture(cx);
    let (state, next_syllable) = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let state = match &view.read(cx).inputs["composer-None-0"] {
                | Field::Text {
                    state, ..
                } => state.clone(),
                | _ => panic!("composer"),
            };
            // Windows starts the next preedit after the commit event, before
            // redraw.
            let next_syllable = view.update(cx, |_, cx| {
                cx.subscribe_in(&state, window, |_, state, event, window, cx| {
                    if matches!(event, gpui::component::input::InputEvent::Change) {
                        state.update(cx, |s, cx| s.replace_and_mark_text_in_range(None, "ㄱ", Some(1 .. 1), window, cx));
                    }
                })
            });
            state.update(cx, |s, cx| {
                s.focus(window, cx);
                for text in ["ㅎ", "하", "한"] {
                    s.replace_and_mark_text_in_range(None, text, Some(1 .. 1), window, cx);
                }
                s.replace_text_in_range(None, "한", window, cx);
            });
            (state, next_syllable)
        })
        .unwrap();
    drop(next_syllable);
    cx.update_window(handle.into(), |_, window, cx| {
        assert_eq!(view.read(cx).conversations[0].input, "한");
        window.render_frame(cx);
        state.update(cx, |s, cx| {
            assert_eq!(s.value().as_ref(), "한ㄱ");
            assert_eq!(s.marked_text_range(window, cx), Some(1 .. 2));
            s.replace_and_mark_text_in_range(None, "그", Some(1 .. 1), window, cx);
            s.replace_and_mark_text_in_range(None, "글", Some(1 .. 1), window, cx);
            s.replace_text_in_range(None, "글", window, cx);
            assert_eq!(s.value().as_ref(), "한글");
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn composer_keeps_shift_enter_and_ime_composition_while_enter_sends(cx: &mut TestAppContext) {
    use gpui::{EntityInputHandler,
               Focusable};
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = match &view.read(cx).inputs["composer-None-0"] {
            | Field::Text {
                state, ..
            } => state.clone(),
            | _ => panic!("composer"),
        };
        state.update(cx, |s, cx| s.focus(window, cx));
        window.press("shift-enter", cx);
        assert!(state.read(cx).value().contains('\n'));
        assert!(!view.read(cx).busy);
        state.update(cx, |s, cx| s.replace_and_mark_text_in_range(None, "한", None, window, cx));
        view.update(cx, |s, cx| s.conversations[0].input = state.read(cx).value().to_string());
        window.render_frame(cx);
        window.press("enter", cx);
        assert!(!view.read(cx).busy, "IME confirmation must not send a message");
        assert!(view.read(cx).conversations[0].messages.is_empty());
        state.update(cx, |s, cx| {
            s.unmark_text(window, cx);
            s.set_value("Send this", window, cx);
        });
        view.update(cx, |s, _| s.conversations[0].input = "Send this".into());
        window.render_frame(cx);
        assert!(state.focus_handle(cx).is_focused(window), "Composer lost focus");
        assert!(view.read(cx).can_send(), "Composer cannot send: {}", view.read(cx).conversations[0].input);
        state.update(cx, |s, cx| assert!(s.marked_text_range(window, cx).is_none()));
        window.press("enter", cx);
        assert_eq!(view.read(cx).conversations[0].messages[0].text, "Send this");
        assert!(view.read(cx).conversations[0].input.is_empty());
        view.update(cx, |s, _| {
            s.busy = false;
            s.conversations[0].new_chat();
            s.conversations[0].input = "Shortcut".into();
        });
        state.update(cx, |s, cx| {
            s.set_value("Shortcut", window, cx);
            s.focus(window, cx);
        });
        window.press("secondary-enter", cx);
        assert_eq!(view.read(cx).conversations[0].messages[0].text, "Shortcut");
    })
    .unwrap();
}

impl AccountApi {
    fn new() -> Self { Self::with_project_history(false) }

    fn with_project_history(project_history: bool) -> Self {
        use std::io::{Read,
                      Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = ApiClient::new(format!("http://{}", listener.local_addr().unwrap()), "fixture".into()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let history_mode = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
        let mode = history_mode.clone();
        let (tx, requests) = std::sync::mpsc::channel();
        let computer = std::sync::Arc::new(std::sync::Mutex::new(ComputerStatus {
            available: true,
            ..Default::default()
        }));
        let computer_state = computer.clone();
        let thread = std::thread::spawn(move || {
            let mut next_job = 5;
            let mut detail: Option<JobDetail> = None;
            let mut journal: Vec<JobEvent> = Vec::new();
            let mut saved_chats = std::collections::HashMap::<i64, serde_json::Value>::new();
            let mut archived_chats = std::collections::HashSet::<i64>::new();
            let mut deleted_chats = std::collections::HashSet::<i64>::new();
            if project_history {
                for id in [7, 17] {
                    saved_chats.insert(
                        id,
                        serde_json::json!({"id":id,"model":null,"session_id":"saved-session","messages":[
                            {"id":1,"user":true,"text":"Previous question","incomplete":false},
                            {"id":2,"user":false,"text":"Previous answer","incomplete":false}
                        ]}),
                    );
                }
            }
            let mut versions = Vec::<ConversationVersion>::new();
            let mut prepared_revision: Option<PreparedRevision> = None;
            let mut next_conversation = 8;
            let mut recorded_job = None;
            let mut antigravity_installed = false;
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                let mut bytes = Vec::new();
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    bytes.push(byte[0]);
                }
                let header = String::from_utf8(bytes).unwrap().to_lowercase();
                if header.is_empty() {
                    continue;
                }
                let length: usize = header.lines().find_map(|l| l.strip_prefix("content-length: ")).unwrap_or("0").parse().unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let payload = String::from_utf8(body).unwrap();
                let request = format!("{header}{payload}");
                let missing_project = request.contains("/missing");
                tx.send(request).unwrap();
                let first = header.lines().next().unwrap();
                let path = first.split_whitespace().nth(1).unwrap();
                while path.contains("/conversations?")
                    && mode.load(std::sync::atomic::Ordering::Relaxed) == 1
                    && !stopped.load(std::sync::atomic::Ordering::Relaxed)
                {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                while path.contains("/files?") && mode.load(std::sync::atomic::Ordering::Relaxed) == 6 && !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                let provider = if path.contains("/claude/") {
                    Provider::Claude
                } else if path.contains("/codex/") {
                    Provider::Codex
                } else {
                    Provider::Antigravity
                };
                if path.ends_with("/state") && first.starts_with("patch ") {
                    let id: i64 = path.rsplit('/').nth(1).unwrap().parse().unwrap();
                    let state: ConversationState = serde_json::from_str(&payload).unwrap();
                    if let Some(archived) = state.archived {
                        if archived {
                            archived_chats.insert(id);
                        } else {
                            archived_chats.remove(&id);
                        }
                    }
                }
                if first.starts_with("delete ")
                    && let Some(id) = path.rsplit('/').next().and_then(|part| part.parse::<i64>().ok())
                {
                    deleted_chats.insert(id);
                }
                if path.ends_with("/jobs") && first.starts_with("post ") {
                    let request: ChatRequest = serde_json::from_str(&payload).unwrap();
                    let events = if request.computer_use {
                        vec![
                            StreamEvent::Started {
                                operation_id: 42,
                            },
                            StreamEvent::Conversation {
                                conversation_id: 7,
                            },
                            StreamEvent::Interaction {
                                operation_id: 42,
                                request_id: 777,
                                event: Event::Permission {
                                    method: "computer/request_control".into(),
                                    details: serde_json::json!({"job_id":next_job}),
                                },
                            },
                        ]
                    } else if request.text == "Queue this" {
                        vec![]
                    } else if request.text == "Fail this" {
                        vec![StreamEvent::Error {
                            message: "Job failed".into(),
                        }]
                    } else if let Some(conversation_id) = request.conversation_id {
                        vec![
                            StreamEvent::Started {
                                operation_id: 42,
                            },
                            StreamEvent::Conversation {
                                conversation_id,
                            },
                            StreamEvent::Completed {
                                session_id: "chat-session".into(),
                                text: if request.text == "Edited question" {
                                    "Edited answer"
                                } else {
                                    "Follow-up answer"
                                }
                                .into(),
                            },
                        ]
                    } else {
                        vec![
                            StreamEvent::Started {
                                operation_id: 42,
                            },
                            StreamEvent::Conversation {
                                conversation_id: 7,
                            },
                            StreamEvent::Agent(Event::Session {
                                id: "chat-session".into(),
                            }),
                            StreamEvent::Agent(Event::Tool {
                                call: serde_json::from_value(serde_json::json!({
                                    "id":"read", "title":"Read project", "status":"in_progress"
                                }))
                                .unwrap(),
                            }),
                            StreamEvent::Interaction {
                                operation_id: 42,
                                request_id: 777,
                                event: Event::Permission {
                                    method: "claude/can_use_tool".into(),
                                    details: serde_json::json!({"input":{"command":"read project"}}),
                                },
                            },
                        ]
                    };
                    journal = events
                        .into_iter()
                        .enumerate()
                        .map(|(index, event)| JobEvent {
                            sequence: index as i64 + 1,
                            event,
                        })
                        .collect();
                    detail = Some(JobDetail {
                        job: Job {
                            id: next_job,
                            scope: JobScope {
                                provider,
                                account_id: "default".into(),
                                workspace_id: Some(42),
                                workspace_path: "/first".into(),
                            },
                            state: JobState::Queued,
                            conversation_id: request.conversation_id,
                            operation_id: None,
                            pending_request_id: None,
                            last_sequence: journal.len() as i64,
                            preview: request.text.clone(),
                            cancel_requested: false,
                            error: None,
                            created_at: 1,
                        },
                        request,
                    });
                    next_job += 1;
                } else if path.ends_with("/revisions") && first.starts_with("post ") {
                    let source_id: i64 = path.rsplit('/').nth(1).unwrap().parse().unwrap();
                    let revision: ReviseRequest = serde_json::from_str(&payload).unwrap();
                    let source = &saved_chats[&source_id];
                    let mut conversation: SavedConversation = serde_json::from_value(source.clone()).unwrap();
                    let index = conversation.messages.iter().rposition(|message| message.user).unwrap();
                    let attachments = conversation.messages[index].attachments.clone();
                    conversation.messages.truncate(index);
                    conversation.session_id = None;
                    conversation.id = next_conversation;
                    if versions.is_empty() {
                        versions.push(ConversationVersion {
                            conversation_id: source_id,
                            question_index: index,
                        });
                    }
                    versions.push(ConversationVersion {
                        conversation_id: next_conversation,
                        question_index: index,
                    });
                    saved_chats.insert(next_conversation, serde_json::to_value(&conversation).unwrap());
                    prepared_revision = Some(PreparedRevision {
                        conversation,
                        request: ChatRequest {
                            extensions: None,
                            skills: vec![],
                            computer_use: false,
                            text: revision.text,
                            attachments,
                            model: revision.model,
                            session_id: None,
                            conversation_id: Some(next_conversation),
                            claude_permission_mode: revision.claude_permission_mode,
                            codex_execution_policy: revision.codex_execution_policy,
                        },
                        versions: versions.clone(),
                    });
                    next_conversation += 1;
                } else if path.starts_with("/v1/jobs/") && first.starts_with("post ") && path.contains("/interactions/") {
                    let response: InteractionResponse = serde_json::from_str(&payload).unwrap();
                    let computer = detail.as_ref().is_some_and(|d| d.request.computer_use);
                    let request_id: u64 = path.rsplit('/').next().unwrap().parse().unwrap();
                    let events = if computer && request_id == 777 {
                        vec![StreamEvent::Agent(Event::InteractionResolved {request_id,response}),
                            StreamEvent::Agent(Event::Tool {call:serde_json::from_value(serde_json::json!({"id":"computer-input","title":"computer_act","status":"in_progress","input":{"revision":"1","action":{"type":"text","text":"한글"}}})).unwrap()}),
                            StreamEvent::Interaction {operation_id:42,request_id:778,event:Event::Elicitation {server_name:"gpui-agent-session".into(),message:"Approve computer_act".into(),schema:Some(serde_json::json!({"type":"object","properties":{}})),url:None}}]
                    } else if computer
                        && matches!(
                            &response,
                            InteractionResponse::Elicitation {
                                action: ElicitationAction::Accept,
                                ..
                            }
                        )
                    {
                        vec![StreamEvent::Agent(Event::InteractionResolved {request_id,response}),
                            StreamEvent::Agent(Event::Tool {call:serde_json::from_value(serde_json::json!({"id":"computer-input","title":"computer_act","status":"completed","output":{"structuredContent":{"revision":"2","width":1,"height":1},"content":[{"type":"image","mimeType":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="}]}})).unwrap()}),
                            StreamEvent::Completed {session_id:"chat-session".into(),text:"한글 입력 확인".into()}]
                    } else if computer {
                        vec![StreamEvent::Agent(Event::InteractionResolved {request_id,response}),
                            StreamEvent::Agent(Event::Tool {call:serde_json::from_value(serde_json::json!({"id":"computer-input","title":"computer_act","status":"failed","output":{"isError":true,"content":[{"type":"text","text":"Computer input declined"}]}})).unwrap()}),
                            StreamEvent::Completed {session_id:"chat-session".into(),text:"Computer input declined".into()}]
                    } else {
                        vec![
                            StreamEvent::Agent(Event::InteractionResolved {
                                request_id: 777,
                                response,
                            }),
                            StreamEvent::Agent(Event::Tool {
                                call: serde_json::from_value(serde_json::json!({
                                    "id":"read", "title":"Read project", "status":"completed"
                                }))
                                .unwrap(),
                            }),
                            StreamEvent::Agent(Event::Text {
                                delta: "Project analysis".into(),
                            }),
                            StreamEvent::Completed {
                                session_id: "chat-session".into(),
                                text: "Project analysis".into(),
                            },
                        ]
                    };
                    for event in events {
                        journal.push(JobEvent {
                            sequence: journal.len() as i64 + 1,
                            event,
                        });
                    }
                } else if path.starts_with("/v1/jobs/") && first.starts_with("delete ") {
                    journal.push(JobEvent {
                        sequence: journal.len() as i64 + 1,
                        event: StreamEvent::Cancelled,
                    });
                }
                if let Some(detail) = &mut detail {
                    detail.job.last_sequence = journal.len() as i64;
                    if detail.request.computer_use {
                        match journal.last().map(|e| &e.event) {
                            | Some(StreamEvent::Interaction {
                                request_id, ..
                            }) => {
                                detail.job.state = JobState::Waiting;
                                detail.job.pending_request_id = Some(*request_id);
                            },
                            | Some(StreamEvent::Completed {
                                ..
                            }) => {
                                detail.job.state = JobState::Completed;
                                detail.job.pending_request_id = None;
                            },
                            | Some(StreamEvent::Cancelled) => {
                                detail.job.state = JobState::Cancelled;
                                detail.job.pending_request_id = None;
                            },
                            | _ => {},
                        }
                        detail.job.operation_id = Some(42);
                    }
                    if recorded_job != Some(detail.job.id)
                        && let Some(StreamEvent::Completed {
                            session_id,
                            text,
                        }) = journal.last().map(|entry| &entry.event)
                    {
                        let id = detail.request.conversation_id.unwrap_or(7);
                        let saved = saved_chats.entry(id).or_insert_with(|| {
                            serde_json::json!({
                                "id":id,"session_id":session_id,"model":null,"messages":[],"timeline":[]
                            })
                        });
                        let messages = saved["messages"].as_array_mut().unwrap();
                        let next = messages.len() + 1;
                        messages.push(
                            serde_json::json!({"id":next,"user":true,"text":detail.request.text,"attachments":detail.request.attachments,"incomplete":false}),
                        );
                        messages.push(serde_json::json!({"id":next+1,"user":false,"text":text,"attachments":[],"incomplete":false}));
                        saved["session_id"] = serde_json::json!(session_id);
                        recorded_job = Some(detail.job.id);
                    }
                }
                let status = serde_json::json!({"authenticated": !path.ends_with("/logout") && !(antigravity_installed && provider == Provider::Antigravity && !path.ends_with("/login")), "account": "fixture@example.com", "method": "fixture"});
                let value = if path == "/v1/workspaces/42/git" {
                    let mut overview = project_git::overview();
                    if payload.contains("commit_selected") {
                        overview.commit = Some("committed-selection".into());
                        overview.proposal = None;
                    }
                    serde_json::to_value(overview).unwrap()
                } else if path == "/v1/runtimes/antigravity" && mode.load(std::sync::atomic::Ordering::Relaxed) == 12 {
                    serde_json::json!({"error":"Release lookup unavailable"})
                } else if path == "/v1/runtimes/antigravity" {
                    antigravity_installed |= matches!(serde_json::from_str::<RuntimeCommand>(&payload).unwrap(), RuntimeCommand::Install { .. });
                    serde_json::json!({
                        "provider":"antigravity","active_path":antigravity_installed.then_some("/fixture/antigravity.exe"),
                        "managed_version":antigravity_installed.then_some("1.4.1"),"override_active":false,
                        "release":{"version":"1.4.1","url":"https://example.com/runtime","sha256":"ab".repeat(32),"size":null},
                        "issue":(!antigravity_installed).then_some("Runtime missing")
                    })
                } else if path == "/v1/control/computer/grant" && first.starts_with("post ") {
                    let grant: ComputerGrantRequest = serde_json::from_str(&payload).unwrap();
                    let mut state = computer_state.lock().unwrap();
                    state.job_id = grant.enabled.then_some(grant.job_id);
                    state.execution_bound = false;
                    state.expires_in_seconds = if grant.enabled { 600 } else { 0 };
                    serde_json::json!({})
                } else if path == "/v1/control/computer" {
                    if mode.load(std::sync::atomic::Ordering::Relaxed) == 11 {
                        serde_json::json!({"error":"Control state unavailable"})
                    } else {
                        serde_json::to_value(&*computer_state.lock().unwrap()).unwrap()
                    }
                } else if path == "/v1/control" {
                    serde_json::json!({"browser_available":true,"browser_running":false,"desktop_available":true,"desktop_enabled_until":0})
                } else if path.ends_with("/oauth") {
                    let command = serde_json::from_str::<McpOAuthCommand>(&payload).unwrap_or(McpOAuthCommand::Status);
                    serde_json::to_value(McpOAuthStatus {
                        state: match command {
                            | McpOAuthCommand::Login => McpOAuthState::Pending,
                            | McpOAuthCommand::Cancel => McpOAuthState::Cancelled,
                            | McpOAuthCommand::Logout => McpOAuthState::SignedOut,
                            | McpOAuthCommand::Status => McpOAuthState::Connected,
                        },
                        authorization_url: (command == McpOAuthCommand::Login).then(|| "https://auth.example.invalid/authorize?state=fixture".into()),
                        expires_at: None,
                        message: None,
                    })
                    .unwrap()
                } else if path.ends_with("/revisions") && first.starts_with("post ") {
                    serde_json::to_value(&prepared_revision).unwrap()
                } else if path.ends_with("/versions") {
                    serde_json::to_value(&versions).unwrap()
                } else if path.contains("/conversations/")
                    && path
                        .rsplit('/')
                        .next()
                        .and_then(|part| part.parse::<i64>().ok())
                        .is_some_and(|id| saved_chats.contains_key(&id))
                {
                    saved_chats[&path.rsplit('/').next().unwrap().parse::<i64>().unwrap()].clone()
                } else if path.ends_with("/jobs") && first.starts_with("post ") {
                    serde_json::to_value(&detail.as_ref().unwrap().job).unwrap()
                } else if project_history && path == "/v1/providers" {
                    let mut providers = vec![
                        serde_json::json!({"provider":"claude","name":"Claude","available":true,"issue":null,"capabilities":{"background_jobs":true,"tool_timeline":true,"approvals":true,"user_input":true}}),
                    ];
                    if antigravity_installed {
                        providers.push(serde_json::json!({"provider":"antigravity","name":"Gemini / Antigravity","available":true,"issue":null}));
                    }
                    serde_json::json!(providers)
                } else if project_history && mode.load(std::sync::atomic::Ordering::Relaxed) == 3 && (path == "/v1/jobs" || path.contains("/conversations?")) {
                    serde_json::json!([])
                } else if project_history && path.contains("/conversations?") {
                    if path.contains("/claude/") {
                        let id = if header.contains("x-knd-workspace: 43") { 17 } else { 7 };
                        if deleted_chats.contains(&id) {
                            serde_json::json!([])
                        } else {
                            serde_json::json!([{"id":id,"title":format!("Project chat {id}"),"updated_at":id,"archived":archived_chats.contains(&id)}])
                        }
                    } else {
                        serde_json::json!([])
                    }
                } else if project_history && path == "/v1/jobs" && detail.is_none() {
                    serde_json::json!([7,17].into_iter().filter(|id| !deleted_chats.contains(id)).map(|id| serde_json::json!({
                        "id":id+100,"scope":{"provider":"claude","account_id":"default","workspace_id":if id==7{42}else{43},"workspace_path":"/fixture"},
                        "state":"completed","conversation_id":id,"operation_id":null,"pending_request_id":null,"last_sequence":0,
                        "preview":format!("Project chat {id}"),"cancel_requested":false,"error":null,"created_at":id
                    })).collect::<Vec<_>>())
                } else if path == "/v1/jobs" {
                    serde_json::to_value(detail.iter().map(|detail| &detail.job).collect::<Vec<_>>()).unwrap()
                } else if path.starts_with("/v1/jobs/") && !path.contains("/events") {
                    serde_json::to_value(&detail).unwrap()
                } else if path == "/v1/guidance" {
                    serde_json::to_value(GuidanceSnapshot::default()).unwrap()
                } else if path == "/v1/projects" {
                    let mut catalog = project_catalog();
                    if first.starts_with("post ") {
                        catalog.registered_workspace_id = Some(43);
                    }
                    serde_json::to_value(catalog).unwrap()
                } else if path.ends_with("/worktrees") || path == "/v1/workspaces/44" {
                    serde_json::to_value(project_catalog()).unwrap()
                } else if path.ends_with("/accounts") {
                    if first.starts_with("post ") {
                        serde_json::json!({"id": "work", "provider": provider, "label": "Work"})
                    } else if project_history && mode.load(std::sync::atomic::Ordering::Relaxed) == 4 && provider == Provider::Claude {
                        serde_json::json!([{"id":"default","provider":provider,"label":"Default"},{"id":"11111111111111111111111111111111","provider":provider,"label":"Work"}])
                    } else {
                        serde_json::json!([{"id": "default", "provider": provider, "label": "default"}])
                    }
                } else if path.contains("/files?") {
                    if mode.load(std::sync::atomic::Ordering::Relaxed) == 5 {
                        serde_json::json!({"error":"Workspace file list is unavailable"})
                    } else if path.ends_with("path=src%2fempty") {
                        serde_json::json!({"path":"src/empty","entries":[],"truncated":false})
                    } else if path.ends_with("path=src") {
                        serde_json::json!({"path":"src","entries":[{"path":"src/empty","directory":true},{"path":"src/main 한글.rs","directory":false}],"truncated":false})
                    } else {
                        serde_json::json!({"path":"","entries":[{"path":"src","directory":true},{"path":"README.md","directory":false}],"truncated":false})
                    }
                } else if path.ends_with("/context") {
                    let source: FileContextRequest = serde_json::from_str(&payload).unwrap();
                    serde_json::to_value(Attachment::Text {
                        name: "main 한글.rs".into(),
                        text: "second\n".into(),
                        source: Some(source),
                    })
                    .unwrap()
                } else if path.ends_with("/models") || path.ends_with("/versions") || path.contains("/conversations?") {
                    serde_json::json!([])
                } else if path.ends_with("/conversations/7") {
                    serde_json::json!({"id":7,"model":null,"session_id":"saved-session","messages":[]})
                } else if path.ends_with("/login") {
                    serde_json::json!({"type": "authenticated", "status": status})
                } else {
                    status
                };
                let body = if path.starts_with("/v1/jobs/") && path.contains("/events?after=") {
                    let cursor: i64 = path.split("after=").nth(1).unwrap().parse().unwrap();
                    journal
                        .iter()
                        .filter(|event| event.sequence >= cursor)
                        .map(|event| format!("{}\n", serde_json::to_string(event).unwrap()))
                        .collect::<String>()
                } else {
                    format!("{value}\n")
                };
                let status_line = if (!project_history && path.ends_with("/status") && header.contains("x-knd-workspace: 43"))
                    || (mode.load(std::sync::atomic::Ordering::Relaxed) == 12 && path == "/v1/runtimes/antigravity")
                    || (mode.load(std::sync::atomic::Ordering::Relaxed) == 11 && path == "/v1/control/computer")
                    || (mode.load(std::sync::atomic::Ordering::Relaxed) == 2 && path.contains("/conversations?"))
                    || missing_project
                    || (mode.load(std::sync::atomic::Ordering::Relaxed) == 5 && path.contains("/files?"))
                    || (path.starts_with("/v1/jobs/") && detail.is_none())
                {
                    "503 Service Unavailable"
                } else {
                    "200 OK"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            client,
            requests,
            stop,
            thread: Some(thread),
            history_mode,
        }
    }
}

impl Drop for AccountApi {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.thread.take().unwrap().join();
    }
}

fn wait_for_accounts(view: &Entity<ChatApp>, cx: &mut TestAppContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let done = cx.update(|cx| {
            view.update(cx, |s, _| {
                s.account_updates();
                s.accounts.providers.iter().all(|p| !p.loading && p.accounts.iter().all(|a| !a.busy))
            })
        });
        if done {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "account request timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[gpui_kit::test]
fn account_failures_and_busy_state_stay_with_their_provider(cx: &mut TestAppContext) {
    use crate::accounts::Change;
    use gpui_agent_presentation::{Action,
                                  Update};
    let (_, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            s.accounts.providers[0].loading = true;
            s.accounts.providers[1].loading = true;
            s.accounts.providers[1].label = "keep label".into();
            s.accounts.providers[2].accounts[0].busy = true;
            s.accounts.tx.send(Change::Catalog(Provider::Claude, Err("catalog failed".into()))).unwrap();
            s.accounts.tx.send(Change::Created(Provider::Codex, Err("create failed".into()))).unwrap();
            s.accounts
                .tx
                .send(Change::Account(
                    Provider::Antigravity,
                    "default".into(),
                    Box::new(Update::Error("login failed".into())),
                ))
                .unwrap();
            s.accounts
                .tx
                .send(Change::Account(Provider::Antigravity, "default".into(), Box::new(Update::Finished)))
                .unwrap();
            s.account_updates();
            assert_eq!(s.accounts.providers[0].error.as_deref(), Some("catalog failed"));
            assert_eq!(s.accounts.providers[1].error.as_deref(), Some("create failed"));
            assert_eq!(s.accounts.providers[1].label, "keep label");
            assert_eq!(s.accounts.providers[2].accounts[0].auth.error.as_deref(), Some("login failed"));
            assert!(s.accounts.providers.iter().all(|p| !p.loading && p.accounts.iter().all(|a| !a.busy)));
            assert!(s.conversations[0].status.as_ref().unwrap().authenticated);
            s.conversations[0].input = "draft".into();
            assert!(s.can_send());
            s.accounts.providers[0].accounts[0].busy = true;
            assert!(!s.can_send());
            s.accounts.providers[0].accounts[0].busy = false;
            s.busy = true;
            s.account_action(Provider::Claude, "default".into(), Action::Logout);
            assert!(
                !s.accounts.busy(Provider::Claude, "default"),
                "do not change the account used by an active conversation"
            );
        })
    });
}

#[gpui_kit::test]
fn account_requests_are_scoped_and_preserve_conversations(cx: &mut TestAppContext) {
    use gpui_agent_presentation::Action;
    let api = AccountApi::new();
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            s.client = api.client.clone();
            s.selected = 0;
            s.conversations[0].input = "Claude draft".into();
            s.conversations[1].input = "Codex draft".into();
            s.conversations[1].session_id = Some("existing-session".into());
            for provider in PROVIDERS {
                s.refresh_accounts(provider);
            }
        })
    });
    wait_for_accounts(&view, cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            for provider in PROVIDERS {
                let account = &s.accounts.providers[provider_index(provider)].accounts[0];
                assert!(
                    account
                        .auth
                        .status
                        .as_ref()
                        .unwrap_or_else(|| panic!(
                            "{provider:?}: catalog={:?}, account={:?}",
                            s.accounts.providers[provider_index(provider)].error,
                            account.auth.error
                        ))
                        .authenticated
                );
                assert_eq!(account.auth.status.as_ref().unwrap().account.as_deref(), Some("fixture@example.com"));
            }
            s.account_action(Provider::Codex, "default".into(), Action::Login);
            s.account_action(Provider::Antigravity, "default".into(), Action::Logout);
            assert!(s.accounts.busy(Provider::Codex, "default"));
            assert!(!s.accounts.busy(Provider::Claude, "default"));
        })
    });
    wait_for_accounts(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.panel = Some(Panel::Accounts);
            s.accounts.providers[1].label = "Work".into();
            s.accounts.selected = 1;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("accounts-manage-codex", cx);
        window.render_frame(cx);
        window.click("account-add-codex", cx);
    })
    .unwrap();
    wait_for_accounts(&view, cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            assert_eq!(s.accounts.providers[1].accounts.len(), 2);
            assert!(s.accounts.providers[1].label.is_empty());
            s.account_action(Provider::Codex, "work".into(), Action::Logout);
        })
    });
    wait_for_accounts(&view, cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            assert!(
                s.conversations[1].status.as_ref().unwrap().authenticated,
                "additional account logout must not change the default account"
            );
            assert!(!s.conversations[2].status.as_ref().unwrap().authenticated);
            assert_eq!(s.selected, 0);
            assert_eq!(s.conversations[0].input, "Claude draft");
            assert_eq!(s.conversations[1].input, "Codex draft");
            assert_eq!(s.conversations[1].session_id.as_deref(), Some("existing-session"));
            assert!(!s.busy);
        })
    });
    let requests: Vec<_> = api.requests.try_iter().collect();
    for provider in PROVIDERS {
        let key = provider_name(provider);
        assert!(requests.iter().any(|r| r.starts_with(&format!("get /v1/providers/{key}/accounts "))));
        assert!(requests.iter().any(|r| r.starts_with(&format!("get /v1/providers/{key}/status "))
            && r.contains("x-knd-account: default")
            && r.contains(&format!("x-knd-provider: {key}"))));
    }
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/providers/codex/login ") && r.contains("x-knd-account: default"))
    );
    assert!(requests.iter().any(|r| r.starts_with("post /v1/providers/antigravity/logout ")));
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/providers/codex/accounts ") && r.contains("\"label\":\"Work\""))
    );
    assert!(
        requests
            .iter()
            .any(|r| r.starts_with("post /v1/providers/codex/logout ") && r.contains("x-knd-account: work"))
    );
}

#[gpui_kit::test]
fn given_a_long_conversation_when_resized_then_only_viewport_rows_are_built(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |s, cx| {
            s.conversations[0].messages = (0 .. 100)
                .map(|n| SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(n),
                    user: n % 2 == 0,
                    text: format!("Message {n}\n\n{}", "A paragraph for variable-height layout.\n\n".repeat(3)),
                    attachments: vec![],
                    incomplete: false,
                })
                .collect();
            s.conversations[0].scroll_to = Some(gpui_agent_presentation::ScrollTarget::Message(0));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("chat-None-0-0-bubble").visible());
        assert!(window.try_find("chat-None-0-99-bubble").is_none(), "offscreen messages must not be built");
        window.click("chat-None-0-0-copy", cx);
        assert!(cx.read_from_clipboard().unwrap().text().unwrap().starts_with("Message 0"));
        view.update(cx, |s, cx| {
            s.conversations[0].view_query = "Message 80".into();
            s.conversations[0].scroll_to = Some(gpui_agent_presentation::ScrollTarget::Message(80));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("chat-None-0-80-bubble").visible());
        assert!(window.try_find("chat-None-0-0-bubble").is_none());
        let origin = window.find("sidebar-resize").bounds().center();
        window.drag(origin, gpui::point(px(350.), origin.y), cx);
        assert!(window.find("chat-None-0-80-bubble").visible());
        view.update(cx, |s, cx| {
            s.conversations[0].view_query.clear();
            s.conversations[0].scroll_to = Some(gpui_agent_presentation::ScrollTarget::Bottom);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("chat-None-0-99-bubble").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
#[ignore = "manual debug-profile redraw measurement; run with --ignored --nocapture"]
fn measure_message_redraw_cost(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        cx.set_reduce_motion(true);
        for count in [0, 10, 50, 100] {
            view.update(cx, |s, cx| {
                s.conversations[0].messages = (0 .. count)
                    .map(|n| SavedMessage {
                        skills: vec![],
                        guidance: None,
                        id: Some(n),
                        user: n % 2 == 0,
                        text: "# Diagnostic\n\nA synthetic paragraph for layout measurement.\n\n```rust\nfn main() { println!(\"fixture\"); }\n```\n\n"
                            .repeat(3),
                        attachments: vec![],
                        incomplete: false,
                    })
                    .collect();
                s.sidebar_width = 256.;
                cx.notify();
            });
            window.render_frame(cx);
            let mut frames = Vec::new();
            for n in 0 .. 12 {
                view.update(cx, |s, cx| {
                    s.sidebar_width = 256. + (n % 2) as f32 * 20.;
                    cx.notify();
                });
                let start = std::time::Instant::now();
                window.render_frame(cx);
                frames.push(start.elapsed().as_secs_f64() * 1000.);
            }
            frames.sort_by(f64::total_cmp);
            let start = std::time::Instant::now();
            window.click("theme-toggle", cx);
            let click = start.elapsed().as_secs_f64() * 1000.;
            let origin = window.find("sidebar-resize").bounds().center();
            let start = std::time::Instant::now();
            window.drag(origin, gpui::point(px(350.), origin.y), cx);
            let drag = start.elapsed().as_secs_f64() * 1000.;
            eprintln!(
                "DIAGNOSTIC messages={count} frame_median_ms={:.2} frame_p90_ms={:.2} click_total_ms={click:.2} drag_total_ms={drag:.2}",
                frames[6], frames[10]
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_idle_ticks_when_no_state_changes_then_redraws_wait_for_a_result(cx: &mut TestAppContext) {
    let (_, view) = fixture(cx);
    cx.update(|cx| view.update(cx, |s, _| s.poll()));
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = notifications.clone();
    let _subscription = cx.update(|cx| cx.observe(&view, move |_, _| observed.set(observed.get() + 1)));
    for _ in 0 .. 3 {
        cx.update(|cx| view.update(cx, |s, cx| s.tick(cx)));
    }
    assert_eq!(notifications.get(), 0, "idle ticks must not notify the root view");
    cx.update(|cx| {
        view.update(cx, |s, cx| {
            s.tx.send((0, gpui_agent_presentation::Update::Error("synthetic failure".into()))).unwrap();
            s.tick(cx);
        });
    });
    assert_eq!(notifications.get(), 1, "completed background results must still redraw");
    assert_eq!(
        view.read_with(cx, |s, _| s.conversations[0].error.clone()).as_deref(),
        Some("synthetic failure")
    );
}

#[gpui_kit::test]
fn given_virtual_messages_when_streaming_then_tail_follows_without_moving_a_scrolled_up_reader(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        let message = |n| SavedMessage {
            skills: vec![],
            guidance: None,
            id: Some(n),
            user: false,
            text: format!("Message {n}\n\n{}", "A variable-height paragraph.\n\n".repeat(6)),
            attachments: vec![],
            incomplete: false,
        };
        view.update(cx, |s, cx| {
            s.conversations[0].messages = (0 .. 100).map(message).collect();
            s.conversations[0].scroll_to = Some(gpui_agent_presentation::ScrollTarget::Bottom);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("chat-None-0-99-bubble").visible());
        view.update(cx, |s, cx| {
            s.conversations[0].messages.push(message(100));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("chat-None-0-100-copy").visible());
        window.scroll("messages-scroll", gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(1500.))), cx);
        let anchor = view.read(cx).message_lists.borrow()[crate::messages::MessageSource::Chat as usize]
            .state
            .logical_scroll_top();
        assert!(
            !view.read(cx).message_lists.borrow()[crate::messages::MessageSource::Chat as usize]
                .state
                .is_following_tail()
        );
        view.update(cx, |s, cx| {
            s.conversations[0].messages[100].text.push_str(&"More streamed text.\n\n".repeat(30));
            s.conversations[0].messages.push(message(101));
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            view.read(cx).message_lists.borrow()[crate::messages::MessageSource::Chat as usize]
                .state
                .logical_scroll_top()
                .item_ix,
            anchor.item_ix
        );
        assert!(window.try_find("chat-None-0-101-bubble").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_long_reference_conversation_when_opened_then_it_uses_its_own_virtual_viewport(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    let messages = || {
        (0 .. 100)
            .map(|n| SavedMessage {
                skills: vec![],
                guidance: None,
                id: Some(n),
                user: false,
                text: format!("Message {n}\n\n{}", "Paragraph.\n\n".repeat(6)),
                attachments: vec![],
                incomplete: false,
            })
            .collect()
    };
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.reference = Some(crate::app::ReferenceConversation {
                provider: 0,
                title: "Reference".into(),
                conversation: SavedConversation {
                    id: 42,
                    model: None,
                    session_id: None,
                    messages: messages(),
                    timeline: vec![],
                },
            });
            s.right_panels.reference_open = true;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.find("reference-None-0-42-0-bubble").visible());
        assert!(window.try_find("reference-None-0-42-99-bubble").is_none());
        assert_eq!(
            view.read(cx).message_lists.borrow()[crate::messages::MessageSource::Reference as usize]
                .state
                .logical_scroll_top()
                .item_ix,
            0
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_clamped_sidebar_when_dragging_further_then_unchanged_width_does_not_notify(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.sidebar_width = 460.;
            cx.notify();
        });
        window.render_frame(cx);
    })
    .unwrap();
    let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = notifications.clone();
    let _subscription = cx.update(|cx| cx.observe(&view, move |_, _| observed.set(observed.get() + 1)));
    cx.update_window(handle.into(), |_, window, cx| {
        let bounds = window.find("sidebar-resize").bounds();
        // Crossing the hero button also notifies the view for its hover style.
        let origin = gpui::point(bounds.center().x, bounds.top() + px(100.));
        assert!(origin.y < window.find("hero-project").bounds().top());
        window.drag(origin, gpui::point(px(900.), origin.y), cx);
    })
    .unwrap();
    assert_eq!(notifications.get(), 0);
    assert_eq!(view.read_with(cx, |s, _| s.sidebar_width), 460.);
}

#[gpui_kit::test]
fn given_a_long_conversation_with_work_panel_open_when_redrawn_or_expanded_then_frames_stay_interactive(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            let mut messages = vec![];
            let mut timeline = vec![];
            for n in 0 .. 40i64 {
                messages.push(SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(n * 2 + 1),
                    user: true,
                    text: format!("question {n}"),
                    attachments: vec![],
                    incomplete: false,
                });
                messages.push(SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(n * 2 + 2),
                    user: false,
                    text: format!("# answer {n}\n\n- a\n- b\n\n```rust\nfn main() {{}}\n```\n").repeat(5),
                    attachments: vec![],
                    incomplete: false,
                });
                for t in 0 .. 6 {
                    timeline.push(TimelineEntry {
                        message_id: n * 2 + 2,
                        event: Event::Tool {
                            call: ToolCall {
                                id: format!("tool-{n}-{t}"),
                                title: Some("Bash".into()),
                                status: Some("completed".into()),
                                input: Some(serde_json::json!({"command":"ls -la"})),
                                output: Some(serde_json::json!("line of output\n".repeat(1500))),
                                output_delta: None,
                                activity: vec![],
                            },
                        },
                    });
                }
            }
            s.conversations[0].messages = messages;
            s.conversations[0].timeline = timeline;
            s.right_panels.work_open = true;
            cx.notify();
        });
        window.render_frame(cx);
        for i in 0 .. 3 {
            let start = std::time::Instant::now();
            view.update(cx, |_, cx| cx.notify());
            window.render_frame(cx);
            // Unvirtualized work rows took ~2.7s per frame here.
            assert!(
                start.elapsed() < std::time::Duration::from_millis(1000),
                "redraw {i} took {:?}",
                start.elapsed()
            );
        }
        let start = std::time::Instant::now();
        view.update(cx, |s, cx| {
            s.expanded_tools.insert("chat-None-0-79-80-tool-39-0".into());
            cx.notify();
        });
        window.render_frame(cx);
        assert!(start.elapsed() < std::time::Duration::from_millis(1000), "expand took {:?}", start.elapsed());
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_long_conversation_with_large_output_when_typing_scrolling_and_expanding_then_each_frame_stays_within_budget(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            let mut messages = vec![];
            let mut timeline = vec![];
            for n in 0 .. 40i64 {
                messages.push(SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(n * 2 + 1),
                    user: true,
                    text: format!("question {n}"),
                    attachments: vec![],
                    incomplete: false,
                });
                messages.push(SavedMessage {
                    skills: vec![],
                    guidance: None,
                    id: Some(n * 2 + 2),
                    user: false,
                    text: format!("# answer {n}\n\n- a\n- b\n\n```rust\nfn main() {{}}\n```\n").repeat(5),
                    attachments: vec![],
                    incomplete: false,
                });
                for t in 0 .. 6 {
                    timeline.push(TimelineEntry {
                        message_id: n * 2 + 2,
                        event: Event::Tool {
                            call: ToolCall {
                                id: format!("tool-{n}-{t}"),
                                title: Some("Bash".into()),
                                status: Some("completed".into()),
                                input: Some(serde_json::json!({"command":"ls -la"})),
                                output: Some(serde_json::json!(if n == 39 && t == 0 {
                                    "line of output\n".repeat(30000)
                                } else {
                                    "line of output\n".repeat(1500)
                                })),
                                output_delta: None,
                                activity: vec![],
                            },
                        },
                    });
                }
            }
            s.conversations[0].messages = messages;
            s.conversations[0].timeline = timeline;
            cx.notify();
        });
        let frame = |label: &str, cx: &mut gpui::App, window: &mut gpui::Window| {
            let start = std::time::Instant::now();
            window.render_frame(cx);
            // Debug builds measure 20-85ms alone and up to ~800ms while the
            // whole suite runs in parallel; the pre-virtualization work panel
            // took ~2.7s on an idle machine.
            assert!(start.elapsed() < std::time::Duration::from_millis(1200), "{label} took {:?}", start.elapsed());
        };
        frame("warm", cx, window);
        for work in [false, true] {
            view.update(cx, |s, cx| {
                s.right_panels.work_open = work;
                cx.notify();
            });
            frame(&format!("work={work} open-panel"), cx, window);
            for i in 0 .. 2 {
                view.update(cx, |s, cx| {
                    s.conversations[0].input.push('a');
                    cx.notify();
                });
                frame(&format!("work={work} root-notify {i}"), cx, window);
            }
        }
        view.update(cx, |s, cx| {
            s.work_list.borrow().state.scroll_to_end();
            cx.notify();
        });
        frame("scroll work list to end", cx, window);
        view.update(cx, |s, cx| {
            s.expanded_tools.insert("chat-None-0-79-80-tool-39-0".into());
            cx.notify();
        });
        frame("expand 450KB output", cx, window);
        view.update(cx, |_, cx| cx.notify());
        frame("redraw with big output open", cx, window);
        view.update(cx, |s, cx| {
            s.right_panels.work_open = true;
            cx.notify();
        });
        for i in 0 .. 3 {
            view.update(cx, |s, cx| {
                s.conversations[0].timeline.push(TimelineEntry {
                    message_id: 80,
                    event: Event::Tool {
                        call: ToolCall {
                            id: format!("stream-{i}"),
                            title: Some("Bash".into()),
                            status: Some("in_progress".into()),
                            input: None,
                            output: None,
                            output_delta: None,
                            activity: vec![],
                        },
                    },
                });
                cx.notify();
            });
            frame(&format!("stream append {i}"), cx, window);
        }
        view.update(cx, |s, cx| {
            s.right_panels.work_open = false;
            s.conversations[0].messages.clear();
            s.conversations[0].timeline.clear();
            cx.notify();
        });
        frame("empty conversation (chrome only)", cx, window);
        frame("empty conversation again", cx, window);
        view.update(cx, |_, cx| cx.notify());
        frame("empty conversation notify", cx, window);
        view.update(cx, |s, cx| {
            s.sidebar_open = false;
            cx.notify();
        });
        frame("no sidebar", cx, window);
        frame("no sidebar again", cx, window);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_the_pr_or_git_pane_when_its_edge_is_dragged_then_width_is_clamped_saved_and_reset_by_double_click(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1500.), px(780.)));
    let preferences = std::env::temp_dir().join(format!("gpui-agent-tool-width-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&preferences);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| s.language_path = Some(preferences.join("ui-language")));
        window.click("chat-pr", cx);
        window.render_frame(cx);
        assert_eq!(window.find("chat-tool-panel").bounds().size.width, px(480.));
        let origin = window.find("chat-tool-resize").bounds().center();
        window.drag(origin, gpui::point(px(800.), origin.y), cx);
        assert_eq!(window.find("chat-tool-panel").bounds().size.width, px(700.));
        let origin = window.find("chat-tool-resize").bounds().center();
        window.drag(origin, gpui::point(px(1490.), origin.y), cx);
        assert_eq!(window.find("chat-tool-panel").bounds().size.width, px(280.));
        let saved = std::fs::read_to_string(preferences.join("ui-right-panels.json")).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&saved).unwrap()["tool_width"], 280.);
        window.double_click("chat-tool-resize", cx);
        assert_eq!(window.find("chat-tool-panel").bounds().size.width, px(480.));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&preferences);
}

#[gpui_kit::test]
fn given_a_form_pane_when_its_edge_is_dragged_then_only_its_outline_is_laid_out_and_inputs_survive(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1500.), px(780.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("chat-pr", cx);
        window.render_frame(cx);
        let full = std::time::Instant::now();
        view.update(cx, |_, cx| cx.notify());
        window.render_frame(cx);
        let full = full.elapsed();
        assert!(window.try_find("pr-preview").is_some());
        view.update(cx, |s, _| s.workspaces.git.pr.title = "Draft title".into());
        for width in [500., 520., 540., 560.] {
            view.update(cx, |s, cx| {
                s.tool_resizing = true;
                s.right_panels.tool_width = Some(width);
                cx.notify();
            });
            let start = std::time::Instant::now();
            window.render_frame(cx);
            // The PR form costs ~235ms per frame in debug; its outline ~20ms.
            assert!(
                start.elapsed() * 3 < full,
                "drag frame {:?} should be far cheaper than a full frame {full:?}",
                start.elapsed()
            );
            assert_eq!(window.find("chat-tool-panel").bounds().size.width, px(width));
            assert!(window.try_find("pr-preview").is_none());
        }
        // Dragging ends: the form returns with its state intact.
        view.update(cx, |s, cx| {
            s.tool_resizing = false;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("pr-preview").is_some());
        assert_eq!(view.read(cx).workspaces.git.pr.title, "Draft title");
        assert!(view.read(cx).inputs.contains_key("pr-title"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_many_history_entries_when_the_sidebar_redraws_then_only_visible_rows_are_built(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1500.), px(900.)));
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |s, cx| {
            let projects: Vec<_> = (0 .. 8)
                .map(|p| serde_json::json!({"id":p+1,"name":format!("P{p}"),"path":format!("/p{p}"),"git":false,"workspaces":[{"id":p+100,"project_id":p+1,"path":format!("/p{p}"),"main":true,"managed":false,"locked":false,"missing":false}]}))
                .collect();
            s.workspaces.catalog = serde_json::from_value(serde_json::json!({"default_workspace_id":100,"projects":projects})).unwrap();
            s.workspaces.loaded = true;
            s.workspaces.selected = Some(100);
            for p in 0 .. 8 {
                let conversations = (0 .. 100i64)
                    .map(|n| serde_json::from_value(serde_json::json!({"id":n+1,"title":format!("Conversation title number {n}"),"updated_at":n})).unwrap())
                    .collect();
                s.history.projects.insert(
                    p + 100,
                    crate::history::ProjectRecords {
                        groups: vec![gpui_agent_presentation::HistoryGroup {
                            provider: PROVIDERS[0],
                            account_id: "default".into(),
                            conversations,
                        }],
                        loading: false,
                        error: None,
                    },
                );
            }
            cx.notify();
        });
        window.render_frame(cx);
        let mut best = std::time::Duration::MAX;
        for _ in 0 .. 5 {
            view.update(cx, |_, cx| cx.notify());
            let start = std::time::Instant::now();
            window.render_frame(cx);
            best = best.min(start.elapsed());
        }
        eprintln!("PERF sidebar 800 entries: {best:?}");
        // Unvirtualized, 800 entries took ~565ms per frame in debug; a virtualized frame
        // takes ~80ms alone and up to ~160ms while the whole suite runs in parallel.
        assert!(best < std::time::Duration::from_millis(250), "sidebar frame took {best:?}");
        assert!(window.find("workspace-100").visible());
        let name = provider_name(PROVIDERS[0]);
        assert!(window.try_find(format!("project-history-100-{name}-default-chat-100")).is_some(), "visible rows are built");
        assert!(window.try_find(format!("project-history-107-{name}-default-chat-100")).is_none(), "off-screen rows must not be built");
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_cached_timeline_when_it_is_cleared_and_refilled_to_the_same_length_then_derived_facts_refresh(cx: &mut TestAppContext) {
    let (_handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            let entry = |event| TimelineEntry {
                message_id: 2,
                event,
            };
            s.conversations[0].timeline = vec![entry(Event::InputDelivery {
                message_id: 2,
                delivered: true,
            })];
            assert!(s.has_work_activity());
            // Same length and (usually) the same allocation, different content.
            s.conversations[0].timeline.clear();
            s.conversations[0].timeline.push(entry(Event::TurnEnd {
                completed: true,
            }));
            assert!(!s.has_work_activity(), "the cache must notice a refilled timeline");
        });
    });
}

#[gpui_kit::test]
fn deleting_the_open_project_chat_resets_the_header_title(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.command(gpui_agent_presentation::Action::Refresh);
        })
    });
    wait_for_sidebar_element(handle, &view, "project-history-42-claude-default-chat-7", cx);
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("project-history-42-claude-default-chat-7", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let s = view.read(cx);
        assert_eq!(s.conversations[0].id, Some(7));
        assert_eq!(s.conversations[0].title.as_deref(), Some("Project chat 7"));
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("history-menu-7", cx);
        window.within("popup-menu").click(5usize, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("delete-confirm", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    wait_for_sidebar_element(handle, &view, "history-empty-42", cx);
    cx.update(|cx| {
        let s = view.read(cx);
        assert_eq!(s.conversations[0].id, None);
        assert_eq!(s.conversations[0].title, None);
    });
}

#[gpui_kit::test]
fn deleting_the_open_chat_of_a_second_account_resets_the_header_title(cx: &mut TestAppContext) {
    let api = AccountApi::with_project_history(true);
    api.history_mode.store(4, std::sync::atomic::Ordering::Relaxed);
    let (handle, view) = fixture(cx);
    cx.update(|cx| {
        view.update(cx, |s, _| {
            (s.tx, s.rx) = std::sync::mpsc::sync_channel(512);
            s.client = api.client.clone();
            s.command(gpui_agent_presentation::Action::Refresh);
        })
    });
    let work = "project-history-42-claude-11111111111111111111111111111111-chat-7";
    wait_for_sidebar_element(handle, &view, work, cx);
    wait_for_chat(&view, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(work, cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let c = &view.read(cx).conversations[0];
        assert_eq!((c.id, c.title.as_deref()), (Some(7), Some("Project chat 7")));
        assert!(!c.messages.is_empty());
    });
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, cx| {
            s.dialog = Some(HistoryDialog::Delete {
                provider: 0,
                id: 7,
                title: "Project chat 7".into(),
            });
            cx.notify();
        });
        window.render_frame(cx);
        window.click("delete-confirm", cx);
    })
    .unwrap();
    wait_for_chat(&view, cx);
    cx.update(|cx| {
        let c = &view.read(cx).conversations[0];
        assert_eq!((c.id, c.title.as_deref(), c.messages.len()), (None, None, 0));
    });
}

#[gpui_kit::test]
fn given_a_title_without_a_loaded_conversation_when_the_operation_finishes_then_the_header_falls_back_to_a_new_chat(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            // The title is set optimistically while a history entry loads; the
            // load then fails because the conversation was deleted.
            s.conversations[0].title = Some("Deleted thread".into());
            s.tx.send((0, gpui_agent_presentation::Update::Error("Conversation not found".into()))).unwrap();
            s.tx.send((0, gpui_agent_presentation::Update::Finished)).unwrap();
            s.updates(&s.wake.clone());
            assert_eq!(s.conversations[0].title, None);
        });
        window.render_frame(cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_a_loaded_conversation_when_the_operation_finishes_then_its_title_is_kept(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.update_window(handle.into(), |_, _window, cx| {
        view.update(cx, |s, _| {
            s.conversations[0].id = Some(7);
            s.conversations[0].title = Some("Kept thread".into());
            s.tx.send((0, gpui_agent_presentation::Update::Finished)).unwrap();
            s.updates(&s.wake.clone());
            assert_eq!(s.conversations[0].title.as_deref(), Some("Kept thread"));
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn given_required_checks_when_edited_as_lines_then_drafts_save_only_valid_changes(cx: &mut TestAppContext) {
    let (handle, view) = fixture(cx);
    cx.simulate_window_resize(handle.into(), size(px(1200.), px(2400.)));
    cx.update_window(handle.into(), |_, window, cx| {
        view.update(cx, |s, _| {
            let mut catalog = project_catalog();
            catalog.projects[0].verification.commands = vec![vec!["cargo".into(), "test".into()]];
            s.workspaces.catalog = catalog;
            s.panel = Some(Panel::Workspace);
        });
        window.render_frame(cx);
        let pid = 1;
        assert_eq!(window.find("project-verification-status-1").label(), Some("On · 1 command"));
        assert!(window.find("verification-command-1-0").visible());
        assert_eq!(window.find("project-verification-save-1").disabled(), Some(true));

        window.click("verification-command-add-1", cx);
        window.render_frame(cx);
        assert!(window.find("verification-command-1-1").visible());
        assert_eq!(
            window.find("project-verification-save-1").disabled(),
            Some(true),
            "an empty new row is not a change"
        );

        view.update(cx, |s, _| {
            s.workspaces.verification_drafts.get_mut(&pid).unwrap().commands[1] = "echo 'open".into()
        });
        window.render_frame(cx);
        assert_ne!(window.find("project-verification-save-1").disabled(), Some(true));
        window.click("project-verification-save-1", cx);
        assert_eq!(view.read(cx).workspaces.error.as_deref(), Some("A command has an unclosed quote."));

        assert!(window.try_find("project-verification-attempts-1").is_none());
        window.click("project-verification-advanced-1", cx);
        window.render_frame(cx);
        assert!(window.find("project-verification-attempts-1").visible());

        window.click("verification-command-remove-1-0", cx);
        assert_eq!(view.read(cx).workspaces.verification_drafts[&pid].commands, ["echo 'open"]);
        window.render_frame(cx);
        window.click("project-verification-revert-1", cx);
        assert_eq!(view.read(cx).workspaces.verification_drafts[&pid].commands, ["cargo test"]);
    })
    .unwrap();
}
