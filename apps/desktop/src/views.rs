use crate::{app::{ChatApp,
                  HistoryDialog},
            ui::*};
use gpui::{self,
           assets::IconName,
           base::{InteractiveElementExt as _,
                  TestSupportExt as _},
           component::{FocusTrapElement as _,
                       Icon,
                       Theme,
                       ThemeMode,
                       menu::{DropdownMenu,
                              PopupMenuItem},
                       scroll::ScrollableElement},
           *};
use gpui_agent_contracts::*;
use gpui_agent_presentation::{Action as Command,
                              ScrollTarget};

#[derive(Clone)]
struct ResizeSidebar;
impl Render for ResizeSidebar {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

#[derive(Clone)]
pub(super) struct ResizeReference;
impl Render for ResizeReference {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

#[derive(Clone)]
pub(super) struct ResizeChatTool;
impl Render for ResizeChatTool {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

impl ChatApp {
    pub(super) fn save_sidebar_width(&mut self) {
        if let Some(path) = &self.language_path {
            let path = path.with_file_name("ui-sidebar-width");
            let result =
                std::fs::create_dir_all(path.parent().expect("preference directory")).and_then(|_| std::fs::write(path, self.sidebar_width.to_string()));
            if let Err(error) = result {
                self.conversations[self.selected].error = Some(format!("Cannot save sidebar width: {error}"));
            }
        }
    }

    pub(super) fn sidebar_width_for(&self, viewport: f32) -> f32 { self.sidebar_width.clamp(208., (viewport - 640.).max(208.)) }

    pub(super) fn set_search(&mut self, query: String, executor: &BackgroundExecutor) {
        for c in &mut self.conversations {
            c.search = query.clone();
        }
        self.search_changed_at = Some(std::time::Instant::now());
        let timer = executor.timer(std::time::Duration::from_millis(250));
        let wake = self.wake.clone();
        self.search_task = Some(executor.spawn(async move {
            timer.await;
            wake.notify();
        }));
    }

    pub(super) fn command(&mut self, action: Command) {
        let wake = self.wake.clone();
        self.start(action, &wake);
    }

    fn sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let i = self.selected;
        let has_workspace = !self.workspaces.loaded || self.workspaces.selected.is_some();
        let no_projects = self.workspaces.loaded && self.workspaces.catalog.projects.is_empty();
        let executor = cx.background_executor().clone();
        let search = self.field(
            format!("search-{i}"),
            "대화 검색",
            self.conversations[i].search.clone(),
            FieldKind::Line,
            has_workspace,
            window,
            cx,
            move |s, value| s.set_search(value, &executor),
        );
        // Flat owned rows: only the visible ones are built each frame.
        let mut rows: Vec<crate::history::SidebarRow> = Vec::new();
        if no_projects {
            rows.push(crate::history::SidebarRow::EmptyProjects);
        }
        if self.workspaces.catalog.projects.is_empty() && !self.workspaces.loaded {
            rows.push(crate::history::SidebarRow::Legacy);
        }
        let jobs = self.merged_history_jobs();
        for (project_index, project) in self.workspaces.catalog.projects.iter().enumerate() {
            for (position, (workspace_index, workspace)) in project.workspaces.iter().enumerate().filter(|(_, w)| !w.missing).enumerate() {
                rows.push(crate::history::SidebarRow::Workspace {
                    project: project_index,
                    workspace: workspace_index,
                    position,
                    status: self.job_monitor.workspace_status(workspace.id, self.language),
                });
                self.history_rows(workspace.id, &jobs, &mut rows);
            }
        }
        let drafts = self.conversations.iter().map(|c| c.drafts.len()).sum();
        let signatures: Vec<u64> = rows.iter().map(|row| row.signature(drafts)).collect();
        let timeline: std::rc::Rc<[TimelineEntry]> = std::rc::Rc::from([]);
        let state = self.sidebar_list.borrow_mut().prepare("sidebar".into(), signatures, timeline);
        *self.sidebar_rows.borrow_mut() = std::rc::Rc::new(rows);
        let projects = list(state, cx.processor(|s, index, _, cx| s.sidebar_row(index, cx))).size_full();
        let mut header = row()
            .h(px(52.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Gpui Agent"));
        if window.viewport_size().width < px(720.) {
            header = header.child(self.icon_button("sidebar-toggle", "사이드바 접기", IconName::PanelLeft, true, cx, |s, _, _| {
                s.sidebar_open = false
            }));
        }
        column()
            .id("app-sidebar")
            .test_support()
            .w_full()
            .flex_shrink_0()
            .h_full()
            .gap_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(header)
            .child(
                column().gap_1().px_2().pb_2().child(
                    row()
                        .flex_nowrap()
                        .gap_1()
                        .child(
                            self.icon_button("search", "검색", IconName::Search, !self.busy && has_workspace, cx, |s, _, _| {
                                s.command(Command::Search(s.conversations[s.selected].search.clone()));
                            }),
                        )
                        .child(div().flex_1().min_w_0().child(search))
                        .children((!self.conversations[i].search.is_empty()).then(|| {
                            self.icon_button("clear-search", "검색 지우기", IconName::X, true, cx, |s, window, cx| {
                                s.set_search(String::new(), cx.background_executor());
                                if !s.busy {
                                    s.command(Command::Search(String::new()));
                                }
                                if let Some(Field::Line {
                                    state, ..
                                }) = s.inputs.get(&format!("search-{}", s.selected))
                                {
                                    state.update(cx, |state, cx| state.focus(window, cx));
                                }
                            })
                        }))
                        .children((!no_projects).then(|| {
                            self.icon_button("project-add", "프로젝트 열기", IconName::FolderPlus, !self.busy, cx, |s, _, _| {
                                s.open_panel(Panel::Workspace);
                                s.command(Command::Projects);
                            })
                        }))
                        .child(self.icon_button("new-chat", "새 대화", IconName::SquarePen, !self.busy, cx, |s, window, cx| {
                            s.close_panel();
                            s.conversations[s.selected].new_chat();
                            if window.viewport_size().width < px(720.) {
                                s.sidebar_open = false;
                            }
                            s.focus.focus(window, cx);
                        })),
                ),
            )
            .child(div().id("history-scroll").test_support().flex_1().min_h_0().px_2().child(projects))
            .child(
                column()
                    .gap_2()
                    .flex_shrink_0()
                    .p_3()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(self.check(
                        "archived",
                        "보관된 대화",
                        self.conversations[i].show_archived,
                        !self.busy,
                        cx,
                        |s, value| {
                            for c in &mut s.conversations {
                                c.show_archived = value;
                            }
                        },
                    ))
                    .child(row().child(self.settings_menu(cx))),
            )
            .into_any_element()
    }

    fn sidebar_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        use crate::history::SidebarRow;
        let rows = self.sidebar_rows.borrow().clone();
        let Some(row_data) = rows.get(index) else {
            return Empty.into_any_element();
        };
        let body = match row_data {
            | SidebarRow::EmptyProjects => self
                .button("empty-open-project", "프로젝트 열기", !self.busy, cx, |s, _, _| {
                    s.open_panel(Panel::Workspace);
                    s.command(Command::Projects);
                })
                .icon(IconName::FolderPlus)
                .w_full()
                .into_any_element(),
            | SidebarRow::Legacy => column()
                .gap_1()
                .child(
                    row()
                        .px_2()
                        .py_1()
                        .child(Icon::new(IconName::Folder).small())
                        .child(note(self.project_name(), cx)),
                )
                .child(self.history_list(cx))
                .into_any_element(),
            | SidebarRow::Workspace {
                project,
                workspace,
                position,
                status,
            } => self.sidebar_workspace_row(*project, *workspace, *position, status, cx),
            | SidebarRow::Preamble {
                workspace,
                loading,
                error,
                empty,
                records,
            } => self.history_preamble_row(*workspace, *loading, error.as_deref(), *empty, *records, cx),
            | SidebarRow::Entry(entry) => self.history_entry_row(entry, cx),
        };
        div().w_full().pb_1().child(body).into_any_element()
    }

