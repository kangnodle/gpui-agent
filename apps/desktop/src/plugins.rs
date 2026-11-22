use crate::{app::ChatApp,
            client::ApiClient,
            ui::*};
use gpui::{assets::IconName,
           base::TestSupportExt as _,
           *};
use gpui_agent_contracts::{McpServer,
                           Plugin,
                           PluginCatalog,
                           PluginClaudeSyncRequest,
                           PluginEnabledRequest,
                           PluginRemoveRequest,
                           PluginSyncResult};
use gpui_agent_presentation::Action;
use gpui_kit::component::{Icon,
                          popover::Popover,
                          tab::{Tab,
                                TabBar}};
use reqwest::Method;
use std::sync::mpsc::{self,
                      Receiver};

#[derive(Default)]
pub(super) struct PluginsUi {
    pub(super) picker_search: String,
    pub(super) picker_filter: String,
    pub(super) conversation_mcp: Vec<McpServer>,
    pub(super) conversation_mcp_scope: String,
    pub(super) conversation_mcp_loaded: bool,
    pub(super) conversation_mcp_error: Option<String>,
    pub(super) conversation_mcp_pending: Option<Receiver<Result<Vec<McpServer>, String>>>,
    pub(super) catalog: PluginCatalog,
    pub(super) scope: String,
    pub(super) loaded: bool,
    pub(super) search: String,
    pub(super) user: bool,
    pub(super) selected: Option<String>,
    pub(super) remove: bool,
    pub(super) error: Option<String>,
    pub(super) pending: Option<Receiver<Result<PluginCatalog, String>>>,
    changed: bool,
    refresh_guidance: bool,
    /// Plugin IDs the last background sync imported and enabled.
    synced: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// The running request is a silent sync after an agent turn.
    auto: bool,
}

