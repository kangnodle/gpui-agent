use crate::{app::ChatApp,
            content::markdown,
            ui::*};
use gpui::{assets::IconName,
           base::TestSupportExt as _,
           *};
use gpui_agent_presentation::{Action,
                              InstructionStatus,
                              guidance_display_path,
                              instruction_status,
                              skill_in_scope};
use gpui_kit::component::{Sizable as _,
                          popover::Popover,
                          tab::{Tab,
                                TabBar},
                          tag::Tag};

impl ChatApp {
    pub(super) fn guidance_editor_scope(&self) -> String { self.client.guidance_editor_scope() }

    pub(super) fn open_guidance_editor(&mut self) {
        if self.guidance_editor.begin(self.guidance_editor_scope()) {
            let wake = self.wake.clone();
            self.start(Action::GuidanceFiles, &wake);
        }
    }

    fn load_guidance_file(&mut self, id: String) {
        if !self.guidance_editor.dirty() {
            let wake = self.wake.clone();
            self.start(Action::ReadGuidance(id), &wake);
        }
    }

    pub(super) fn guidance_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let skills = self.panel == Some(Panel::Skills);
        let same_scope = self.guidance_editor.scope == self.guidance_editor_scope();
        let dirty = self.guidance_editor.dirty();
        let available = !self.busy && !dirty && same_scope;
        let user = self.guidance_editor.user;
        let has_project = self.workspaces.selected_workspace().is_some();
        let can_add = available && (user || has_project);
        let snapshot = self.conversations[self.selected].guidance.snapshot.as_ref();
        let user_count = self.guidance_editor.skill_count(snapshot, true);
        let project_count = self.guidance_editor.skill_count(snapshot, false);
        let mut body = column().gap_4();
        if let Some(message) = &self.workspaces.error {
            body = body.child(error(message.clone(), cx));
        }
        if let Some(snapshot) = &self.conversations[self.selected].guidance.snapshot {
            for diagnostic in &snapshot.diagnostics {
                body = body.child(note(format!("{} · {}", diagnostic.source, diagnostic.message), cx));
            }
        }
        if !same_scope {
            body = body.child(note(
                self.text("작업공간이 변경되었습니다. 초안을 복사하거나 편집 취소 후 다시 불러오세요."),
                cx,
            ));
        }
        if dirty {
            body = body.child(note(self.text("저장하지 않은 변경이 있습니다. 저장하거나 편집을 취소하세요."), cx));
        }
        if skills {
            let tabs = TabBar::new("skill-scope")
                .underline()
                .w_full()
                .suffix(
                    self.button("guidance-reload", "지침·스킬 새로고침", !self.busy && !dirty, cx, |s, _, _| {
                        s.open_guidance_editor()
                    })
                    .ghost(),
                )
                .selected_index(if user { 0 } else { 1 })
                .child(Tab::new().label(format!("{} · {user_count}", self.text("앱 공통"))).disabled(!available))
                .child(
                    Tab::new()
                        .label(format!("{} · {project_count}", self.text("현재 프로젝트")))
                        .disabled(!available),
                )
                .on_click(cx.listener(|s, index, _, cx| {
                    if !s.busy && !s.guidance_editor.dirty() && s.guidance_editor.scope == s.guidance_editor_scope() {
                        s.guidance_editor.select_scope(*index == 0);
                        cx.notify();
                    }
                }));
            body = body.child(div().id("skill-scope-tabs").test_support().w_full().child(tabs));
        } else {
            // Both scopes apply together, so show them side by side instead of
            // as mutually exclusive tabs.
            body = body
                .child(note(
                    self.text("두 지침이 모두 매 턴 적용됩니다. 앱 공통 지침 다음에 현재 프로젝트 지침이 이어집니다."),
                    cx,
                ))
                .child(
                    column()
                        .id("guidance-scopes")
                        .test_support()
                        .gap_1()
                        .child(self.instruction_scope_card(true, available, has_project, cx))
                        .child(note(format!("↓ {}", self.text("이어서 적용")), cx).px_3())
                        .child(self.instruction_scope_card(false, available, has_project, cx)),
                )
                .child(
                    row().justify_end().child(
                        self.button("guidance-reload", "지침·스킬 새로고침", !self.busy && !dirty, cx, |s, _, _| {
                            s.open_guidance_editor()
                        })
                        .icon(IconName::RefreshCw),
                    ),
                );
        }
        if skills {
            let project = self
                .workspaces
                .catalog
                .projects
                .iter()
                .find(|project| project.workspaces.iter().any(|workspace| Some(workspace.id) == self.workspaces.selected));
            body = body.child(note(
                if user {
                    self.text("앱 공통")
                } else {
                    format!(
                        "{} · {}",
                        self.text("현재 프로젝트"),
                        project.map_or(self.text("프로젝트 없음").as_ref(), |project| project.name.as_str())
                    )
                },
                cx,
            ));
            let description = self.text(if user {
                "모든 프로젝트에서 사용할 수 있습니다."
            } else if has_project {
                "이 프로젝트에서만 사용할 수 있습니다."
            } else {
                "프로젝트를 열면 프로젝트 스킬을 관리할 수 있습니다."
            });
            body = body.child(
                note(description.clone(), cx)
                    .id("skill-scope-description")
                    .test_support()
                    .aria_label(description),
            );
        }
        if skills && self.guidance_editor.file.is_none() {
            body = body.child(self.field(
                "skill-search",
                "이 범위에서 스킬 검색",
                self.guidance_editor.search.clone(),
                FieldKind::Line,
                !self.busy,
                window,
                cx,
                |s, value| s.guidance_editor.search = value,
            ));
            body = body.child(
                row()
                    .child(self.button(
                        "skill-folder-import",
                        if user {
                            "앱 공통에 폴더 가져오기…"
                        } else {
                            "이 프로젝트에 폴더 가져오기…"
                        },
                        can_add,
                        cx,
                        |s, _, cx| {
                            let scope = s.guidance_editor_scope();
                            let user = s.guidance_editor.user;
                            let prefix = s.guidance_editor.catalog.project_prefix.clone();
                            let picked = cx.prompt_for_paths(PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some("스킬 폴더 가져오기".into()),
                            });
                            cx.spawn(async move |view, cx| {
                                let picked = picked.await;
                                let _ = view.update(cx, |s, cx| {
                                    if scope != s.guidance_editor_scope() || user != s.guidance_editor.user || s.busy || s.guidance_editor.dirty() {
                                        return;
                                    }
                                    match picked {
                                        | Ok(Ok(Some(paths))) if paths.len() == 1 =>
                                            if let Some(name) = paths[0].file_name().and_then(|name| name.to_str()) {
                                                let id = if user {
                                                    format!("user:skills/{name}/SKILL.md")
                                                } else {
                                                    format!("repo:{prefix}.agents/skills/{name}/SKILL.md")
                                                };
                                                let wake = s.wake.clone();
                                                s.start(
                                                    Action::ImportSkillFolder {
                                                        path: paths[0].clone(),
                                                        id,
                                                    },
                                                    &wake,
                                                );
                                            },
                                        | Ok(Err(error)) => s.workspaces.error = Some(error.to_string()),
                                        | _ => {},
                                    }
                                    cx.notify();
                                });
                            })
                            .detach();
                        },
                    ))
                    .child(
                        self.button(
                            "skill-new",
                            if user {
                                "앱 공통 스킬 추가"
                            } else {
                                "이 프로젝트에 스킬 추가"
                            },
                            can_add,
                            cx,
                            |s, _, _| s.guidance_editor.creating = true,
                        )
                        .primary(),
                    ),
            );
            if self.guidance_editor.creating {
                body = body.child(self.field(
                    "guidance-name",
                    "스킬 이름",
                    self.guidance_editor.name.clone(),
                    FieldKind::Line,
                    available,
                    window,
                    cx,
                    |s, value| s.guidance_editor.name = value,
                ));
                body = body.child(self.field(
                    "skill-description",
                    "설명 · 사용할 상황",
                    self.guidance_editor.description.clone(),
                    FieldKind::Line,
                    available,
                    window,
                    cx,
                    |s, value| s.guidance_editor.description = value,
                ));
                body = body.child(self.button(
                    "guidance-new-skill",
                    "만들기",
                    can_add
                        && !self.guidance_editor.description.trim().is_empty()
                        && gpui_agent_contracts::validate_guidance_file_id(&self.guidance_editor.new_id(true)).is_ok(),
                    cx,
                    |s, _, _| s.load_guidance_file(s.guidance_editor.new_id(true)),
                ));
                body = body.child(self.button("skill-create-cancel", "생성 취소", available, cx, |s, _, _| {
                    s.guidance_editor.creating = false
                }));
            }
            if !user {
                for (number, proposal) in self.guidance_editor.proposals.clone().into_iter().enumerate() {
                    let title = format!("{} · {}", self.text("스킬 제안"), proposal.name);
                    let mut files = column().id(format!("skill-proposal-files-{number}")).test_support().gap_1();
                    for (path, text) in gpui_agent_presentation::skill_proposal_files(&proposal) {
                        files = files.child(heading(path)).child(note(text, cx));
                    }
                    let (approve, reject) = (proposal.id.clone(), proposal.id);
                    body = body.child(
                        column()
                            .id(format!("skill-proposal-{number}"))
                            .test_support()
                            .role(Role::Group)
                            .aria_label(title.clone())
                            .gap_1()
                            .child(heading(title))
                            .child(note(self.text("에이전트가 제안했습니다. 승인하기 전에는 설치되지 않습니다."), cx))
                            .child(
                                note(
                                    format!(
                                        "{}: {}",
                                        self.text("대화의 마지막 요청"),
                                        proposal.requested_during.clone().unwrap_or_else(|| "-".into())
                                    ),
                                    cx,
                                )
                                .id(format!("skill-proposal-request-{number}"))
                                .test_support(),
                            )
                            .child(
                                div()
                                    .id(format!("skill-proposal-scroll-{number}"))
                                    .max_h(px(240.))
                                    .overflow_y_scroll()
                                    .child(files),
                            )
                            .child(
                                row()
                                    .gap_1()
                                    .child(
                                        self.button(
                                            format!("skill-proposal-approve-{number}"),
                                            "승인하고 설치",
                                            available && has_project,
                                            cx,
                                            move |state, _, _| {
                                                let wake = state.wake.clone();
                                                state.start(
                                                    Action::DecideSkillProposal {
                                                        id: approve.clone(),
                                                        allow: true,
                                                    },
                                                    &wake,
                                                );
                                            },
                                        )
                                        .primary(),
                                    )
                                    .child(
                                        self.button(format!("skill-proposal-reject-{number}"), "거절", !self.busy, cx, move |state, _, _| {
                                            let wake = state.wake.clone();
                                            state.start(
                                                Action::DecideSkillProposal {
                                                    id: reject.clone(),
                                                    allow: false,
                                                },
                                                &wake,
                                            );
                                        }),
                                    ),
                            ),
                    );
                }
                for (number, proposal) in self.guidance_editor.mcp_proposals.clone().into_iter().enumerate() {
                    let title = format!("{} · {}", self.text("MCP 서버 제안"), proposal.server.name());
                    let mut facts = column().id(format!("mcp-proposal-facts-{number}")).test_support().gap_1();
                    for (label, value) in gpui_agent_presentation::mcp_proposal_facts(&proposal) {
                        facts = facts.child(note(format!("{}: {value}", self.text(label)), cx));
                    }
                    let (approve, reject) = (proposal.id.clone(), proposal.id);
                    body = body.child(
                        column()
                            .id(format!("mcp-proposal-{number}"))
                            .test_support()
                            .role(Role::Group)
                            .aria_label(title.clone())
                            .gap_1()
                            .child(heading(title))
                            .child(note(
                                self.text("에이전트가 MCP 서버를 제안했습니다. 승인하면 설치되고 활성화되어 다음 턴부터 시작됩니다."),
                                cx,
                            ))
                            .child(facts)
                            .child(
                                row()
                                    .gap_1()
                                    .child(
                                        self.button(
                                            format!("mcp-proposal-approve-{number}"),
                                            "승인하고 설치",
                                            available && has_project,
                                            cx,
                                            move |state, _, _| {
                                                let wake = state.wake.clone();
                                                state.plugins.loaded = false;
                                                state.start(
                                                    Action::DecideMcpProposal {
                                                        id: approve.clone(),
                                                        allow: true,
                                                    },
                                                    &wake,
                                                );
                                            },
                                        )
                                        .primary(),
                                    )
                                    .child(
                                        self.button(format!("mcp-proposal-reject-{number}"), "거절", !self.busy, cx, move |state, _, _| {
                                            let wake = state.wake.clone();
                                            state.start(
                                                Action::DecideMcpProposal {
                                                    id: reject.clone(),
                                                    allow: false,
                                                },
                                                &wake,
                                            );
                                        }),
                                    ),
                            ),
                    );
                }
                body = body.child(self.button("skill-proposals-refresh", "제안 새로 고침", available, cx, |s, _, _| {
                    let wake = s.wake.clone();
                    s.start(Action::GuidanceFiles, &wake);
                }));
            }
            let mut visible_skills = 0;
            if let Some(snapshot) = self.conversations[self.selected].guidance.snapshot.clone() {
                for (number, skill) in snapshot.skills.into_iter().enumerate() {
                    if !skill_in_scope(&skill.id, user) {
                        continue;
                    }
                    let query = self.guidance_editor.search.to_lowercase();
                    if !format!("{} {} {}", skill.name, skill.description, skill.id).to_lowercase().contains(&query) {
                        continue;
                    }
                    visible_skills += 1;
                    let packaged = skill.id.starts_with("plugin:");
                    let origin = if packaged {
                        "플러그인"
                    } else if skill.id.starts_with("user:") {
                        "앱 공통"
                    } else {
                        "현재 프로젝트"
                    };
                    let details_id = skill.id.clone();
                    let description = skill.description.clone();
                    body = body.child(
                        column()
                            .gap_1()
                            .child(
                                row()
                                    .w_full()
                                    .min_w_0()
                                    .child(
                                        note(format!("{} · {}", skill.display_name.as_deref().unwrap_or(&skill.name), skill.id), cx)
                                            .id(format!("skill-label-{number}"))
                                            .test_support()
                                            .max_w_full()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis(),
                                    )
                                    .child(self.button(
                                        format!("shared-skill-enabled-{number}"),
                                        if skill.enabled { "공통 사용 끄기" } else { "공통 사용 켜기" },
                                        !self.busy,
                                        cx,
                                        move |state, _, _| {
                                            let wake = state.wake.clone();
                                            state.start(
                                                Action::SetSkillEnabled {
                                                    id: skill.id.clone(),
                                                    enabled: !skill.enabled,
                                                },
                                                &wake,
                                            );
                                        },
                                    )),
                            )
                            .child(note(self.text(origin), cx))
                            .child(note(
                                self.text(if skill.allow_implicit_invocation {
                                    "자동 사용 허용 · 직접 선택 가능"
                                } else {
                                    "직접 선택으로 사용"
                                }),
                                cx,
                            ))
                            .child(self.button(
                                format!("skill-details-{number}"),
                                "자세히 · 편집",
                                available && !packaged,
                                cx,
                                move |s, _, _| s.load_guidance_file(details_id.clone()),
                            ))
                            .child(
                                div()
                                    .id(format!("skill-description-{number}"))
                                    .test_support()
                                    .aria_label(description.clone())
                                    .child(note(description, cx)),
                            ),
                    );
                }
            }
            if visible_skills == 0 {
                let message = if self.conversations[self.selected].guidance.snapshot.is_none() {
                    "스킬 목록을 확인 중입니다."
                } else if self
                    .guidance_editor
                    .skill_count(self.conversations[self.selected].guidance.snapshot.as_ref(), user)
                    > 0
                    && self.guidance_editor.search.is_empty()
                {
                    "아래 파일을 열어 스킬 내용을 확인하세요."
                } else if self.guidance_editor.search.is_empty() {
                    if user {
                        "앱 공통에 등록된 스킬이 없습니다."
                    } else {
                        "이 프로젝트에 등록된 스킬이 없습니다."
                    }
                } else {
                    "검색 결과가 없습니다."
                };
                let message = self.text(message);
                body = body.child(
                    div()
                        .id("skill-library-empty")
                        .test_support()
                        .role(Role::Status)
                        .aria_label(message.clone())
                        .child(note(message, cx)),
                );
            }
        }
        let mut files = column().gap_1();
        for (index, id) in self.guidance_editor.catalog.files.clone().into_iter().enumerate() {
            // Instruction files are listed inside their scope cards.
            if !skills || !id.ends_with("/SKILL.md") || !skill_in_scope(&id, user) {
                continue;
            }
            if self.guidance_editor.file.is_none()
                && self.conversations[self.selected]
                    .guidance
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.skills.iter().any(|skill| skill.id == id))
            {
                continue;
            }
            if !self.guidance_editor.search.is_empty() {
                let query = self.guidance_editor.search.to_lowercase();
                let matching = self.conversations[self.selected]
                    .guidance
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.skills.iter().find(|skill| skill.id == id));
                if !matching.map_or_else(
                    || id.to_lowercase().contains(&query),
                    |skill| format!("{} {} {}", skill.name, skill.description, skill.id).to_lowercase().contains(&query),
                ) {
                    continue;
                }
            }
            let selected = self.guidance_editor.file.as_ref().is_some_and(|file| file.id == id);
            files = files.child(
                self.choice(format!("guidance-file-{index}"), id.clone(), selected, available, cx, move |s, _, _| {
                    s.load_guidance_file(id.clone())
                }),
            );
        }
        if skills {
            body = body.child(div().id("guidance-files").max_h(px(160.)).overflow_y_scroll().child(files));
        }
        if let Some(file) = self.guidance_editor.file.clone() {
            let label = if skills {
                file.id.clone()
            } else {
                format!(
                    "{} · {} · {}",
                    self.text("편집 중"),
                    self.text(if skill_in_scope(&file.id, true) {
                        "앱 공통"
                    } else {
                        "현재 프로젝트"
                    }),
                    guidance_display_path(&file.id)
                )
            };
            body = body.child(note(label, cx).id("guidance-editing").test_support()).child(note(
                self.text(if skills {
                    "SKILL.md 64 KiB · UTF-8 Markdown"
                } else {
                    "AGENTS.md 32 KiB · UTF-8 Markdown"
                }),
                cx,
            ));
            if !skills {
                body = body.child(self.button(
                    "guidance-preview-toggle",
                    if self.guidance_preview { "미리보기 닫기" } else { "미리보기" },
                    true,
                    cx,
                    |s, _, _| s.guidance_preview = !s.guidance_preview,
                ));
            }
            let editor = self.field(
                if skills { "skill-guidance-text" } else { "guidance-text" },
                "Markdown 본문",
                self.guidance_editor.text.clone(),
                FieldKind::Text,
                !self.busy,
                window,
                cx,
                |s, value| {
                    s.guidance_editor.text = value;
                    s.guidance_editor.message.clear();
                },
            );
            let editor = div().id("guidance-text").test_support().w_full().child(editor);
            if !skills && self.guidance_preview {
                let preview = div()
                    .id("guidance-preview-content")
                    .test_support()
                    .w_full()
                    .child(markdown("guidance-preview-markdown", self.guidance_editor.text.clone()));
                let preview = column().min_w_0().w_full().gap_2().child(note(self.text("미리보기"), cx)).child(preview);
                body = if window.viewport_size().width >= px(1000.) {
                    body.child(row().flex_nowrap().items_start().child(editor.min_w_0()).child(preview))
                } else {
                    body.child(column().gap_4().child(editor).child(preview))
                };
            } else {
                body = body.child(editor);
            }
            let validation = self.guidance_editor.request().and_then(|request| request.validate().err());
            if let Some(message) = validation {
                body = body.child(error(message, cx));
            }
            body = body.child(
                row()
                    .child(self.button(
                        "guidance-import",
                        if dirty {
                            "초안을 교체하고 파일 불러오기…"
                        } else {
                            "Markdown 파일 불러오기…"
                        },
                        !self.busy && same_scope,
                        cx,
                        |s, _, cx| {
                            let scope = s.guidance_editor_scope();
                            let id = s.guidance_editor.file.as_ref().map(|file| file.id.clone());
                            let draft = s.guidance_editor.text.clone();
                            let picked = cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: false,
                                prompt: Some(s.text("Markdown 파일 불러오기…").into()),
                            });
                            cx.spawn(async move |view, cx| {
                                let picked = picked.await;
                                let _ = view.update(cx, |s, cx| {
                                    if scope == s.guidance_editor_scope()
                                        && id == s.guidance_editor.file.as_ref().map(|file| file.id.clone())
                                        && draft == s.guidance_editor.text
                                        && !s.busy
                                    {
                                        match picked {
                                            | Ok(Ok(Some(paths))) if paths.len() == 1 => {
                                                let wake = s.wake.clone();
                                                s.start(Action::ImportGuidance(paths[0].clone()), &wake);
                                            },
                                            | Ok(Err(error)) => s.workspaces.error = Some(error.to_string()),
                                            | _ => {},
                                        }
                                    }
                                    cx.notify();
                                });
                            })
                            .detach();
                        },
                    ))
                    .child(
                        self.button(
                            "guidance-save",
                            "저장",
                            !self.busy && same_scope && validation.is_none() && (dirty || file.revision.is_none()),
                            cx,
                            |s, _, _| {
                                if let Some(request) = s.guidance_editor.request() {
                                    let wake = s.wake.clone();
                                    s.start(Action::WriteGuidance(request), &wake);
                                }
                            },
                        )
                        .primary(),
                    )
                    .child(self.button("guidance-discard", "편집 취소", !self.busy, cx, |s, _, _| {
                        s.guidance_editor.file = None;
                        s.guidance_editor.text.clear();
                        s.guidance_editor.message.clear();
                        s.open_guidance_editor();
                    })),
            );
        }
        body = body.child(note(self.guidance_editor.message.clone(), cx));
        if skills {
            return body.into_any_element();
        }
        self.settings_section("지침", body, cx).into_any_element()
    }

    /// One AGENTS.md scope: what it covers, whether it reaches the next turn,
    /// and its files.
    fn instruction_scope_card(&self, user: bool, available: bool, has_project: bool, cx: &Context<Self>) -> AnyElement {
        let (key, title, icon, description) = if user {
            ("user", "앱 공통", IconName::Globe, "앱 공통 지침은 모든 프로젝트에 적용됩니다.")
        } else {
            ("project", "현재 프로젝트", IconName::Folder, "프로젝트 지침은 현재 프로젝트에 적용됩니다.")
        };
        let snapshot = self.conversations[self.selected].guidance.snapshot.as_ref();
        let status = instruction_status(snapshot, user);
        let can_create = user || has_project;
        let tag = match status {
            | InstructionStatus::Applied => Tag::success(),
            | InstructionStatus::Truncated => Tag::warning(),
            | InstructionStatus::Problem => Tag::danger(),
            | InstructionStatus::Checking | InstructionStatus::Missing => Tag::secondary(),
        }
        .small();
        let status_label = self.text(if !can_create && status == InstructionStatus::Missing {
            "프로젝트 없음"
        } else {
            status.label()
        });
        let tag = div()
            .id(format!("guidance-status-{key}"))
            .test_support()
            .role(Role::Status)
            .aria_label(status_label.clone())
            .child(tag.child(status_label.clone()));
        let title = self.text(title);
        let open = self.guidance_editor.file.as_ref().map(|file| file.id.clone());
        let mut files = column().gap_1();
        let mut has_files = false;
        for (index, id) in self.guidance_editor.catalog.files.iter().enumerate() {
            if id.ends_with("/SKILL.md") || !skill_in_scope(id, user) {
                continue;
            }
            has_files = true;
            let target = id.clone();
            files = files.child(
                self.choice(
                    format!("guidance-file-{index}"),
                    guidance_display_path(id),
                    open.as_deref() == Some(id.as_str()),
                    available,
                    cx,
                    move |s, _, _| {
                        if s.guidance_editor.select_scope(user) {
                            s.load_guidance_file(target.clone());
                        }
                    },
                )
                .icon(IconName::BookOpen),
            );
        }
        if !has_files {
            files = if can_create {
                files.child(
                    self.button(format!("guidance-new-agents-{key}"), "AGENTS.md 만들기", available, cx, move |s, _, _| {
                        if s.guidance_editor.select_scope(user) {
                            s.load_guidance_file(s.guidance_editor.new_id(false));
                        }
                    })
                    .icon(IconName::Plus),
                )
            } else {
                files.child(note(self.text("프로젝트를 열면 프로젝트 지침을 만들 수 있습니다."), cx))
            };
        }
        column()
            .id(format!("guidance-scope-{key}"))
            .test_support()
            .role(Role::Group)
            .aria_label(format!("{title} · {status_label}"))
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                row()
                    .flex_nowrap()
                    .child(gpui::component::Icon::new(icon).small())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(tag),
            )
            .child(note(self.text(description), cx))
            .child(files)
            .into_any_element()
    }

    pub(super) fn conversation_context(&mut self, max_width: f32, cx: &mut Context<Self>) -> Div {
        if self.conversations[self.selected].guidance.snapshot.is_some()
            && (self.plugins.scope != self.client.guidance_editor_scope() || self.plugins.error.is_none())
        {
            self.open_plugins();
        }
        let snapshot = self.conversations[self.selected].guidance.snapshot.as_ref();
        let count = snapshot.map_or(0, |snapshot| snapshot.instructions.len());
        let label = match snapshot {
            | None => self.text("지침 확인 필요"),
            | Some(snapshot) if !snapshot.diagnostics.is_empty() => self.text("지침 확인 필요 · 진단 있음"),
            | Some(_) if count == 0 => self.text("지침 설정"),
            | Some(_) => match self.language {
                | crate::language::Language::Korean => format!("지침 {count}개"),
                | crate::language::Language::English => format!("Instructions · {count}"),
            },
        };
        let trigger = self
            .button("guidance-status", label, count > 0 || !self.busy, cx, move |s, _, _| {
                if count == 0 {
                    s.open_panel(Panel::Guidance);
                }
            })
            .icon(if count == 0 { IconName::Plus } else { IconName::BookOpen });
        let instruction = if count == 0 {
            trigger.into_any_element()
        } else {
            let view = cx.weak_entity();
            let scope = gpui_agent_presentation::guidance_scope(self.client.workspace_id(), self.conversations[self.selected].account_id.as_deref());
            Popover::new(ElementId::Name(format!("conversation-instructions-{}-{scope}", self.selected).into()))
                .trigger(trigger)
                .content(move |_, window, cx| {
                    let popup = cx.entity().downgrade();
                    let close_popup = popup.clone();
                    view.update(cx, |s, cx| {
                        let mut detail = column()
                            .id("conversation-instructions")
                            .test_support()
                            .gap_2()
                            .w(px(320.))
                            .max_w(window.viewport_size().width - px(40.))
                            .child(row().justify_between().child(heading(s.text("지침"))).child(s.icon_button(
                                "conversation-guidance-close",
                                "닫기",
                                IconName::X,
                                true,
                                cx,
                                move |_, window, cx| {
                                    let _ = close_popup.update(cx, |state, cx| state.dismiss(window, cx));
                                },
                            )));
                        let mut documents = column().gap_3();
                        if let Some(snapshot) = &s.conversations[s.selected].guidance.snapshot {
                            for (index, doc) in snapshot.instructions.iter().enumerate() {
                                documents = documents.child(
                                    column()
                                        .child(
                                            div()
                                                .id(format!("conversation-instruction-{index}"))
                                                .test_support()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .role(Role::Group)
                                                .aria_label(doc.source.clone())
                                                .child(note(doc.source.clone(), cx)),
                                        )
                                        .child(
                                            div()
                                                .id(format!("conversation-instruction-content-{index}"))
                                                .test_support()
                                                .child(markdown(format!("conversation-instruction-markdown-{index}"), doc.content.clone())),
                                        ),
                                );
                            }
                        }
                        detail = detail.child(
                            div()
                                .id("conversation-instruction-list")
                                .max_h(px((f32::from(window.viewport_size().height) - 200.).clamp(80., 280.)))
                                .overflow_y_scroll()
                                .child(documents),
                        );
                        detail
                            .child(s.button("conversation-guidance-manage", "지침 설정", !s.busy, cx, move |s, window, cx| {
                                let _ = popup.update(cx, |state, cx| state.dismiss(window, cx));
                                s.open_panel(Panel::Guidance);
                            }))
                            .into_any_element()
                    })
                    .unwrap_or_else(|_| Empty.into_any_element())
                })
                .into_any_element()
        };
        div().w(px(max_width)).min_w_0().child(
            row()
                .id("conversation-context")
                .test_support()
                .justify_center()
                .flex_wrap()
                .w_full()
                .max_h(px(120.))
                .overflow_y_scroll()
                .gap_2()
                .py_2()
                .child(instruction)
                .children(self.conversation_plugins(cx)),
        )
    }

    pub(super) fn guidance_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let index = self.selected;
        let snapshot = self.conversations[index].guidance.snapshot.clone();
        let mut view = column().gap_1();
        // shortcut: match the draft suffix until the composer exposes caret
        // context.
        let suffix = self.conversations[index]
            .input
            .split_whitespace()
            .last()
            .filter(|token| token.starts_with('$') && self.conversations[index].input.ends_with(token))
            .map(str::to_owned);
        if self.conversations[index].guidance.open || suffix.is_some() {
            if self.plugins.error.is_none() {
                self.open_plugins();
            }
            let message = self.text(match &snapshot {
                | None => "지침·스킬 확인 중…",
                | Some(_) if !self.conversations[index].guidance.selected.is_empty() => "선택한 스킬은 이 대화의 다음 턴에도 적용됩니다.",
                | Some(snapshot) if !snapshot.skills.iter().any(|skill| skill.enabled) => "사용할 수 있는 스킬이 없습니다.",
                | Some(_) => "선택한 스킬이 없습니다. 현재 턴에 적용할 스킬을 선택하세요.",
            });
            let mut entries = column().gap_1().child(
                div()
                    .id("shared-guidance-message")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(message.clone())
                    .child(note(message, cx)),
            );
            if let Some(snapshot) = snapshot {
                for (group, numbers) in gpui_agent_presentation::skill_groups(&snapshot.skills) {
                    let title = if group.is_empty() {
                        self.text("개별 스킬")
                    } else {
                        let plugin = self.plugins.catalog.plugins.iter().find(|plugin| plugin.id == group);
                        plugin
                            .map(|plugin| format!("{} · {group}", plugin.manifest.display_name.as_deref().unwrap_or(&plugin.manifest.name)))
                            .unwrap_or_else(|| group.to_owned())
                    };
                    let mut group_view = column()
                        .id(format!("shared-skill-group-{group}"))
                        .test_support()
                        .role(Role::Group)
                        .aria_label(title.clone())
                        .gap_1()
                        .child(heading(title));
                    let mut visible = false;
                    for number in numbers {
                        let skill = &snapshot.skills[number];
                        if let Some(token) = &suffix
                            && (!skill.enabled
                                || !format!("{} {}", skill.name, skill.description)
                                    .to_lowercase()
                                    .contains(&token[1 ..].to_lowercase()))
                        {
                            continue;
                        }
                        let selected = self.conversations[index].guidance.selected.iter().any(|reference| reference.id == skill.id);
                        let id = skill.id.clone();
                        let token = suffix.clone();
                        visible = true;
                        group_view = group_view
                            .child(self.choice(
                                format!("shared-skill-{number}"),
                                format!("{} · {}", skill.display_name.as_deref().unwrap_or(&skill.name), skill.id),
                                selected,
                                !self.busy && (skill.enabled || selected),
                                cx,
                                move |state, _, _| {
                                    let result = if token.is_some() && state.conversations[index].guidance.selected.iter().any(|reference| reference.id == id) {
                                        Ok(())
                                    } else {
                                        state.conversations[index].guidance.toggle(&id)
                                    };
                                    if let Err(error) = result {
                                        state.conversations[index].error = Some(error.into());
                                    } else if let Some(token) = &token {
                                        let input = &mut state.conversations[index].input;
                                        if input.ends_with(token) {
                                            input.truncate(input.len() - token.len());
                                        }
                                    }
                                },
                            ))
                            .child(note(skill.description.clone(), cx));
                    }
                    if visible {
                        entries = entries.child(group_view);
                    }
                }
            }
            view = view.child(
                div()
                    .id("shared-guidance-list")
                    .test_support()
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .child(entries),
            );
        }
        view
    }
}