    fn sidebar_workspace_row(&self, project_index: usize, workspace_index: usize, position: usize, status: &str, cx: &Context<Self>) -> AnyElement {
        let Some(project) = self.workspaces.catalog.projects.get(project_index) else {
            return Empty.into_any_element();
        };
        let Some(workspace) = project.workspaces.get(workspace_index) else {
            return Empty.into_any_element();
        };
        let id = workspace.id;
        let active = self.workspaces.selected == Some(id);
        let select = self
            .sidebar_button(
                format!("workspace-{id}"),
                if workspace.main {
                    &project.name
                } else {
                    workspace.branch.as_deref().unwrap_or(&project.name)
                },
                if workspace.main { IconName::Folder } else { IconName::GitBranch },
                !self.busy,
                cx,
                move |s, window, _| {
                    let wake = s.wake.clone();
                    s.select_workspace(Some(id), &wake);
                    s.close_panel();
                    if window.viewport_size().width < px(720.) {
                        s.sidebar_open = false;
                    }
                },
            )
            .selected(active)
            .bg(gpui::transparent_black())
            .flex_1()
            .min_w_0();
        let mut line = row().group("project-row").flex_nowrap().gap_0().child(select);
        if position == 0 {
            line = line.child(self.project_menu(project, cx));
        }
        let mut block = column().gap_1().child(line);
        if !status.is_empty() {
            block = block.child(note(status.to_owned(), cx).pl_3());
        }
        block.into_any_element()
    }

    fn project_name(&self) -> String {
        self.workspaces
            .catalog
            .projects
            .iter()
            .find(|p| p.workspaces.iter().any(|w| Some(w.id) == self.workspaces.selected))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| {
                self.cwd
                    .trim_end_matches(['/', '\\'])
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&self.cwd)
                    .to_owned()
            })
    }

    pub(super) fn history_drafts(&self, cx: &Context<Self>) -> Div {
        let mut body = column().gap_1().pl_2();
        for (provider, conversation) in self.conversations.iter().enumerate() {
            for (index, draft) in conversation.drafts.iter().enumerate() {
                let id = draft.conversation_id;
                let preview = if draft.input.is_empty() {
                    draft.attachments.first().map_or("", Attachment::name)
                } else {
                    &draft.input
                };
                let label = format!("{} · {}", self.text("작성 중인 초안"), preview.chars().take(40).collect::<String>());
                body = body.child(self.sidebar_button(
                    format!("chat-draft-{provider}-{index}"),
                    &label,
                    IconName::SquarePen,
                    !self.busy,
                    cx,
                    move |s, window, _| {
                        s.selected = provider;
                        s.close_panel();
                        if let Some(id) = id {
                            s.command(Command::Load {
                                id,
                                message_id: None,
                            });
                        } else {
                            let conversation = &mut s.conversations[provider];
                            let draft = conversation.drafts.remove(index);
                            conversation.save_draft();
                            conversation.new_chat();
                            conversation.input = draft.input;
                            conversation.attachments = draft.attachments;
                            conversation.guidance.selected = draft.skills;
                            conversation.guidance.extensions = draft.extensions;
                        }
                        if window.viewport_size().width < px(720.) {
                            s.sidebar_open = false;
                        }
                    },
                ));
            }
        }
        body
    }

    fn history_list(&self, cx: &Context<Self>) -> Div {
        let mut entries: Vec<_> = self
            .conversations
            .iter()
            .enumerate()
            .flat_map(|(provider, c)| {
                c.history
                    .iter()
                    .filter(|h| h.archived == self.conversations[self.selected].show_archived)
                    .map(move |h| (provider, h))
            })
            .collect();
        entries.sort_by_key(|(_, c)| (std::cmp::Reverse(c.pinned), std::cmp::Reverse(c.updated_at), c.id));
        let mut body = self.history_drafts(cx);
        for (provider, conversation) in self.conversations.iter().enumerate().filter(|(provider, _)| *provider != self.selected) {
            if let Some(message) = &conversation.error {
                body = body.child(
                    row()
                        .child(error(format!("{}: {}", display_name(PROVIDERS[provider]), self.text(message)), cx).flex_1())
                        .child(
                            self.icon_button(format!("dismiss-sidebar-{provider}"), "닫기", IconName::X, true, cx, move |s, _, _| {
                                s.conversations[provider].error = None
                            }),
                        ),
                );
            }
        }
        if entries.is_empty() && !self.busy {
            body = body.child(
                note(
                    self.text(if self.conversations[self.selected].query.is_empty() {
                        "저장된 대화가 없습니다"
                    } else {
                        "검색 결과가 없습니다"
                    }),
                    cx,
                )
                .px_2()
                .py_2(),
            );
        }
        for (provider, c) in entries {
            let (id, hit) = (c.id, c.matched_message.as_ref().map(|m| m.message_id));
            let active = provider == self.selected && self.conversations[provider].id == Some(id) && self.panel.is_none();
            let mut item = column()
                .gap_0()
                .rounded(px(8.))
                .bg(if active { cx.theme().muted } else { gpui::transparent_black() })
                .child(
                    row()
                        .flex_nowrap()
                        .gap_0()
                        .child(
                            self.sidebar_button(
                                format!("history-{id}"),
                                &c.title,
                                if c.pinned { IconName::Pin } else { IconName::MessageSquare },
                                !self.busy,
                                cx,
                                move |s, window, _| {
                                    s.selected = provider;
                                    s.close_panel();
                                    s.command(Command::Load {
                                        id,
                                        message_id: hit,
                                    });
                                    if window.viewport_size().width < px(720.) {
                                        s.sidebar_open = false;
                                    }
                                },
                            )
                            .selected(active)
                            .bg(gpui::transparent_black())
                            .h_7()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden(),
                        )
                        .child(self.history_menu(provider, c, cx)),
                );
            if let Some(hit) = &c.matched_message {
                item = item.child(note(hit.preview.clone(), cx).px_2().pb_1().max_h(px(44.)).overflow_hidden());
            }
            body = body.child(item);
        }
        body
    }

