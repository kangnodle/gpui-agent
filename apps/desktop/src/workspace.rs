use gpui_agent_contracts::{FileDiff,
                           GitChanges,
                           ProjectCatalog,
                           Workspace};
use gpui_agent_presentation::{Action,
                              Conversation};
use std::{collections::{HashMap,
                        HashSet},
          path::PathBuf};
#[derive(Default)]
pub struct WorkspaceUi {
    pub(super) verification_drafts: HashMap<i64, gpui_agent_presentation::VerificationDraft>,
    pub(super) verification_advanced: HashSet<i64>,
    pub git: crate::checkpoint::GitUi,
    pub catalog: ProjectCatalog,
    pub loaded: bool,
    pub selected: Option<i64>,
    pub open: bool,
    pub(super) focus_composer: bool,
    pub diff_open: bool,
    pub next_selection: Option<Option<i64>>,
    pub cached: HashMap<Option<i64>, [Conversation; gpui_agent_contracts::PROVIDERS.len()]>,
    pub providers: HashMap<Option<i64>, usize>,
    pub error: Option<String>,
    pub(super) path: String,
    pub(super) branch: String,
    pub(super) base: String,
    pub(super) project: Option<i64>,
    pub(super) remove: Option<Workspace>,
    pub changes: Option<GitChanges>,
    pub diff: Option<FileDiff>,
    pub(super) diff_views: HashMap<&'static str, crate::content::DiffPreview>,
}
impl WorkspaceUi {
    pub(super) fn apply_picked_folder(&mut self, picked: Option<PathBuf>) -> Option<Action> {
        if let Some(path) = picked {
            match path.into_os_string().into_string() {
                | Ok(path) => {
                    self.path = path;
                    self.error = None;
                    return Some(Action::OpenProject(self.path.clone()));
                },
                | Err(_) => self.error = Some("선택한 폴더 경로를 UTF-8로 표시할 수 없습니다.".into()),
            }
        }
        None
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        let id = self.selected?;
        self.catalog
            .projects
            .iter()
            .flat_map(|project| &project.workspaces)
            .find(|workspace| workspace.id == id)
    }

    pub fn apply(&mut self, catalog: ProjectCatalog) {
        let first_catalog = !self.loaded;
        self.loaded = true;
        let old_project = self.selected_workspace().map(|workspace| workspace.project_id);
        let selected_exists = self.selected.is_some_and(|id| {
            catalog
                .projects
                .iter()
                .flat_map(|project| &project.workspaces)
                .any(|workspace| workspace.id == id && !workspace.missing)
        });
        if self.selected.is_some() && !selected_exists {
            self.next_selection = Some(
                old_project
                    .and_then(|id| catalog.projects.iter().find(|project| project.id == id))
                    .and_then(|project| project.workspaces.iter().find(|workspace| workspace.main && !workspace.missing))
                    .map(|workspace| workspace.id)
                    .or(catalog.default_workspace_id),
            );
        }
        if self.selected.is_none() {
            if first_catalog && catalog.default_workspace_id.is_some() {
                self.selected = catalog.default_workspace_id;
            } else {
                self.next_selection = Some(catalog.default_workspace_id);
            }
        }
        self.catalog = catalog;
    }
}

use crate::{app::ChatApp,
            ui::*};
use gpui::{self,
           assets::IconName,
           base::TestSupportExt as _,
           *};