impl ChatApp {
    pub(super) fn conversation_plugins(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        use gpui_agent_presentation::ExtensionKind;
        self.follow_conversation_mcp_scope();
        let index = self.selected;
        if !self.conversations[index].guidance.extensions.mcp.is_empty()
            && !self.plugins.conversation_mcp_loaded
            && self.plugins.conversation_mcp_error.is_none()
        {
            self.load_conversation_mcp();
        }
        let scope = self.client.guidance_editor_scope();
        let account = self.conversations[index].account_id.clone();
        let conversation_id = self.conversations[index].id;
        let view = cx.weak_entity();
        let trigger = self
            .button("conversation-plugins-toggle", "플러그인 연결", true, cx, |s, _, _| {
                s.open_plugins();
                s.plugins.conversation_mcp_loaded = false;
                s.load_conversation_mcp();
                if !s.busy && s.conversations[s.selected].guidance.snapshot.is_none() {
                    s.start(Action::Guidance, &s.wake.clone());
                }
            })
            .icon(IconName::Plus);
        let picker = Popover::new(ElementId::Name(
            format!("conversation-extensions-{index}-{scope}-{account:?}-{conversation_id:?}").into(),
        ))
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popup = cx.entity().downgrade();
            let close_popup = popup.clone();
            view.update(cx, |s, cx| {
                if s.selected != index
                    || s.client.guidance_editor_scope() != scope
                    || s.conversations[index].account_id != account
                    || s.conversations[index].id != conversation_id
                {
                    return Empty.into_any_element();
                }
                let mut body = column()
                    .id("conversation-plugins-picker")
                    .test_support()
                    .gap_2()
                    .w(px(380.))
                    .max_w(window.viewport_size().width - px(40.))
                    .child(row().justify_between().child(heading(s.text("이 대화에서 사용"))).child(s.icon_button(
                        "conversation-plugins-close",
                        "닫기",
                        IconName::X,
                        true,
                        cx,
                        move |_, window, cx| {
                            let _ = close_popup.update(cx, |state, cx| state.dismiss(window, cx));
                        },
                    )))
                    .child(note(s.text("체크 해제해도 설치·연결 상태는 유지됩니다."), cx))
                    .child(s.field(
                        "conversation-plugins-search",
                        "플러그인, 스킬, MCP 검색",
                        s.plugins.picker_search.clone(),
                        FieldKind::Line,
                        true,
                        window,
                        cx,
                        |s, value| s.plugins.picker_search = value,
                    ));
                let mut filters = row().flex_wrap().gap_1();
                for (filter, label) in [("", "전체"), ("plugin", "플러그인"), ("skill", "스킬"), ("mcp", "MCP")] {
                    filters = filters.child(s.choice(
                        format!("conversation-filter-{filter}"),
                        label,
                        s.plugins.picker_filter == filter,
                        true,
                        cx,
                        move |s, _, _| s.plugins.picker_filter = filter.into(),
                    ));
                }
                body = body.child(filters);
                let mut entries = column().gap_2();
                if let Some(error) = s.plugins.error.clone() {
                    entries = entries.child(note(error, cx)).child(s.button(
                        "conversation-plugins-retry",
                        "플러그인 확인 다시 시도",
                        !s.busy,
                        cx,
                        |s, _, _| {
                            s.plugins.loaded = false;
                            s.open_plugins();
                        },
                    ));
                } else if !s.plugins.loaded {
                    entries = entries.child(
                        div()
                            .id("conversation-plugins-loading")
                            .test_support()
                            .child(note(s.text("플러그인 확인 중…"), cx)),
                    );
                }
                if let Some(error) = s.plugins.conversation_mcp_error.clone() {
                    entries = entries
                        .child(note(error, cx))
                        .child(s.button("conversation-mcp-retry", "MCP 다시 확인", !s.busy, cx, |s, _, _| {
                            s.plugins.conversation_mcp_loaded = false;
                            s.load_conversation_mcp();
                        }));
                } else if !s.plugins.conversation_mcp_loaded {
                    entries = entries.child(note(s.text("MCP 연결 확인 중…"), cx));
                }
                let query = s.plugins.picker_search.trim().to_lowercase();
                let catalog = if s.plugins.loaded && s.plugins.error.is_none() {
                    s.plugins.catalog.clone()
                } else {
                    PluginCatalog::default()
                };
                let choices = s.conversations[index].guidance.extension_entries(&catalog, &s.plugins.conversation_mcp);
                let mut visible = 0;
                for entry in choices {
                    let (filter, label, icon) = match entry.kind {
                        | ExtensionKind::Plugin => ("plugin", "플러그인", IconName::Puzzle),
                        | ExtensionKind::Skill => ("skill", "스킬", IconName::Sparkles),
                        | ExtensionKind::Mcp => ("mcp", "MCP", IconName::Server),
                    };
                    if (!s.plugins.picker_filter.is_empty() && s.plugins.picker_filter != filter)
                        || !format!("{} {} {} {}", entry.name, entry.id, entry.description, label)
                            .to_lowercase()
                            .contains(&query)
                    {
                        continue;
                    }
                    visible += 1;
                    let id = entry.id.clone();
                    let kind = entry.kind;
                    let status = if !entry.enabled {
                        "설정에서 비활성화됨"
                    } else if kind == ExtensionKind::Mcp {
                        "등록됨"
                    } else {
                        "설치됨"
                    };
                    entries = entries.child(
                        column()
                            .gap_1()
                            .child(
                                row().gap_2().child(Icon::new(icon)).child(
                                    s.check(
                                        format!(
                                            "conversation-{}-check-{}",
                                            filter,
                                            if kind == ExtensionKind::Skill {
                                                entry.key.trim_start_matches("skill-")
                                            } else {
                                                &entry.id
                                            }
                                        ),
                                        &entry.name,
                                        entry.selected,
                                        !s.busy && (entry.enabled || entry.selected),
                                        cx,
                                        move |s, _| {
                                            if let Err(error) = s.conversations[index].guidance.toggle_extension(kind, &id) {
                                                s.conversations[index].error = Some(error.into());
                                            }
                                        },
                                    )
                                    .min_w_0()
                                    .max_w(px(280.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .tooltip({
                                        let text = entry.id.clone();
                                        move |window, cx| gpui::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
                                    }),
                                ),
                            )
                            .child(note(format!("{} · {} · {}", s.text(label), s.text(status), entry.id), cx)),
                    );
                }
                if visible == 0 {
                    entries = entries.child(note(s.text("선택할 수 있는 항목이 없습니다."), cx));
                }
                let selected = &s.conversations[index].guidance;
                let count = selected.selected.len() + selected.extensions.plugins.len() + selected.extensions.mcp.len();
                body.child(
                    div()
                        .id("conversation-plugins-list")
                        .max_h(px((f32::from(window.viewport_size().height) - 300.).clamp(60., 320.)))
                        .overflow_y_scroll()
                        .child(entries),
                )
                .child(note(format!("{} · {count}", s.text("현재 대화에만 적용")), cx))
                .child(s.button("conversation-plugins-manage", "설치·연결 관리", !s.busy, cx, move |s, window, cx| {
                    let _ = popup.update(cx, |state, cx| state.dismiss(window, cx));
                    s.open_panel(Panel::Plugins);
                }))
                .into_any_element()
            })
            .unwrap_or_else(|_| Empty.into_any_element())
        });
        let mut result = vec![picker.into_any_element()];
        let catalog = if self.plugins.loaded && self.plugins.error.is_none() {
            self.plugins.catalog.clone()
        } else {
            PluginCatalog::default()
        };
        for entry in self.conversations[index]
            .guidance
            .extension_entries(&catalog, &self.plugins.conversation_mcp)
            .into_iter()
            .filter(|entry| entry.selected && entry.enabled)
        {
            let (label, icon) = match entry.kind {
                | ExtensionKind::Plugin => ("플러그인", IconName::Puzzle),
                | ExtensionKind::Skill => ("스킬", IconName::Sparkles),
                | ExtensionKind::Mcp => ("MCP", IconName::Server),
            };
            let id = entry.id.clone();
            let kind = entry.kind;
            result.push(
                row()
                    .id(format!("conversation-{}-chip", entry.key))
                    .test_support()
                    .role(Role::Group)
                    .aria_label(format!("{} · {}", self.text(label), entry.name))
                    .gap_1()
                    .max_w(px(180.))
                    .overflow_hidden()
                    .child(Icon::new(icon))
                    .child(
                        self.check(format!("conversation-{}", entry.key), entry.name, true, !self.busy, cx, move |s, _| {
                            if let Err(error) = s.conversations[index].guidance.toggle_extension(kind, &id) {
                                s.conversations[index].error = Some(error.into());
                            }
                        })
                        .min_w_0()
                        .max_w(px(150.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .tooltip({
                            let text = format!("{} · {}", self.text(label), entry.id);
                            move |window, cx| gpui::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
                        }),
                    )
                    .into_any_element(),
            );
        }
        result
    }

    fn follow_conversation_mcp_scope(&mut self) {
        let index = self.selected;
        let provider = gpui_agent_contracts::PROVIDERS[index];
        let account = self.conversations[index].account_id.clone().unwrap_or_else(|| "default".into());
        let scope = format!("{}:{provider:?}:{account}", self.client.guidance_editor_scope());
        if self.plugins.conversation_mcp_scope != scope {
            self.plugins.conversation_mcp_scope = scope;
            self.plugins.conversation_mcp.clear();
            self.plugins.conversation_mcp_loaded = false;
            self.plugins.conversation_mcp_pending = None;
            self.plugins.conversation_mcp_error = None;
        }
    }

    fn load_conversation_mcp(&mut self) {
        self.follow_conversation_mcp_scope();
        let provider = gpui_agent_contracts::PROVIDERS[self.selected];
        let account = self.conversations[self.selected].account_id.clone().unwrap_or_else(|| "default".into());
        if self.plugins.conversation_mcp_loaded || self.plugins.conversation_mcp_pending.is_some() {
            return;
        }
        let client = self.client.for_account(provider, account, self.client.workspace_id());
        let wake = self.wake.clone();
        let (tx, rx) = mpsc::channel();
        self.plugins.conversation_mcp_pending = Some(rx);
        self.plugins.conversation_mcp_error = None;
        std::thread::spawn(move || {
            let result = client
                .response(Method::GET, "/v1/mcp", None)
                .and_then(|response| Ok(response.json()?))
                .map_err(|error| error.to_string());
            let _ = tx.send(result);
            wake.notify();
        });
    }

    pub(super) fn open_plugins(&mut self) {
        self.follow_plugin_scope();
        if !self.plugins.loaded && self.plugins.pending.is_none() {
            self.run_plugins(false, |client| Ok(client.response(Method::GET, "/v1/plugins", None)?.json()?));
        }
    }

    fn follow_plugin_scope(&mut self) {
        let scope = self.client.guidance_editor_scope();
        if scope != self.plugins.scope {
            self.plugins = PluginsUi {
                scope,
                user: self.plugins.user,
                ..Default::default()
            };
            self.mcp.plugin_servers.clear();
        }
    }

    fn run_plugins(&mut self, changed: bool, work: impl FnOnce(ApiClient) -> anyhow::Result<PluginCatalog> + Send + 'static) {
        if self.plugins.pending.is_some() {
            return;
        }
        let client = self.client.clone();
        let wake = self.wake.clone();
        let (tx, rx) = mpsc::channel();
        self.plugins.pending = Some(rx);
        self.plugins.changed = changed;
        self.plugins.error = None;
        std::thread::spawn(move || {
            let _ = tx.send(work(client).map_err(|error| error.to_string()));
            wake.notify();
        });
    }

    pub(super) fn poll_plugins(&mut self) -> bool {
        let mut changed = self.plugins.scope != self.client.guidance_editor_scope();
        self.follow_plugin_scope();
        if let Some(rx) = &self.plugins.conversation_mcp_pending
            && let Ok(result) = rx.try_recv()
        {
            changed = true;
            self.plugins.conversation_mcp_pending = None;
            match result {
                | Ok(servers) => {
                    self.plugins.conversation_mcp = servers;
                    self.plugins.conversation_mcp_loaded = true;
                },
                | Err(error) => self.plugins.conversation_mcp_error = Some(error),
            }
        }
        if let Some(rx) = &self.plugins.pending
            && let Ok(result) = rx.try_recv()
        {
            changed = true;
            self.plugins.pending = None;
            let auto = std::mem::take(&mut self.plugins.auto);
            match result {
                | Ok(catalog) => {
                    let imported = std::mem::take(&mut *self.plugins.synced.lock().unwrap_or_else(|e| e.into_inner()));
                    for id in imported {
                        let selection = &mut self.conversations[self.selected].guidance;
                        if !selection.extensions.plugins.contains(&id) {
                            let _ = selection.toggle_extension(gpui_agent_presentation::ExtensionKind::Plugin, &id);
                        }
                        self.plugins.refresh_guidance = true;
                    }
                    self.mcp.plugin_servers = catalog
                        .plugins
                        .iter()
                        .filter(|plugin| plugin.enabled)
                        .flat_map(|plugin| plugin.manifest.mcp_servers.clone())
                        .collect();
                    self.plugins.catalog = catalog;
                    self.plugins.loaded = true;
                    self.plugins.remove = false;
                    if self
                        .plugins
                        .selected
                        .as_ref()
                        .is_some_and(|id| !self.plugins.catalog.plugins.iter().any(|plugin| &plugin.id == id))
                    {
                        self.plugins.selected = None;
                    }
                    if self.plugins.changed {
                        self.plugins.refresh_guidance = true;
                        self.mcp.oauth.clear();
                    }
                },
                | Err(_) if auto => {},
                | Err(error) => self.plugins.error = Some(error),
            }
        }
        if self.plugins.refresh_guidance && !self.busy {
            changed = true;
            self.plugins.refresh_guidance = false;
            let wake = self.wake.clone();
            self.start(Action::Guidance, &wake);
        }
        if self.panel == Some(Panel::Plugins) && !self.plugins.loaded && self.plugins.error.is_none() && self.plugins.pending.is_none() {
            self.open_plugins();
            changed = true;
        }
        changed
    }

    pub(super) fn plugin_tabs(&self, wide: bool, cx: &Context<Self>) -> AnyElement {
        let selected = match self.panel {
            | Some(Panel::Skills) => 1,
            | Some(Panel::Mcp) => 2,
            | _ => 0,
        };
        if wide {
            let mut tabs = column().id("plugin-tabs").test_support().w(px(176.)).flex_shrink_0();
            for (index, (panel, label)) in [(Panel::Plugins, "설치됨"), (Panel::Skills, "스킬"), (Panel::Mcp, "MCP 연결")]
                .into_iter()
                .enumerate()
            {
                tabs = tabs.child(
                    self.sidebar_button(
                        format!("plugin-category-{index}"),
                        label,
                        gpui::assets::IconName::Bot,
                        true,
                        cx,
                        move |s, _, cx| {
                            s.open_panel(panel);
                            cx.notify();
                        },
                    )
                    .selected(selected == index),
                );
            }
            return tabs.into_any_element();
        }
        let tabs = TabBar::new("plugin-category-tabs")
            .underline()
            .w_full()
            .selected_index(selected)
            .child(Tab::new().label(self.text("설치됨")))
            .child(Tab::new().label(self.text("스킬")))
            .child(Tab::new().label(self.text("MCP 연결")))
            .on_click(cx.listener(|s, index, _, cx| {
                let panel = [Panel::Plugins, Panel::Skills, Panel::Mcp][*index];
                s.open_panel(panel);
                cx.notify();
            }));
        div().id("plugin-tabs").test_support().w_full().child(tabs).into_any_element()
    }

    /// Skills the agent installed through the Claude CLI become available at
    /// once: imported, enabled and selected for the current conversation.
    pub(super) fn auto_sync_claude_plugins(&mut self) {
        self.follow_plugin_scope();
        if self.plugins.pending.is_some() {
            return;
        }
        let synced = self.plugins.synced.clone();
        let request = PluginClaudeSyncRequest {
            scope: "user".into(),
            enable: true,
        };
        self.run_plugins(false, move |client| {
            let result: Option<PluginSyncResult> = client
                .response(Method::POST, "/v1/plugins/claude", Some(serde_json::to_value(request)?))
                .and_then(|response| Ok(response.json()?))
                .ok();
            match result {
                | Some(result) => {
                    if let Ok(mut ids) = synced.lock() {
                        ids.extend(result.imported);
                    }
                    Ok(result.catalog)
                },
                | None => Ok(client.response(Method::GET, "/v1/plugins", None)?.json()?),
            }
        });
        self.plugins.auto = true;
    }

    fn sync_claude_plugins(&mut self) {
        let request = PluginClaudeSyncRequest {
            scope: if self.plugins.user { "user" } else { "repo" }.into(),
            enable: false,
        };
        self.run_plugins(true, move |client| {
            let result: PluginSyncResult = client
                .response(Method::POST, "/v1/plugins/claude", Some(serde_json::to_value(request)?))?
                .json()?;
            if result.imported.is_empty() && !result.skipped.is_empty() {
                let reasons: Vec<_> = result.skipped.iter().map(|d| format!("{} · {}", d.source, d.message)).collect();
                anyhow::bail!("가져올 새 Claude 플러그인이 없습니다. {}", reasons.join(" / "));
            }
            Ok(result.catalog)
        });
    }

    fn import_plugin(&mut self, cx: &mut Context<Self>) {
        let scope = self.client.guidance_editor_scope();
        let user = self.plugins.user;
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(self.text("플러그인 폴더 가져오기").into()),
        });
        cx.spawn(async move |view, cx| {
            let picked = picked.await;
            let _ = view.update(cx, |s, cx| {
                if scope != s.client.guidance_editor_scope() || s.plugins.pending.is_some() {
                    return;
                }
                match picked {
                    | Ok(Ok(Some(paths))) if paths.len() == 1 => {
                        let path = paths[0].clone();
                        s.run_plugins(true, move |client| {
                            let request = gpui_agent_infrastructure::PluginFiles::collect_plugin(&path, if user { "user" } else { "repo" }.into())?;
                            Ok(client
                                .response(Method::POST, "/v1/plugins/import", Some(serde_json::to_value(request)?))?
                                .json()?)
                        });
                    },
                    | Ok(Err(error)) => s.plugins.error = Some(error.to_string()),
                    | Err(error) => s.plugins.error = Some(error.to_string()),
                    | _ => {},
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn plugins_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let available = self.plugins.pending.is_none();
        let mut body = column().gap_4();
        if let Some(message) = &self.plugins.error {
            body = body.child(error(message.clone(), cx).id("plugin-error").test_support());
        }
        let actions = row()
            .child(self.button("plugin-import", "플러그인 폴더 가져오기…", available, cx, |s, _, cx| {
                s.import_plugin(cx)
            }))
            .child(self.button("plugin-sync-claude", "Claude 설치 플러그인 가져오기", available, cx, |s, _, _| {
                s.sync_claude_plugins()
            }))
            .child(
                self.button("plugin-refresh", "플러그인 목록 새로고침", available, cx, |s, _, _| {
                    s.plugins.loaded = false;
                    s.plugins.error = None;
                    s.open_plugins();
                })
                .ghost(),
            );
        let tabs = TabBar::new("plugin-scope")
            .underline()
            .w_full()
            .selected_index(if self.plugins.user { 0 } else { 1 })
            .child(Tab::new().label(self.text("앱 공통")).disabled(!available))
            .child(Tab::new().label(self.text("현재 프로젝트")).disabled(!available))
            .on_click(cx.listener(|s, index, _, cx| {
                if s.plugins.pending.is_none() {
                    s.plugins.user = *index == 0;
                    cx.notify();
                }
            }));
        body = body.child(div().id("plugin-scope-tabs").test_support().w_full().child(tabs)).child(actions);
        body = body.child(note(self.text("가져온 플러그인은 비활성 상태입니다. 활성화와 MCP 로그인은 별도입니다."), cx));
        if !available {
            body = body.child(note(self.text("플러그인 처리 중…"), cx).id("plugin-loading").test_support());
        }
        for (index, diagnostic) in self.plugins.catalog.diagnostics.iter().enumerate() {
            let mut diagnostic_row = row().child(note(format!("{} · {}", diagnostic.source, diagnostic.message), cx));
            if gpui_agent_contracts::validate_plugin_id(&diagnostic.source).is_ok() {
                let request = PluginEnabledRequest {
                    id: diagnostic.source.clone(),
                    enabled: false,
                };
                diagnostic_row =
                    diagnostic_row.child(
                        self.button(format!("plugin-diagnostic-disable-{index}"), "비활성화", available, cx, move |s, _, _| {
                            let request = request.clone();
                            s.run_plugins(true, move |client| {
                                Ok(client
                                    .response(Method::PUT, "/v1/plugins/enabled", Some(serde_json::to_value(request)?))?
                                    .json()?)
                            });
                        }),
                    );
            }
            body = body.child(diagnostic_row);
        }
        if let Some(plugin) = self
            .plugins
            .selected
            .as_ref()
            .and_then(|id| self.plugins.catalog.plugins.iter().find(|plugin| &plugin.id == id))
            .cloned()
        {
            return body.child(self.plugin_detail(plugin, available, cx)).into_any_element();
        }
        body = body.child(self.field(
            "plugin-search",
            "플러그인 검색",
            self.plugins.search.clone(),
            FieldKind::Line,
            true,
            window,
            cx,
            |s, value| s.plugins.search = value,
        ));
        let query = self.plugins.search.trim().to_lowercase();
        let plugins: Vec<_> = self
            .plugins
            .catalog
            .plugins
            .iter()
            .filter(|plugin| {
                format!(
                    "{} {} {}",
                    plugin.manifest.name,
                    plugin.manifest.display_name.as_deref().unwrap_or_default(),
                    plugin.manifest.description
                )
                .to_lowercase()
                .contains(&query)
            })
            .cloned()
            .collect();
        if self.plugins.loaded && plugins.is_empty() {
            body = body.child(div().id("plugins-empty").test_support().child(empty_state(
                self.text(if query.is_empty() {
                    "설치된 플러그인이 없습니다"
                } else {
                    "검색 결과가 없습니다"
                }),
                self.text(if query.is_empty() {
                    "플러그인 폴더를 가져와 시작하세요."
                } else {
                    "다른 검색어를 입력해 보세요."
                }),
            )));
        }
        for (index, plugin) in plugins.into_iter().enumerate() {
            let id = plugin.id.clone();
            body = body.child(
                settings_group()
                    .border_color(cx.theme().border)
                    .child(self.button(
                        format!("plugin-card-{index}"),
                        plugin.manifest.display_name.as_deref().unwrap_or(&plugin.manifest.name),
                        true,
                        cx,
                        move |s, _, _| {
                            s.plugins.selected = Some(id.clone());
                            s.plugins.remove = false;
                        },
                    ))
                    .child(note(plugin.manifest.description.clone(), cx))
                    .child(note(self.plugin_summary(&plugin), cx)),
            );
        }
        body.into_any_element()
    }

    fn plugin_summary(&self, plugin: &Plugin) -> String {
        format!(
            "{} · {} {} · {} MCP · {}",
            self.text(if plugin.id.starts_with("user:") {
                "앱 공통"
            } else {
                "현재 프로젝트"
            }),
            plugin.skills.len(),
            self.text("스킬"),
            plugin.manifest.mcp_servers.len(),
            self.text(if plugin.enabled { "활성화됨" } else { "비활성화됨" })
        )
    }

    fn plugin_detail(&self, plugin: Plugin, available: bool, cx: &Context<Self>) -> Div {
        let enabled = PluginEnabledRequest {
            id: plugin.id.clone(),
            enabled: !plugin.enabled,
        };
        let remove = PluginRemoveRequest {
            id: plugin.id.clone(),
        };
        let mut detail = settings_group()
            .border_color(cx.theme().border)
            .child(self.button("plugin-back", "목록으로", true, cx, |s, _, _| {
                s.plugins.selected = None;
                s.plugins.remove = false;
            }))
            .child(heading(plugin.manifest.display_name.as_deref().unwrap_or(&plugin.manifest.name).to_owned()))
            .child(note(plugin.manifest.description.clone(), cx))
            .child(note(self.plugin_summary(&plugin), cx))
            .child(note(format!("{} · {}", plugin.id, plugin.manifest.version), cx));
        for skill in plugin.skills {
            detail = detail.child(note(format!("{} · {}", skill.name, skill.description), cx));
        }
        for server in plugin.manifest.mcp_servers {
            let name = match server {
                | McpServer::Stdio {
                    name, ..
                }
                | McpServer::Http {
                    name, ..
                } => name,
            };
            detail = detail.child(note(format!("MCP · {name}"), cx));
        }
        detail = detail.child(
            row()
                .child(self.button(
                    "plugin-enabled",
                    if plugin.enabled { "비활성화" } else { "활성화" },
                    available,
                    cx,
                    move |s, _, _| {
                        let request = enabled.clone();
                        s.run_plugins(true, move |client| {
                            Ok(client
                                .response(Method::PUT, "/v1/plugins/enabled", Some(serde_json::to_value(request)?))?
                                .json()?)
                        });
                    },
                ))
                .child(self.button("plugin-connections", "MCP 연결", available, cx, |s, _, _| s.open_panel(Panel::Mcp)))
                .child(self.button("plugin-remove", "제거…", available, cx, |s, _, _| s.plugins.remove = true)),
        );
        if self.plugins.remove {
            detail = detail
                .child(note(self.text("이 설치된 플러그인을 제거할까요? 원본 폴더는 유지됩니다."), cx))
                .child(
                    row()
                        .child(self.button("plugin-remove-confirm", "제거 확인", available, cx, move |s, _, _| {
                            let request = remove.clone();
                            s.run_plugins(true, move |client| {
                                Ok(client
                                    .response(Method::POST, "/v1/plugins/remove", Some(serde_json::to_value(request)?))?
                                    .json()?)
                            });
                        }))
                        .child(self.button("plugin-remove-cancel", "취소", available, cx, |s, _, _| s.plugins.remove = false)),
                );
        }
        detail
    }
}