    pub(super) fn project_menu(&self, project: &Project, cx: &Context<Self>) -> AnyElement {
        let id = project.id;
        let active = self
            .job_monitor
            .jobs
            .iter()
            .any(|job| !job.state.terminal() && project.workspaces.iter().any(|workspace| job.scope.workspace_id == Some(workspace.id)));
        let label = self.text(if active {
            "목록에서 제거 (진행 중인 작업 있음)"
        } else {
            "목록에서 제거"
        });
        let view = cx.weak_entity();
        let git = project.git;
        let add_label = self.text("이 프로젝트에 worktree 추가");
        let remove_label = self.text("worktree 제거");
        let worktrees: Vec<_> = project.workspaces.iter().filter(|w| w.managed && !w.main).cloned().collect();
        self.icon_button(format!("project-menu-{id}"), "프로젝트 메뉴", IconName::Ellipsis, !self.busy, cx, |_, _, _| {})
            .opacity(if self.panel == Some(Panel::Workspace) { 1. } else { 0. })
            .group_hover("project-row", |style| style.opacity(1.))
            .focus(|style| style.opacity(1.))
            .dropdown_caret(false)
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                if git {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(add_label.clone()).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |s, cx| {
                            if !s.busy {
                                s.open_panel(Panel::Workspace);
                                s.workspaces.project = Some(id);
                                s.workspaces.remove = None;
                                cx.notify();
                            }
                        });
                    }));
                }
                for worktree in &worktrees {
                    let view = view.clone();
                    let worktree = worktree.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{remove_label}: {}", worktree.branch.as_deref().unwrap_or(&worktree.path)))
                            .disabled(worktree.locked)
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |s, cx| {
                                    if !s.busy && !worktree.locked {
                                        s.open_panel(Panel::Workspace);
                                        s.workspaces.project = None;
                                        s.workspaces.remove = Some(worktree.clone());
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
                let forget_view = view.clone();
                menu.item(PopupMenuItem::new(label.clone()).disabled(active).on_click(move |_, _, cx| {
                    let _ = forget_view.update(cx, |s, cx| {
                        if !s.busy {
                            s.command(Command::ForgetProject(id));
                            cx.notify();
                        }
                    });
                }))
            })
            .into_any_element()
    }

    pub(super) fn history_menu(&self, provider: usize, c: &ConversationSummary, cx: &Context<Self>) -> AnyElement {
        let (id, title, pinned, archived) = (c.id, c.title.clone(), c.pinned, c.archived);
        // Reset the popup when its dialog takes focus.
        if self.dialog.is_some() {
            return self
                .icon_button(format!("history-menu-{id}"), "대화 메뉴", IconName::Ellipsis, false, cx, |_, _, _| {})
                .into_any_element();
        }
        let view = cx.weak_entity();
        let labels = [
            "이름 변경",
            if pinned { "고정 해제" } else { "대화 고정" },
            if archived { "보관 해제" } else { "대화 보관" },
            "참조로 열기",
            "삭제",
        ]
        .map(|label| self.text(label));
        self.icon_button(format!("history-menu-{id}"), "대화 메뉴", IconName::Ellipsis, !self.busy, cx, |_, _, _| {})
            .dropdown_caret(false)
            .dropdown_menu(move |mut menu, _, _| {
                for (index, label) in labels.iter().enumerate() {
                    let view = view.clone();
                    let title = title.clone();
                    if index == 4 {
                        menu = menu.separator();
                    }
                    menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| {
                        let _ = view.update(cx, |s, cx| {
                            if s.busy {
                                return;
                            }
                            let wake = s.wake.clone();
                            match index {
                                | 0 =>
                                    s.dialog = Some(HistoryDialog::Rename {
                                        provider,
                                        id,
                                        title: title.clone(),
                                    }),
                                | 1 => s.start_for(
                                    provider,
                                    Command::State {
                                        id,
                                        state: ConversationState {
                                            pinned: Some(!pinned),
                                            archived: None,
                                        },
                                    },
                                    &wake,
                                ),
                                | 2 => s.start_for(
                                    provider,
                                    Command::State {
                                        id,
                                        state: ConversationState {
                                            pinned: None,
                                            archived: Some(!archived),
                                        },
                                    },
                                    &wake,
                                ),
                                | 3 => s.start_for(
                                    provider,
                                    Command::Reference {
                                        id,
                                        title: title.clone(),
                                    },
                                    &wake,
                                ),
                                | _ =>
                                    s.dialog = Some(HistoryDialog::Delete {
                                        provider,
                                        id,
                                        title: title.clone(),
                                    }),
                            }
                            s.focus.focus(window, cx);
                            cx.notify();
                        });
                    }));
                }
                menu
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn provider_picker(&self, id: &'static str, cx: &Context<Self>) -> AnyElement {
        let view = cx.weak_entity();
        let selected = self.selected;
        self.button(id, display_name(PROVIDERS[selected]), !self.busy, cx, |_, _, _| {})
            .ghost()
            .icon(IconName::Bot)
            .dropdown_menu(move |mut menu, _, _| {
                for (index, provider) in PROVIDERS.into_iter().enumerate() {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(display_name(provider))
                            .checked(index == selected)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |s, cx| {
                                    if !s.busy {
                                        s.selected = index;
                                        s.command(Command::Refresh);
                                        s.focus.focus(window, cx);
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn settings_menu(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.weak_entity();
        let groups: Vec<_> = Panel::SETTINGS_GROUPS
            .iter()
            .map(|(label, panels)| (self.text(label), panels.iter().map(|p| (*p, self.text(p.nav_title()))).collect::<Vec<_>>()))
            .collect();
        self.button("settings-toggle", "설정", true, cx, |_, _, _| {})
            .ghost()
            .icon(IconName::Settings)
            .dropdown_menu(move |mut menu, _, _| {
                for (group, panels) in &groups {
                    menu = menu.label(group.clone());
                    for (panel, label) in panels {
                        let panel = *panel;
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(label.clone()).icon(panel.icon()).on_click(move |_, window, cx| {
                            let _ = view.update(cx, |s, cx| {
                                s.open_panel(panel);
                                s.focus.focus(window, cx);
                                cx.notify();
                            });
                        }));
                    }
                }
                menu
            })
            .into_any_element()
    }

    pub(super) fn work_panel(&self, cx: &Context<Self>) -> AnyElement {
        let conversation = &self.conversations[self.selected];
        // The merged tool calls only change when the timeline does; rebuilding
        // them per frame made every keystroke cost O(messages x events).
        let stamp = (self.timeline_stamp(), conversation.messages.len());
        let mut cache = self.work_cache.borrow_mut();
        if cache.as_ref().is_none_or(|cached| cached.stamp != stamp) {
            let timeline: Vec<_> = conversation
                .timeline
                .iter()
                .filter(|entry| {
                    !matches!(entry.event, Event::Verification { .. })
                        && (gpui_agent_presentation::is_work_event(&entry.event) || matches!(entry.event, Event::TurnEnd { .. }))
                })
                .map(|entry| TimelineEntry {
                    message_id: entry.message_id,
                    event: entry.event.clone(),
                })
                .collect();
            let tools = conversation
                .messages
                .iter()
                .enumerate()
                .filter(|(_, message)| !message.user)
                .map(|(n, message)| (n, gpui_agent_presentation::tools(&timeline, message.id.unwrap_or(-1))))
                .collect();
            *cache = Some(WorkCache {
                stamp,
                timeline,
                tools,
            });
        }
        let cached = cache.as_ref().expect("work cache was filled above");
        // Rows are virtualized: only the visible turns are built and laid out.
        let signatures = cached
            .tools
            .iter()
            .map(|(n, tools)| {
                use std::hash::{Hash as _,
                                Hasher as _};
                let key = format!("chat-{:?}-{}-{n}", self.client.workspace_id(), self.selected);
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for tool in tools {
                    let open = self
                        .expanded_tools
                        .contains(&format!("{key}-{}-{}", conversation.messages[*n].id.unwrap_or(-1), tool.id));
                    (&tool.id, &tool.status, open, tool.activity.len(), tool.output.is_some()).hash(&mut hasher);
                }
                tools.len().hash(&mut hasher);
                // Verification checks toggle independently of tools.
                self.expanded_tools.iter().filter(|k| k.starts_with("verification-")).count().hash(&mut hasher);
                conversation.timeline.len().hash(&mut hasher);
                hasher.finish()
            })
            .collect();
        let scope = format!("work-{:?}-{}-{:?}", self.client.workspace_id(), self.selected, conversation.id);
        let state = self.work_list.borrow_mut().prepare(scope, signatures, std::rc::Rc::from([]));
        drop(cache);
        let activity = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(list(state.clone(), cx.processor(|s, index, _, cx| s.work_row(index, cx))).size_full())
            .vertical_scrollbar(&state);
        column()
            .id("work-panel")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                row()
                    .h(px(44.))
                    .px_3()
                    .flex_shrink_0()
                    .child(div().flex_1().child(self.text("작업 내역")))
                    .child(self.icon_button(
                        "work-panel-maximize",
                        if self.work_maximized {
                            "작업 패널 복원"
                        } else {
                            "작업 패널 최대화"
                        },
                        if self.work_maximized { IconName::Minimize } else { IconName::Maximize },
                        true,
                        cx,
                        |s, _, _| {
                            s.work_maximized = !s.work_maximized;
                        },
                    ))
                    .child(self.icon_button("work-panel-close", "작업 패널 접기", IconName::X, true, cx, |s, _, _| {
                        s.work_maximized = false;
                        s.right_panels.work_open = false;
                        s.save_right_panels();
                    })),
            )
            .child(div().id("work-scroll").test_support().flex().flex_1().min_h_0().p_3().child(activity))
            .into_any_element()
    }

    fn work_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let conversation = &self.conversations[self.selected];
        let cache = self.work_cache.borrow();
        let Some(cached) = cache.as_ref() else {
            return Empty.into_any_element();
        };
        let Some((n, tools)) = cached.tools.get(index) else {
            return Empty.into_any_element();
        };
        let id = conversation.messages[*n].id.unwrap_or(-1);
        let key = format!("chat-{:?}-{}-{n}", self.client.workspace_id(), self.selected);
        column()
            .gap_4()
            .pb_4()
            .child(self.verification_view(&conversation.timeline, id, cx))
            .child(self.timeline_view_with(&key, &cached.timeline, id, tools, cx))
            .into_any_element()
    }

    /// While the pane edge is dragged, only its frame is laid out. Re-flowing
    /// every field of the PR/Git form on each pointer move made dragging lag;
    /// the content is drawn again when the drag ends.
    fn resize_outline(&self, panel: Panel, cx: &Context<Self>) -> AnyElement {
        column()
            .id("chat-tool-panel")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(row().h(px(44.)).px_4().flex_shrink_0().child(div().flex_1().child(self.text(panel.title()))))
            .into_any_element()
    }

    fn chat_tool_panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let content = self.panel_view(panel, window, cx);
        column()
            .id("chat-tool-panel")
            .test_support()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                row()
                    .h(px(44.))
                    .px_4()
                    .flex_shrink_0()
                    .child(div().flex_1().child(self.text(panel.title())))
                    .child(self.icon_button("chat-tool-close", "닫기", IconName::X, true, cx, |s, _, _| s.close_chat_tool())),
            )
            .child(
                column()
                    .id("chat-tool-scroll")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.chat_tool_scroll)
                    .vertical_scrollbar(&self.chat_tool_scroll)
                    .px_4()
                    .py_4()
                    .gap_4()
                    .child(note(self.text(panel.description()), cx))
                    .child(content),
            )
            .into_any_element()
    }

    fn model_picker(&self, cx: &Context<Self>) -> AnyElement {
        use gpui::component::menu::{DropdownMenu,
                                    PopupMenuItem};
        let index = self.selected;
        let c = &self.conversations[index];
        let workspace = self.client.workspace_id();
        let selected = c.model.clone();
        let mut models = vec![(String::new(), self.text("기본 모델"))];
        models.extend(c.models.iter().map(|m| (m.id.clone(), m.name.clone())));
        let label = models
            .iter()
            .find(|(id, _)| *id == selected)
            .map(|(_, name)| name.clone())
            .unwrap_or(selected.clone());
        let view = cx.weak_entity();
        self.button("model-picker", label, !self.busy, cx, |_, _, _| {})
            .ghost()
            .max_w(px(160.))
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name) in &models {
                    let id = id.clone();
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(selected == id).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |s, cx| {
                            if !s.busy && s.selected == index && s.client.workspace_id() == workspace {
                                s.conversations[index].model = id.clone();
                                cx.notify();
                            }
                        });
                    }));
                }
                menu
            })
            .into_any_element()
    }

    fn chat(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let i = self.selected;
        let c = &self.conversations[i];
        let logged = c.status.as_ref().is_some_and(|s| s.authenticated);
        let resumable = c.id.is_none() || c.session_id.is_some() || (self.busy && c.assistant.is_some());
        let enabled = logged && resumable && !self.busy && !self.accounts.busy(PROVIDERS[i], c.account_id.as_deref().unwrap_or("default"));
        let compact = window.viewport_size().height < px(520.);
        let empty = c.messages.is_empty();
        let hero = empty && !compact;
        let messages = self.messages_view(crate::messages::MessageSource::Chat, cx);
        if let Some(target) = self.conversations[i].scroll_to.take() {
            let lists = self.message_lists.borrow();
            let state = &lists[crate::messages::MessageSource::Chat as usize].state;
            match target {
                | ScrollTarget::Bottom => {
                    state.set_follow_mode(FollowMode::Tail);
                    state.scroll_to_end();
                },
                | ScrollTarget::Message(id) =>
                    if let Some(index) = self.conversations[i].messages.iter().position(|m| m.id == Some(id)) {
                        state.set_follow_mode(FollowMode::Normal);
                        state.scroll_to(ListOffset {
                            item_ix: index,
                            offset_in_item: px(0.),
                        });
                    },
            }
        }
        let mut body = column().id("chat-view").test_support().flex_1().min_w_0().min_h_0().gap_0();
        if hero {
            body = body.child(div().flex_1().min_h_0());
        } else {
            body = body.child(messages);
        }
        let mut lane = column().w_full().max_w(px(736.)).mx_auto().gap_2();
        if hero {
            lane = lane.child(
                column()
                    .items_center()
                    .gap_2()
                    .pb_4()
                    .child(div().text_2xl().child(self.text("무엇을 만들어 볼까요?")))
                    .child(
                        self.button("hero-project", self.project_name(), !self.busy, cx, |s, _, _| {
                            s.open_panel(Panel::Workspace);
                            s.command(Command::Projects);
                        })
                        .ghost()
                        .icon(IconName::Folder)
                        .self_center(),
                    ),
            );
        }
        if let Some(error_text) = &self.conversations[i].error {
            lane = lane.child(row().flex_shrink_0().child(error(self.text(error_text), cx)).child(self.icon_button(
                "dismiss-error",
                "닫기",
                IconName::X,
                true,
                cx,
                move |s, _, _| s.conversations[i].error = None,
            )));
        }
        if let Some((url, code)) = &self.conversations[i].authorization {
            let url = url.clone();
            lane = lane.child(self.button("authorization", "브라우저에서 로그인 계속하기", true, cx, move |_, _, cx| {
                cx.open_url(&url)
            }));
            if let Some(code) = code {
                lane = lane.child(note(code.clone(), cx));
            }
        }
        let mut composer = column()
            .id("composer-surface")
            .test_support()
            .gap_1()
            .p_3()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(24.));
        composer = composer.child(self.guidance_view(window, cx));
        if self.conversations[i].editing.is_some() {
            composer = composer
                .child(
                    row().child(note(self.text("마지막 질문 수정"), cx)).child(
                        self.button("edit-question-cancel", "취소", !self.busy, cx, |s, _, _| {
                            s.conversations[s.selected].editing = None;
                            if let Some(skills) = s.conversations[s.selected].skills_before_edit.take() {
                                s.conversations[s.selected].guidance.selected = skills;
                            }
                        })
                        .ghost(),
                    ),
                )
                .child(note(self.text("재질문해도 이전 실행의 파일 변경은 유지됩니다."), cx));
        }
        if self.conversations[i].editing.is_none() && !self.conversations[i].attachments.is_empty() {
            let mut attachments = column().gap_1();
            for (n, attachment) in self.conversations[i].attachments.iter().enumerate() {
                attachments = attachments.child(
                    self.button(
                        format!("draft-remove-{n}"),
                        format!("{} · {}", attachment.name(), self.text("제거")),
                        !self.busy,
                        cx,
                        move |s, _, _| {
                            s.conversations[i].attachments.remove(n);
                        },
                    )
                    .ghost()
                    .icon(IconName::X),
                );
            }
            composer = composer.child(
                div()
                    .id("draft-previews")
                    .max_h(px(if compact { 48. } else { 120. }))
                    .overflow_y_scroll()
                    .child(attachments)
                    .child(self.attachments_view("draft", &self.conversations[i].attachments, cx)),
            );
        }
        composer = composer.child(self.field(
            format!("composer-{:?}-{i}", self.client.workspace_id()),
            "메시지를 입력하세요",
            self.conversations[i].editing.clone().unwrap_or_else(|| self.conversations[i].input.clone()),
            FieldKind::Text,
            logged && resumable,
            window,
            cx,
            move |s, value| {
                if let Some(editing) = &mut s.conversations[i].editing {
                    *editing = value;
                } else {
                    s.conversations[i].input = value;
                }
            },
        ));
        if self.workspaces.focus_composer {
            self.workspaces.focus_composer = false;
            if let Some(Field::Text {
                state, ..
            }) = self.inputs.get(&format!("composer-{:?}-{i}", self.client.workspace_id()))
            {
                state.update(cx, |state, cx| state.focus(window, cx));
            }
        }
        let mut controls = row()
            .flex_nowrap()
            .gap_1()
            .child(self.icon_button(
                "attach",
                "파일 첨부",
                IconName::Plus,
                enabled && self.conversations[i].editing.is_none(),
                cx,
                |s, _, _| {
                    s.open_panel(Panel::Attach);
                },
            ))
            .child(self.provider_picker("provider-picker", cx))
            .child(self.model_picker(cx))
            .child(div().flex_1());
        if self.background_job.is_some() || self.operation.is_some() {
            controls = controls.child(self.icon_button("stop-operation", "중지", IconName::Square, true, cx, |s, _, _| s.stop_chat()));
        } else {
            let send = if self.conversations[i].editing.is_some() {
                self.button("send", "수정하여 다시 보내기", self.can_send(), cx, |s, _, _| {
                    let wake = s.wake.clone();
                    s.send(&wake);
                })
                .icon(IconName::ArrowUp)
            } else {
                self.icon_button("send", "보내기", IconName::ArrowUp, self.can_send(), cx, |s, _, _| {
                    let wake = s.wake.clone();
                    s.send(&wake);
                })
                .size_8()
                .rounded(px(999.))
                .bg(cx.theme().foreground)
                .text_color(cx.theme().background)
                .opacity(if self.can_send() { 1. } else { 0.4 })
            };
            controls = controls.child(send);
        }
        composer = composer.child(controls);
        let workspace = self.workspaces.selected_workspace();
        let branch = workspace.and_then(|w| w.branch.as_deref()).unwrap_or("—");
        let mut context = row()
            .id("composer-context")
            .test_support()
            .gap_1()
            .px_3()
            .py_1()
            .mx(px(22.))
            .bg(cx.theme().sidebar)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_b(px(16.))
            .child(
                self.button("composer-workspace", branch, !self.busy, cx, |s, _, _| {
                    s.open_panel(Panel::Workspace);
                    s.command(Command::Projects);
                })
                .ghost()
                .icon(IconName::GitBranch),
            )
            .child(div().flex_1());
        if PROVIDERS[i] == Provider::Claude {
            context = context.child(self.permission_modes(None, self.conversations[i].permission_mode, !self.busy, cx));
        }
        let mut footer = column().gap_0().child(composer);
        if PROVIDERS[i] == Provider::Codex && self.providers.iter().any(|p| p.provider == Provider::Codex && p.capabilities.background_jobs) {
            footer = footer.child(
                self.button(
                    "computer-submit",
                    "컴퓨터 작업 제출",
                    self.can_send() && self.conversations[i].editing.is_none(),
                    cx,
                    |s, _, _| {
                        s.send_task(&s.wake.clone(), true);
                    },
                )
                .outline(),
            );
        }
        if PROVIDERS[i] == Provider::Codex {
            footer = footer.child(self.codex_policy_picker(
                crate::codex_policy::PolicyTarget::Chat(i),
                self.conversations[i].codex_execution_policy,
                !self.busy,
                cx,
            ));
        }
        lane = lane.child(footer.children((!compact).then_some(context)));
        if self.providers.iter().any(|p| p.provider == PROVIDERS[i] && !p.available) {
            lane = lane.child(
                self.button("chat-runtime-settings", "런타임 설치", !self.busy, cx, |s, _, _| {
                    s.open_panel(Panel::Accounts)
                })
                .primary(),
            );
        } else if !logged {
            lane = lane.child(
                self.button("chat-login", "브라우저로 로그인", !self.busy, cx, |s, _, _| s.command(Command::Login))
                    .primary(),
            );
        }
        if !resumable {
            lane = lane.child(note(self.text("세션 생성 전에 중단된 대화입니다. 새 대화를 시작하세요."), cx));
        }
        if !compact && !empty {
            let mut report = self.cached_timeline_facts().usage;
            if report.usage.is_some() {
                report.missing_turns = self.conversations[i]
                    .messages
                    .iter()
                    .filter(|m| !m.user)
                    .count()
                    .saturating_sub(report.reported_turns);
                lane = lane.child(crate::usage::report_view(report, PROVIDERS[i], self.language, cx));
            }
        }
        body = body.child(div().flex_shrink_0().px_4().py_3().child(lane));
        if hero {
            body = body.child(div().flex_1().min_h_0());
        }
        body.into_any_element()
    }

    fn history_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mut body = column();
        let confirm;
        match &self.dialog {
            | Some(HistoryDialog::Rename {
                provider,
                id,
                title,
            }) => {
                let id = *id;
                let provider = *provider;
                let title = title.clone();
                body = body.child(heading(self.text("대화 제목 변경"))).child(self.field(
                    format!("rename-title-{id}"),
                    "새 제목",
                    title.clone(),
                    FieldKind::Line,
                    !self.busy,
                    window,
                    cx,
                    move |s, v| {
                        s.dialog = Some(HistoryDialog::Rename {
                            provider,
                            id,
                            title: v,
                        })
                    },
                ));
                let valid = !title.trim().is_empty() && title.chars().count() <= 100 && !title.chars().any(char::is_control);
                confirm = self.button("rename-confirm", "저장", valid && !self.busy, cx, move |s, _, _| {
                    if let Some(HistoryDialog::Rename {
                        provider,
                        id,
                        title,
                    }) = s.dialog.take()
                    {
                        let wake = s.wake.clone();
                        s.start_for(
                            provider,
                            Command::Rename {
                                id,
                                title: title.trim().into(),
                            },
                            &wake,
                        );
                    }
                });
            },
            | Some(HistoryDialog::Delete {
                provider,
                id,
                title,
            }) => {
                let id = *id;
                let provider = *provider;
                body = body
                    .child(heading(self.text("대화를 삭제할까요?")))
                    .child(title.clone())
                    .child(note(self.text("앱에 저장된 대화와 메시지를 영구 삭제합니다. 되돌릴 수 없습니다."), cx));
                confirm = self
                    .button("delete-confirm", "삭제 확인", !self.busy, cx, move |s, _, _| {
                        s.dialog = None;
                        let wake = s.wake.clone();
                        s.start_for(provider, Command::Delete(id), &wake);
                    })
                    .danger();
            },
            | None => return None,
        }
        Some(
            body.child(
                row()
                    .justify_end()
                    .pt_2()
                    .child(self.button("history-cancel", "취소", true, cx, |s, _, _| s.dialog = None).outline())
                    .child(confirm),
            )
            .into_any_element(),
        )
    }

    pub(super) fn panel_view(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match panel {
            | Panel::General => self.general_panel(cx),
            | Panel::Guidance | Panel::Skills => self.guidance_panel(window, cx),
            | Panel::Plugins => self.plugins_panel(window, cx),
            | Panel::Appearance => self.appearance_panel(cx),
            | Panel::Accounts => self.accounts_panel(window, cx),
            | Panel::Attach => self.attachment_panel(window, cx),
            | Panel::Workspace => self.workspace_panel(window, cx),
            | Panel::Git => self.git_panel(window, cx),
            | Panel::Pr => self.pr_panel(window, cx),
            | Panel::Mcp => self.mcp_panel(window, cx),
        }
    }
}