use gpui_agent_contracts::ProjectVerification;
use gpui_agent_presentation::VerificationDraft;
use gpui_kit::component::tag::Tag;
impl ChatApp {
    /// Required checks the host runs before a job may complete.
    fn verification_section(&mut self, pid: i64, saved: &ProjectVerification, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let draft = self
            .workspaces
            .verification_drafts
            .entry(pid)
            .or_insert_with(|| VerificationDraft::from_plan(saved))
            .clone();
        let changed = draft.changes(saved);
        let enabled = !self.busy;
        let count = saved.commands.len();
        let status = if count == 0 {
            self.text("꺼짐")
        } else {
            match self.language {
                | crate::language::Language::Korean => format!("켜짐 · 명령 {count}개"),
                | crate::language::Language::English => format!("On · {count} command{}", if count == 1 { "" } else { "s" }),
            }
        };
        let badge = div()
            .id(format!("project-verification-status-{pid}"))
            .test_support()
            .role(Role::Status)
            .aria_label(status.clone())
            .child(if count == 0 { Tag::secondary() } else { Tag::success() }.small().child(status));
        let step = |text: &str, cx: &App| {
            div()
                .px_2()
                .py_1()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(self.text(text))
        };
        let arrow = || gpui::component::Icon::new(IconName::ArrowRight).xsmall();
        let flow = row()
            .gap_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(step("에이전트 작업", cx))
            .child(arrow())
            .child(step("명령 실행", cx))
            .child(arrow())
            .child(step("모두 성공 → 완료", cx))
            .child(self.text("또는"))
            .child(step(
                &match self.language {
                    | crate::language::Language::Korean => format!("실패 → 다시 수정 (최대 {}회)", saved.max_attempts),
                    | crate::language::Language::English => format!("Failed → fix again (up to {})", saved.max_attempts),
                },
                cx,
            ));
        let mut commands = column().id(format!("project-verification-commands-{pid}")).test_support().gap_1();
        for (index, line) in draft.commands.iter().enumerate() {
            let label = match self.language {
                | crate::language::Language::Korean => format!("명령 {}", index + 1),
                | crate::language::Language::English => format!("Command {}", index + 1),
            };
            commands = commands.child(
                row()
                    .flex_nowrap()
                    .child(
                        div()
                            .id(format!("verification-command-{pid}-{index}"))
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .child(self.field(
                                format!("verification-command-{pid}-{index}"),
                                &label,
                                line.clone(),
                                FieldKind::Line,
                                enabled,
                                window,
                                cx,
                                move |s, value| {
                                    if let Some(line) = s.workspaces.verification_drafts.get_mut(&pid).and_then(|draft| draft.commands.get_mut(index)) {
                                        *line = value;
                                    }
                                },
                            )),
                    )
                    .child(self.icon_button(
                        format!("verification-command-remove-{pid}-{index}"),
                        "명령 삭제",
                        IconName::X,
                        enabled,
                        cx,
                        move |s, _, _| {
                            if let Some(draft) = s.workspaces.verification_drafts.get_mut(&pid)
                                && index < draft.commands.len()
                            {
                                draft.commands.remove(index);
                            }
                        },
                    )),
            );
        }
        let advanced_open = self.workspaces.verification_advanced.contains(&pid);
        let advanced_label = match self.language {
            | crate::language::Language::Korean => format!("고급 설정 · 최대 {}회 시도 · 명령당 {}초", saved.max_attempts, saved.timeout_seconds),
            | crate::language::Language::English => format!("Advanced · up to {} attempts · {}s per command", saved.max_attempts, saved.timeout_seconds),
        };
        let mut section = column()
            .id(format!("project-verification-{pid}"))
            .test_support()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                row()
                    .flex_nowrap()
                    .child(gpui::component::Icon::new(IconName::CircleCheck).small())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(self.text("완료 전 필수 검증")))
                    .child(badge),
            )
            .child(note(
                self.text("에이전트가 작업을 마칠 때마다 앱이 아래 명령을 이 프로젝트 폴더에서 직접 실행합니다. 모두 성공해야 작업이 완료되고, 실패하면 결과를 에이전트에게 보내 다시 고치게 합니다."),
                cx,
            ))
            .child(flow)
            .child(note(self.text("실행할 명령 · 위에서부터 순서대로"), cx))
            .child(commands)
            .child(
                self.button(format!("verification-command-add-{pid}"), "명령 추가", enabled, cx, move |s, _, _| {
                    if let Some(draft) = s.workspaces.verification_drafts.get_mut(&pid) {
                        draft.commands.push(String::new());
                    }
                })
                .icon(IconName::Plus)
                .ghost(),
            )
            .child(note(
                self.text("셸 없이 실행하므로 &&, 파이프, 리디렉션은 쓸 수 없습니다. 공백이 든 인자는 따옴표로 감싸세요. 명령을 모두 지우면 검증이 꺼집니다."),
                cx,
            ))
            .child(
                self.button(format!("project-verification-advanced-{pid}"), &advanced_label, true, cx, move |s, _, _| {
                    if !s.workspaces.verification_advanced.remove(&pid) {
                        s.workspaces.verification_advanced.insert(pid);
                    }
                })
                .icon(if advanced_open { IconName::ChevronDown } else { IconName::ChevronRight })
                .ghost(),
            );
        if advanced_open {
            let attempts = self.field(
                format!("project-verification-attempts-{pid}"),
                "최대 시도 횟수 (1~10)",
                draft.max_attempts.clone(),
                FieldKind::Line,
                enabled,
                window,
                cx,
                move |s, value| {
                    if let Some(draft) = s.workspaces.verification_drafts.get_mut(&pid) {
                        draft.max_attempts = value;
                    }
                },
            );
            let timeout = self.field(
                format!("project-verification-timeout-{pid}"),
                "명령당 제한 시간 (초, 1~3600)",
                draft.timeout_seconds.clone(),
                FieldKind::Line,
                enabled,
                window,
                cx,
                move |s, value| {
                    if let Some(draft) = s.workspaces.verification_drafts.get_mut(&pid) {
                        draft.timeout_seconds = value;
                    }
                },
            );
            section = section
                .child(div().id(format!("project-verification-attempts-{pid}")).test_support().child(attempts))
                .child(div().id(format!("project-verification-timeout-{pid}")).test_support().child(timeout));
        }
        let reverted = VerificationDraft::from_plan(saved);
        section
            .child(
                row()
                    .justify_end()
                    .children(changed.then(|| note(self.text("저장하지 않은 변경이 있습니다."), cx)))
                    .child(self.button(
                        format!("project-verification-revert-{pid}"),
                        "되돌리기",
                        enabled && changed,
                        cx,
                        move |s, _, _| {
                            s.workspaces.verification_drafts.insert(pid, reverted.clone());
                        },
                    ))
                    .child(
                        self.button(format!("project-verification-save-{pid}"), "저장", enabled && changed, cx, move |s, _, _| {
                            let Some(draft) = s.workspaces.verification_drafts.get(&pid) else {
                                return;
                            };
                            match draft.to_plan() {
                                | Ok(plan) => s.command(Action::SetProjectVerification {
                                    project: pid,
                                    plan,
                                }),
                                | Err(error) => s.workspaces.error = Some(s.text(&error)),
                            }
                        })
                        .primary(),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn workspace_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let refresh = row().w_full().justify_end().child(
            self.button("projects-refresh", "프로젝트 목록 새로고침", !self.busy, cx, |s, _, _| {
                s.command(Action::Projects)
            })
            .ghost(),
        );
        let mut body = column();
        body = body
            .child(self.field(
                "project-path",
                "작업 폴더",
                self.workspaces.path.clone(),
                FieldKind::Line,
                !self.busy,
                window,
                cx,
                |s, v| s.workspaces.path = v,
            ))
            .child(
                row()
                    .child(self.button("project-browse", "폴더 선택", !self.busy, cx, |s, _, cx| {
                        // Keep native modal loops outside GPUI borrows.
                        let picked = cx.prompt_for_paths(PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: false,
                            prompt: Some(s.text("프로젝트 열기").into()),
                        });
                        cx.spawn(async move |view, cx| {
                            let picked = picked.await;
                            let _ = view.update(cx, |s, cx| {
                                match picked {
                                    | Ok(Ok(paths)) => {
                                        if let Some(action) = s.workspaces.apply_picked_folder(paths.and_then(|paths| paths.into_iter().next())) {
                                            s.command(action);
                                        }
                                    },
                                    | Ok(Err(error)) => s.workspaces.error = Some(error.to_string()),
                                    | Err(_) => {},
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                    }))
                    .child(self.button(
                        "project-register",
                        "프로젝트 열기",
                        !self.busy && !self.workspaces.path.trim().is_empty(),
                        cx,
                        |s, _, _| s.command(Action::OpenProject(s.workspaces.path.clone())),
                    )),
            );
        let mut page = column()
            .gap_6()
            .child(refresh)
            .child(self.settings_section(self.text("프로젝트 열기"), body, cx));
        for project in self.workspaces.catalog.projects.clone() {
            let pid = project.id;
            let mut project_view = column().child(
                row()
                    .id(format!("project-actions-{pid}"))
                    .test_support()
                    .group("project-row")
                    .w_full()
                    .justify_between()
                    .child(note(project.path.clone(), cx).min_w_0())
                    .child(self.project_menu(&project, cx)),
            );
            // Worktree rows and the forms the project menu opens stay next to
            // the menu; the taller verification section closes the card.
            project_view = project_view.child(note("Worktree", cx));
            for w in &project.workspaces {
                let w = w.clone();
                let id = w.id;
                let line = row().min_w_0().w_full().child(
                    self.choice(
                        format!("project-select-{id}"),
                        format!("{} · {}", w.branch.as_deref().unwrap_or(""), w.path),
                        self.workspaces.selected == Some(id),
                        !self.busy && !w.missing,
                        cx,
                        move |s, _, _| {
                            let wake = s.wake.clone();
                            s.select_workspace(Some(id), &wake);
                            s.panel = None;
                        },
                    )
                    .max_w_full()
                    .min_w_0(),
                );
                project_view = project_view.child(line);
            }
            if self.workspaces.project == Some(pid) && project.git {
                project_view = project_view
                    .child(self.field(
                        "worktree-branch",
                        "새 브랜치",
                        self.workspaces.branch.clone(),
                        FieldKind::Line,
                        !self.busy,
                        window,
                        cx,
                        |s, v| s.workspaces.branch = v,
                    ))
                    .child(self.field(
                        "worktree-base",
                        "기준 브랜치",
                        self.workspaces.base.clone(),
                        FieldKind::Line,
                        !self.busy,
                        window,
                        cx,
                        |s, v| s.workspaces.base = v,
                    ))
                    .child(self.button(
                        "worktree-create",
                        "worktree 만들기",
                        !self.busy && !self.workspaces.branch.trim().is_empty(),
                        cx,
                        move |s, _, _| {
                            s.command(Action::CreateWorktree {
                                project_id: pid,
                                branch: s.workspaces.branch.clone(),
                                base: s.workspaces.base.clone(),
                            })
                        },
                    ))
                    .child(self.button("worktree-create-cancel", "취소", !self.busy, cx, |s, _, _| {
                        s.workspaces.project = None;
                    }));
            }
            if let Some(w) = &self.workspaces.remove
                && w.project_id == pid
            {
                let id = w.id;
                project_view = project_view
                    .child(error(format!("{}: {}", self.text("worktree를 제거할까요?"), w.path), cx))
                    .child(
                        row()
                            .child(self.button("remove-worktree-cancel", "취소", true, cx, |s, _, _| s.workspaces.remove = None))
                            .child(self.button("remove-worktree-confirm", "삭제 확인", !self.busy, cx, move |s, _, _| {
                                s.workspaces.remove = None;
                                s.command(Action::RemoveWorktree(id));
                            })),
                    );
            }
            project_view = project_view.child(self.verification_section(pid, &project.verification, window, cx));
            page = page.child(self.settings_section(project.name.clone(), project_view, cx));
        }
        if let Some(e) = &self.workspaces.error {
            page = page.child(error(e.clone(), cx));
        }
        page.into_any_element()
    }
}