pub(super) struct WorkCache {
    stamp: (crate::panels::TimelineStamp, usize),
    timeline: Vec<TimelineEntry>,
    tools: Vec<(usize, Vec<ToolCall>)>,
}

impl Render for ChatApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.used_fields.clear();
        self.images.borrow_mut().retain(|id, _| self.used_images.borrow().contains(id));
        self.used_images.borrow_mut().clear();
        let width = f32::from(window.viewport_size().width);
        let no_projects = self.workspaces.loaded && self.workspaces.catalog.projects.is_empty();
        let conversation = &self.conversations[self.selected];
        let title = conversation
            .title
            .clone()
            .or_else(|| conversation.history.iter().find(|c| Some(c.id) == conversation.id).map(|c| c.title.clone()))
            .or_else(|| conversation.messages.iter().find(|m| m.user).map(|m| m.text.chars().take(100).collect()))
            .unwrap_or_else(|| self.text("새 대화"));
        let mut breadcrumb = row().flex_1().min_w_0().flex_nowrap().gap_1().child(self.icon_button(
            "sidebar-toggle",
            "대화 목록",
            IconName::PanelLeft,
            true,
            cx,
            |s, window, _| {
                if window.viewport_size().width < px(720.) {
                    s.close_chat_tool();
                }
                s.sidebar_open = !s.sidebar_open
            },
        ));
        if width >= 800. {
            breadcrumb = breadcrumb.child(note(self.project_name(), cx)).child(note("/", cx));
        }
        breadcrumb = breadcrumb.child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title));
        let mut actions = row()
            .flex_nowrap()
            .justify_end()
            .gap_1()
            .children(
                conversation
                    .timeline
                    .iter()
                    .any(|entry| gpui_agent_presentation::is_work_event(&entry.event))
                    .then(|| {
                        self.icon_button(
                            "work-toggle",
                            if self.busy { "작업 중" } else { "작업 내역" },
                            IconName::PanelRight,
                            true,
                            cx,
                            |s, _, _| {
                                s.right_panels.work_open = !s.right_panels.work_open || s.chat_tool.is_some();
                                if !s.right_panels.work_open {
                                    s.work_maximized = false;
                                }
                                s.close_chat_tool();
                                s.save_right_panels();
                            },
                        )
                        .selected(self.right_panels.work_open && self.chat_tool.is_none())
                        .toggled(self.right_panels.work_open && self.chat_tool.is_none())
                    }),
            )
            .child(self.icon_button("refresh-chat", "다시 확인", IconName::RefreshCw, !self.busy, cx, |s, _, _| {
                s.plugins.loaded = false;
                s.open_plugins();
                s.command(Command::Refresh)
            }))
            .child(
                self.icon_button("reference-toggle", "참고 대화", IconName::BookOpen, true, cx, |s, _, _| {
                    s.toggle_reference();
                })
                .selected(self.right_panels.reference_open && self.chat_tool.is_none())
                .toggled(self.right_panels.reference_open && self.chat_tool.is_none()),
            )
            .child(self.icon_button("chat-pr", "GitHub PR", IconName::GitPullRequest, true, cx, |s, _, _| s.open_panel(Panel::Pr)))
            .child(
                self.icon_button(
                    "chat-git",
                    "프로젝트 Git 작업",
                    IconName::GitBranch,
                    self.workspaces.selected.is_some(),
                    cx,
                    |s, _, _| s.open_panel(Panel::Git),
                )
                .selected(self.chat_tool == Some(Panel::Git)),
            )
            .child(self.icon_button(
                "theme-toggle",
                if cx.theme().mode == ThemeMode::Dark { "밝게" } else { "어둡게" },
                if cx.theme().mode == ThemeMode::Dark { IconName::Sun } else { IconName::Moon },
                true,
                cx,
                |_, window, cx| {
                    let mode = if cx.theme().mode == ThemeMode::Dark {
                        ThemeMode::Light
                    } else {
                        ThemeMode::Dark
                    };
                    Theme::change(mode, Some(window), cx);
                    apply_theme(cx);
                },
            ))
            .child(self.icon_button("language", "한국어 / English", IconName::Languages, true, cx, |s, _, _| {
                s.set_language(if s.language == crate::language::Language::Korean {
                    crate::language::Language::English
                } else {
                    crate::language::Language::Korean
                });
            }));
        let show_sidebar = if self.chat_tool.is_some() && width < 720. {
            false
        } else if self.panel.is_some() && width < 720. {
            self.settings_nav_open
        } else {
            self.sidebar_open
        };
        if !show_sidebar && self.panel.is_none() {
            if no_projects {
                actions = actions.child(
                    self.button("empty-open-project", "프로젝트 열기", !self.busy, cx, |s, _, _| {
                        s.open_panel(Panel::Workspace);
                        s.command(Command::Projects);
                    })
                    .icon(IconName::FolderPlus)
                    .flex_shrink_0(),
                );
            } else {
                actions = actions
                    .child(self.icon_button("new-chat", "새 대화", IconName::SquarePen, !self.busy, cx, |s, _, _| {
                        s.close_panel();
                        s.conversations[s.selected].new_chat();
                    }))
                    .child(self.settings_menu(cx));
            }
        }
        let header_width = width - if show_sidebar { self.sidebar_width_for(width) } else { 0. };
        let context_width = header_width - 24. - 2. * if show_sidebar { 240. } else { 360. };
        let inline_context = context_width >= 220.;
        let context = if self.panel.is_none() {
            self.conversation_context(if inline_context { context_width } else { header_width - 24. }, cx)
        } else {
            div()
        };
        let navigation = if inline_context {
            row()
                .min_h(px(52.))
                .child(breadcrumb)
                .child(context.flex_shrink_0())
                .child(actions.flex_1().min_w_0())
                .into_any_element()
        } else {
            column()
                .gap_0()
                .child(row().h(px(52.)).flex_nowrap().child(breadcrumb).child(actions))
                .child(context.self_center())
                .into_any_element()
        };
        let navigation = div()
            .id("chat-header")
            .test_support()
            .min_h(px(52.))
            .flex_shrink_0()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(navigation);
        let navigation = if let Some(panel) = self.panel {
            self.settings_header(panel, cx)
        } else {
            navigation.into_any_element()
        };
        let mut center = div().flex().flex_1().min_w_0().min_h_0();
        let available_width = width - if show_sidebar { self.sidebar_width_for(width) } else { 0. };
        let (tool_default, tool_min_width) = if self.chat_tool == Some(Panel::Git) { (360., 840.) } else { (480., 960.) };
        let tool_width = self.right_panels.tool_width_for(tool_default, available_width);
        if show_sidebar {
            let sidebar = if self.panel.is_some() {
                self.settings_sidebar(window, cx)
            } else {
                self.sidebar(window, cx)
            };
            center = center.child(
                div()
                    .relative()
                    .w(if width >= 720. {
                        px(self.sidebar_width_for(width))
                    } else {
                        window.viewport_size().width
                    })
                    .flex_shrink_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(sidebar)
                    .children((width >= 720.).then(|| {
                        gpui::component::resizable::resize_handle("sidebar-resize", Axis::Horizontal)
                            .inside(gpui::component::resizable::HandleEdge::Trailing)
                            .on_drag(ResizeSidebar, |drag, _, _, cx| cx.new(|_| drag.clone()))
                            .on_double_click(cx.listener(|s, _, _, cx| {
                                s.sidebar_width = 256.;
                                s.save_sidebar_width();
                                cx.notify();
                            }))
                    })),
            );
        }
        if !show_sidebar || width >= 720. {
            let mut main = column()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .gap_0()
                .child(navigation)
                .children(self.panel.is_none().then(|| self.git_proposal_notice(cx)).flatten());
            if self.work_maximized && self.panel.is_none() && self.chat_tool.is_none() && self.right_panels.work_open && self.has_work_activity() {
                main = main.child(self.work_panel(cx));
            } else if let Some(panel) = self.chat_tool.filter(|_| available_width < tool_min_width) {
                main = main.child(self.chat_tool_panel(panel, window, cx));
            } else {
                let content = if let Some(panel) = self.panel {
                    self.settings_page(panel, window, cx)
                } else if self.workspaces.loaded && self.workspaces.selected.is_none() {
                    column()
                        .id("empty-project-state")
                        .test_support()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .child(heading(self.text("프로젝트를 열어 시작하세요")))
                        .into_any_element()
                } else {
                    self.chat(window, cx)
                };
                let mut workspace = div().flex().flex_1().min_w_0().min_h_0().child(content);
                if let Some(panel) = self.chat_tool {
                    workspace = workspace.child(
                        div()
                            .relative()
                            .flex()
                            .w(px(tool_width))
                            .flex_shrink_0()
                            .child(if self.tool_resizing {
                                self.resize_outline(panel, cx)
                            } else {
                                self.chat_tool_panel(panel, window, cx)
                            })
                            .child(
                                gpui::component::resizable::resize_handle("chat-tool-resize", Axis::Horizontal)
                                    .inside(gpui::component::resizable::HandleEdge::Leading)
                                    .on_drag(ResizeChatTool, |drag, _, _, cx| cx.new(|_| drag.clone()))
                                    .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| s.finish_panel_resize()))
                                    .on_double_click(cx.listener(|s, _, _, cx| {
                                        s.right_panels.tool_width = None;
                                        s.save_right_panels();
                                        cx.notify();
                                    })),
                            ),
                    );
                } else if self.panel.is_none() && (self.right_panels.reference_open || (self.right_panels.work_open && self.has_work_activity())) {
                    let split = available_width >= 760.;
                    if !split && !self.right_panels.reference_open {
                        workspace = workspace.flex_col();
                    }
                    let panel = div().flex().min_h_0().min_w_0().flex_shrink_0().child(self.right_panel_stack(split, cx));
                    if split {
                        workspace = workspace.child(panel.w(px(self.right_panels.width_for(available_width))));
                    } else if !self.right_panels.reference_open {
                        workspace = workspace.child(panel.h(window.viewport_size().height * 0.3).w_full());
                    }
                }
                main = main.child(workspace);
            }
            center = center.child(main);
        }
        if !self.work_maximized && self.right_panels.reference_open && self.panel.is_none() && self.chat_tool.is_none() && available_width < 760. {
            center = div().flex().flex_1().min_h_0().child(self.right_panel_stack(false, cx));
        }
        let mut status = row()
            .px_3()
            .py_1()
            .flex_shrink_0()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                note(
                    format!(
                        "{} · {} {} · {} {} · {}",
                        self.text(if self.busy { self.activity } else { "준비됨" }),
                        self.text("실행 중"),
                        self.job_monitor.active_count(),
                        self.text("확인 대기"),
                        self.job_monitor.waiting_count(),
                        self.cwd
                    ),
                    cx,
                )
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis(),
            );
        if (self.background_job.is_some() || self.operation.is_some()) && (self.panel.is_some() || (self.sidebar_open && width < 720.)) {
            status = status.child(self.button("stop-operation", "중지", true, cx, |s, _, _| s.stop_chat()));
        }
        // Linux compositors may require client decorations even in Native mode.
        let titlebar = (self.titlebar_mode == crate::titlebar::TitlebarMode::Gpui
            || (cfg!(target_os = "linux") && matches!(window.window_decorations(), Decorations::Client { .. })))
        .then(|| {
            gpui::component::TitleBar::new()
                .bg(cx.theme().sidebar)
                .border_color(cx.theme().border)
                .child("Gpui Agent")
        });
        let mut root = column()
            .gap_0()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .font_family(cx.theme().font_family.clone())
            .children(titlebar)
            .track_focus(&self.focus)
            .on_drag_move(cx.listener(|s, event: &DragMoveEvent<ResizeSidebar>, window, cx| {
                let width = f32::from(event.event.position.x).clamp(208., (f32::from(window.viewport_size().width) - 640.).max(208.));
                if s.sidebar_width != width {
                    s.sidebar_resizing = true;
                    s.sidebar_width = width;
                    cx.notify();
                }
            }))
            .on_drag_move(cx.listener(|s, event: &DragMoveEvent<ResizeReference>, window, cx| {
                let width = f32::from(window.viewport_size().width);
                let available = width - if s.sidebar_open { s.sidebar_width_for(width) } else { 0. };
                let width = (width - f32::from(event.event.position.x)).clamp(280., (available - 480.).max(280.));
                if s.right_panels.width != width {
                    s.reference_resizing = true;
                    s.right_panels.width = width;
                    cx.notify();
                }
            }))
            .on_drag_move(cx.listener(|s, event: &DragMoveEvent<ResizeChatTool>, window, cx| {
                let width = f32::from(window.viewport_size().width);
                let available = width - if s.sidebar_open { s.sidebar_width_for(width) } else { 0. };
                let width = (width - f32::from(event.event.position.x)).clamp(280., (available - 480.).max(280.));
                if s.right_panels.tool_width != Some(width) {
                    s.tool_resizing = true;
                    s.right_panels.tool_width = Some(width);
                    cx.notify();
                }
            }))
            // Resize handles may consume mouse-up before it bubbles.
            .capture_any_mouse_up(
                cx.listener(|s, event: &MouseUpEvent, _, cx| {
                    if event.button != MouseButton::Left { return; }
                    if std::mem::take(&mut s.sidebar_resizing) {
                        s.save_sidebar_width();
                    }
                    // The pane content is drawn again once the drag ends.
                    let outlined = s.tool_resizing;
                    s.finish_panel_resize();
                    if outlined {
                        cx.notify();
                    }
                }),
            )
            .capture_action(cx.listener(|s, _: &gpui::component::input::Escape, window, cx| {
                if s.dialog.is_some() {
                    s.dialog = None;
                } else if s.chat_tool.is_some() {
                    s.close_chat_tool();
                } else if s.panel.is_some() {
                    if s.settings_nav_open {
                        s.settings_nav_open = false;
                    } else {
                        s.close_panel();
                    }
                } else {
                    return;
                }
                s.focus.focus(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .capture_action(cx.listener(|s, action: &gpui::component::input::Enter, window, cx| {
                if action.shift || s.panel.is_some() || s.dialog.is_some() || s.pending_target().is_some() {
                    return;
                }
                let key = format!("composer-{:?}-{}", s.client.workspace_id(), s.selected);
                if let Some(Field::Text {
                    state, ..
                }) = s.inputs.get(&key)
                    && state.focus_handle(cx).is_focused(window)
                    && s.can_send()
                    && state.update(cx, |state, cx| state.marked_text_range(window, cx).is_none())
                {
                    let wake = s.wake.clone();
                    s.send(&wake);
                    cx.stop_propagation();
                    cx.notify();
                } else if let Some(Field::Line {
                    state, ..
                }) = s.inputs.get(&format!("search-{}", s.selected))
                    && state.focus_handle(cx).is_focused(window)
                    && !s.busy
                    && state.update(cx, |state, cx| state.marked_text_range(window, cx).is_none())
                {
                    s.command(Command::Search(state.read(cx).value().to_string()));
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_key_down(cx.listener(|s, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "r" && e.keystroke.modifiers.shift && (e.keystroke.modifiers.control || e.keystroke.modifiers.platform) {
                    s.toggle_reference();
                    cx.stop_propagation();
                    cx.notify();
                }
                if e.keystroke.key == "l" && e.keystroke.modifiers.shift && (e.keystroke.modifiers.control || e.keystroke.modifiers.platform) {
                    s.set_language(if s.language == crate::language::Language::Korean {
                        crate::language::Language::English
                    } else {
                        crate::language::Language::Korean
                    });
                    cx.stop_propagation();
                    cx.notify();
                }
                if e.keystroke.key == "b" && (e.keystroke.modifiers.control || e.keystroke.modifiers.platform) {
                    if s.chat_tool.is_some() && window.viewport_size().width < px(720.) {
                        s.close_chat_tool();
                        s.sidebar_open = true;
                    } else if s.panel.is_some() && window.viewport_size().width < px(720.) {
                        s.settings_nav_open = !s.settings_nav_open;
                    } else {
                        s.sidebar_open = !s.sidebar_open;
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
                if e.keystroke.key == "escape" {
                    if s.chat_tool.is_some() {
                        s.close_chat_tool();
                    } else {
                        s.close_panel();
                    }
                    s.dialog = None;
                    s.focus.focus(window, cx);
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|s, paths: &ExternalPaths, _, cx| {
                if !s.busy && s.dialog.is_none() && s.conversations[s.selected].pending.is_none() {
                    if s.panel.is_some() {
                        return;
                    }
                    s.command(Command::AttachFiles(paths.0.to_vec()));
                    cx.notify();
                }
            }))
            .child(center)
            .children((self.busy || self.job_monitor.active_count() > 0 || self.job_monitor.waiting_count() > 0).then_some(status));
        let overlay = if let Some(target) = self.pending_target() {
            Some(self.interaction_panel(target, window, cx))
        } else {
            self.history_confirmation(window, cx)
        };
        if let Some(overlay) = overlay {
            let focus = self
                .modal_focus
                .get_or_insert_with(|| {
                    let focus = cx.focus_handle();
                    focus.focus(window, cx);
                    focus
                })
                .clone();
            let (role, label) = match &self.dialog {
                | Some(HistoryDialog::Delete {
                    ..
                }) => (Role::AlertDialog, "대화를 삭제할까요?"),
                | Some(HistoryDialog::Rename {
                    ..
                }) => (Role::Dialog, "대화 제목 변경"),
                | None => (Role::Dialog, "사용자 확인"),
            };
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(cx.theme().overlay)
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_3()
                    .occlude()
                    .child(
                        div()
                            .id("modal-scroll")
                            .test_support()
                            .role(role)
                            .aria_label(self.text(label))
                            .w_full()
                            .max_w(px(if self.dialog.is_some() { 512. } else { 640. }))
                            .max_h(window.viewport_size().height - px(24.))
                            .overflow_y_scroll()
                            .p_6()
                            .bg(cx.theme().popover)
                            .text_color(cx.theme().popover_foreground)
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(16.))
                            .shadow_lg()
                            .child(overlay)
                            .focus_trap("confirmation-dialog", &focus),
                    ),
            );
        } else if let Some(focus) = self.modal_focus.take()
            && focus.contains_focused(window, cx)
        {
            self.focus.focus(window, cx);
        }
        // A pane drawn as an outline while it is dragged builds no fields; its
        // inputs must survive until the real content is drawn again.
        if !self.tool_resizing {
            self.inputs.retain(|id, _| self.used_fields.contains(id));
        }
        root
    }
}

impl ChatApp {
    pub(super) fn can_send(&self) -> bool {
        let c = &self.conversations[self.selected];
        if c.editing.is_some() {
            return !self.busy
                && c.status.as_ref().is_some_and(|status| status.authenticated)
                && !self.accounts.busy(PROVIDERS[self.selected], c.account_id.as_deref().unwrap_or("default"))
                && c.revision_changed();
        }
        !self.busy
            && (!self.workspaces.loaded || self.workspaces.selected.is_some())
            && !self.accounts.busy(PROVIDERS[self.selected], c.account_id.as_deref().unwrap_or("default"))
            && c.status.as_ref().is_some_and(|s| s.authenticated)
            && (c.id.is_none() || c.session_id.is_some())
            && (!c.input.trim().is_empty() || !c.attachments.is_empty())
    }
}

/// A live job's placeholder reads as progress; only a stopped one is
/// "incomplete".
pub(super) fn incomplete_note(live: bool) -> &'static str { if live { "응답 생성 중…" } else { "완료되지 않은 응답" } }

#[cfg(test)]
mod tests {
    use super::incomplete_note;
    #[test]
    fn given_a_running_response_when_shown_then_it_is_not_labelled_incomplete() {
        assert_eq!(incomplete_note(true), "응답 생성 중…");
        assert_eq!(incomplete_note(false), "완료되지 않은 응답");
    }
}
