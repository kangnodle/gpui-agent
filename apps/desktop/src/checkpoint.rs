use gpui_agent_contracts::{GitCommand,
                           GitOverview};
use std::{sync::mpsc::{self,
                       Receiver},
          time::{Duration,
                 Instant}};
#[derive(Default)]
pub struct GitUi {
    pub(super) pr: crate::pull_request::PrUi,
    pub(super) overview: Option<GitOverview>,
    pub(super) label: String,
    pub(super) message: String,
    pub(super) confirmation: Option<GitCommand>,
    pub(super) restore_preview: bool,
    pub(super) checkpoints_tab: bool,
    pub(super) selected_paths: Vec<String>,
    pub(super) dismissed: bool,
    pub(super) pending: Option<Receiver<Result<GitOverview, String>>>,
    pub(super) diff_pending: Option<(gpui_agent_contracts::GitVersion, Receiver<Result<gpui_agent_contracts::FileDiff, String>>)>,
    pub(super) last: Option<Instant>,
    pub(super) error: Option<String>,
    pub(super) writing: bool,
}
impl GitUi {
    pub fn apply(&mut self, overview: GitOverview, restore: Option<String>) {
        let previous = self.overview.as_ref().and_then(|o| o.proposal.as_ref());
        if let Some(proposal) = &overview.proposal
            && previous.is_none_or(|p| p.expected != proposal.expected || p.message != proposal.message || p.paths != proposal.paths)
        {
            self.message = proposal.message.clone();
            self.selected_paths = proposal.paths.clone();
            self.dismissed = false;
        }
        if overview.commit.is_some() {
            self.message.clear();
            self.selected_paths.clear();
        }
        if let Some(pr) = &overview.pr {
            self.pr.apply(overview.version.clone(), pr.clone());
        }
        self.restore_preview = restore.is_some();
        if self.restore_preview {
            self.checkpoints_tab = false;
        }
        self.confirmation = restore.map(|checkpoint| GitCommand::Restore {
            checkpoint,
            expected: overview.version.clone(),
        });
        self.overview = Some(overview);
    }

    fn inspect(&mut self, workspace: i64, client: &crate::client::ApiClient, wake: &crate::ui::Wake) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.last = Some(Instant::now());
        self.error = None;
        let client = client.clone();
        let wake = wake.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<GitOverview> {
                Ok(client
                    .response(reqwest::Method::GET, &format!("/v1/workspaces/{workspace}/git"), None)?
                    .json()?)
            })()
            .map_err(|error| error.to_string());
            let _ = tx.send(result);
            wake.notify();
        });
    }
}

use crate::{app::ChatApp,
            ui::*};
use gpui::{self,
           *};
impl ChatApp {
    pub(super) fn git_command(&mut self, command: GitCommand) {
        if let Some(workspace) = self.workspaces.selected {
            if matches!(command, GitCommand::Inspect) {
                self.workspaces.git.inspect(workspace, &self.client, &self.wake);
                return;
            }
            if self.busy {
                return;
            }
            self.workspaces.git.pending = None;
            self.workspaces.git.diff_pending = None;
            self.workspaces.git.writing = true;
            self.command(gpui_agent_presentation::Action::Git {
                workspace,
                command,
            });
        }
    }

    pub(super) fn poll_git(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.workspaces.git.pending
            && let Ok(result) = rx.try_recv()
        {
            self.workspaces.git.pending = None;
            changed = true;
            match result {
                | Ok(overview) if self.workspaces.selected == Some(overview.workspace_id) => {
                    if self.workspaces.git.confirmation.is_none() && !self.workspaces.git.restore_preview {
                        if self
                            .workspaces
                            .git
                            .overview
                            .as_ref()
                            .is_some_and(|previous| previous.version != overview.version)
                        {
                            self.workspaces.diff = None;
                        }
                        self.workspaces.git.apply(overview, None);
                    }
                },
                | Ok(_) => {},
                | Err(error) => self.workspaces.git.error = Some(error),
            }
        }
        if let Some((version, rx)) = &self.workspaces.git.diff_pending
            && let Ok(result) = rx.try_recv()
        {
            let fresh = self.workspaces.git.overview.as_ref().is_some_and(|overview| overview.version == *version);
            self.workspaces.git.diff_pending = None;
            changed = true;
            match result {
                | Ok(diff) if fresh && self.workspaces.selected == Some(diff.workspace_id) => self.workspaces.diff = Some(diff),
                | Ok(_) => {},
                | Err(error) => self.workspaces.git.error = Some(error),
            }
        }
        let git_workspace = self.workspaces.selected_workspace().is_some_and(|workspace| {
            self.workspaces
                .catalog
                .projects
                .iter()
                .any(|project| project.id == workspace.project_id && project.git)
        });
        if git_workspace
            && !self.workspaces.git.writing
            && self.workspaces.git.pending.is_none()
            && self.workspaces.git.confirmation.is_none()
            && !self.workspaces.git.restore_preview
            && (self.workspaces.git.last.is_none()
                || ((self.chat_tool == Some(Panel::Git) || self.busy) && self.workspaces.git.last.is_some_and(|last| last.elapsed() >= Duration::from_secs(2))))
        {
            self.git_command(GitCommand::Inspect);
        }
        changed
    }

    fn inspect_git_file(&mut self, path: String) {
        let Some(overview) = self.workspaces.git.overview.as_ref() else {
            return;
        };
        if self.workspaces.selected != Some(overview.workspace_id) {
            return;
        }
        let workspace = overview.workspace_id;
        let (tx, rx) = mpsc::channel();
        self.workspaces.git.diff_pending = Some((overview.version.clone(), rx));
        self.workspaces.git.error = None;
        self.workspaces.diff = None;
        let client = self.client.clone();
        let wake = self.wake.clone();
        let provider = gpui_agent_contracts::PROVIDERS[self.selected];
        std::thread::spawn(move || {
            let result = client.run(
                provider,
                gpui_agent_presentation::Action::Diff {
                    workspace_id: workspace,
                    path,
                },
                "",
                &|update| {
                    let _ = tx.send(match update {
                        | gpui_agent_presentation::Update::Diff(diff) => Ok(diff),
                        | _ => return true,
                    });
                    true
                },
            );
            if let Err(error) = result {
                let _ = tx.send(Err(error.to_string()));
            }
            wake.notify();
        });
    }

    pub(super) fn git_proposal_notice(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let overview = self.workspaces.git.overview.as_ref()?;
        let proposal = overview.proposal.as_ref()?;
        if self.workspaces.selected != Some(overview.workspace_id) || self.workspaces.git.dismissed {
            return None;
        }
        Some(
            row()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    note(format!("{} · {}", self.text("에이전트의 커밋 제안"), proposal.message), cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(self.button("git-proposal-review", "커밋 제안 검토", true, cx, |s, _, _| {
                    s.workspaces.git.checkpoints_tab = false;
                    s.open_panel(Panel::Git);
                }))
                .into_any_element(),
        )
    }

    fn commit_proposal_view(&mut self, overview: &GitOverview, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let proposal = overview.proposal.as_ref()?;
        let fresh = proposal.expected == overview.version;
        let enabled = !self.busy && !self.workspaces.git.restore_preview;
        let mut body = column().child(heading(self.text("에이전트의 커밋 제안"))).child(note(
            self.text("변경 내용을 검토하고 포함할 파일을 선택하세요. 선택한 파일만 스테이징하여 커밋합니다."),
            cx,
        ));
        if !fresh {
            body = body.child(
                note(self.text("제안 이후 파일이나 스테이징이 바뀌었습니다. 에이전트에게 새 제안을 요청하세요."), cx)
                    .id("git-proposal-stale")
                    .test_support(),
            );
        }
        for (index, path) in proposal.paths.iter().enumerate() {
            let selected = self.workspaces.git.selected_paths.contains(path);
            let path = path.clone();
            let label = path.clone();
            let diff_path = path.clone();
            body = body.child(
                row()
                    .child(self.check(format!("proposal-file-{index}"), label, selected, enabled, cx, move |s, checked| {
                        s.workspaces.git.selected_paths.retain(|p| p != &path);
                        if checked {
                            s.workspaces.git.selected_paths.push(path.clone());
                        }
                    }))
                    .child(
                        self.button(format!("proposal-diff-{index}"), "변경 검토", true, cx, move |s, _, _| {
                            s.inspect_git_file(diff_path.clone());
                        })
                        .ghost(),
                    ),
            );
        }
        body = body.child(self.field(
            "proposal-message",
            "커밋 메시지",
            self.workspaces.git.message.clone(),
            FieldKind::Text,
            enabled,
            window,
            cx,
            |s, value| s.workspaces.git.message = value,
        ));
        if let Some(diff) = self.git_file_review(cx) {
            body = body.child(diff);
        }
        let command = GitCommand::CommitSelected {
            message: self.workspaces.git.message.clone(),
            paths: self.workspaces.git.selected_paths.clone(),
            expected: proposal.expected.clone(),
        };
        let can_commit = enabled && fresh && command.validate().is_ok();
        body = body.child(
            row()
                .child(
                    self.button("proposal-approve", "선택한 파일 커밋", can_commit, cx, move |s, _, _| {
                        s.git_command(command.clone())
                    })
                    .primary(),
                )
                .child(
                    self.button("proposal-later", "나중에", true, cx, |s, _, _| {
                        s.workspaces.git.dismissed = true;
                        s.close_chat_tool();
                    })
                    .ghost(),
                ),
        );
        Some(
            body.p_3()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .into_any_element(),
        )
    }

    fn git_file_review(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let diff = self.workspaces.diff.clone()?;
        if self.workspaces.selected != Some(diff.workspace_id) {
            return None;
        }
        let mut body = column()
            .child(heading(diff.path.clone()))
            .child(self.button("diff-attach", "파일 문맥으로 첨부", !self.busy, cx, move |s, _, _| {
                if s.workspaces.selected == Some(diff.workspace_id) {
                    s.command(gpui_agent_presentation::Action::Context(gpui_agent_contracts::FileContextRequest {
                        path: diff.path.clone(),
                        start_line: None,
                        end_line: None,
                    }));
                }
            }))
            .child(note(self.text("작업 파일"), cx))
            .child(self.diff_view("git-file-unstaged", &diff.unstaged, cx))
            .child(note(self.text("스테이징"), cx))
            .child(self.diff_view("git-file-staged", &diff.staged, cx));
        if diff.truncated {
            body = body.child(note(self.text("큰 diff의 일부만 표시합니다."), cx));
        }
        Some(body.into_any_element())
    }

    pub(super) fn git_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let enabled = !self.busy && self.workspaces.selected.is_some();
        let scope = self
            .workspaces
            .selected_workspace()
            .map(|workspace| {
                let branch = self
                    .workspaces
                    .git
                    .overview
                    .as_ref()
                    .map(|overview| overview.version.branch.as_deref().unwrap_or("HEAD (detached)"))
                    .unwrap_or_else(|| workspace.branch.as_deref().unwrap_or("HEAD"));
                format!("{} · {}", branch, workspace.path)
            })
            .unwrap_or_else(|| self.text("프로젝트를 열어 시작하세요"));
        let mut header = column()
            .child(note(scope, cx).id("project-git-scope").test_support())
            .child(
                row().w_full().justify_end().child(
                    self.button(
                        "git-inspect",
                        "Git 상태 새로고침",
                        self.workspaces.selected.is_some() && self.workspaces.git.pending.is_none(),
                        cx,
                        |s, _, _| s.git_command(GitCommand::Inspect),
                    )
                    .ghost(),
                ),
            )
            .child(note(
                self.text("현재 프로젝트의 Git 작업입니다. 변경과 제안은 작업 중에도 확인할 수 있고, 커밋·복원은 작업이 끝난 뒤 실행합니다."),
                cx,
            ))
            .child(row().child(self.button("pr-open", "PR 만들기", enabled, cx, |s, _, _| s.open_panel(Panel::Pr))));
        if let Some(error) = &self.workspaces.git.error {
            header = header.child(crate::ui::error(error.clone(), cx));
        }
        if let Some(error) = &self.workspaces.error {
            header = header.child(crate::ui::error(error.clone(), cx));
        }
        let mut checkpoints = column()
            .child(note(self.text("현재 변경 파일과 스테이징 상태를 복원 지점으로 저장합니다. 저장 후에도 계속 작업할 수 있습니다. Git이 무시하는 파일은 제외되며, 저장 당시 커밋에서만 복원할 수 있습니다."), cx))
            .child(self.field(
                "checkpoint-label",
                "복원 지점 이름",
                self.workspaces.git.label.clone(),
                FieldKind::Line,
                enabled,
                window,
                cx,
                |s, v| s.workspaces.git.label = v,
            ))
            .child(self.button(
                "checkpoint-save",
                "복원 지점 저장",
                enabled && !self.workspaces.git.label.trim().is_empty(),
                cx,
                |s, _, _| {
                    s.git_command(GitCommand::Checkpoint {
                        label: s.workspaces.git.label.clone(),
                    })
                },
            ));
        let mut body = column();
        let overview = self.workspaces.git.overview.clone();
        if let Some(o) = overview {
            let show_manual = o.proposal.is_none() || self.workspaces.git.restore_preview;
            if !show_manual
                && !self.workspaces.git.checkpoints_tab
                && let Some(proposal) = self.commit_proposal_view(&o, window, cx)
            {
                body = body.child(proposal);
            }
            if let Some(backup) = &o.backup {
                body = body.child(note(format!("{}: {backup}", self.text("복구용 복원 지점")), cx));
            }
            if let Some(commit) = &o.commit {
                body = body.child(note(format!("Commit: {commit}"), cx));
            }
            for c in o.checkpoints {
                let id = c.id.clone();
                checkpoints = checkpoints.child(row().child(note(format!("{} · {}", c.label, c.id), cx)).child(self.button(
                    format!("restore-{id}"),
                    "되돌리기 미리보기",
                    enabled && c.head == o.version.head,
                    cx,
                    move |s, _, _| {
                        s.git_command(GitCommand::PreviewRestore {
                            checkpoint: id.clone(),
                        })
                    },
                )));
            }
            for (index, f) in o.files.into_iter().enumerate().filter(|_| show_manual) {
                let path = f.path.clone();
                let version = o.version.clone();
                let path2 = path.clone();
                let version2 = version.clone();
                let diff_path = f.path.clone();
                body = body.child(
                    row()
                        .child(note(format!("{}{} {}", f.index_status, f.worktree_status, f.path), cx))
                        .child(
                            self.button(format!("git-diff-{index}"), "변경 검토", true, cx, move |s, _, _| {
                                s.inspect_git_file(diff_path.clone());
                            })
                            .ghost(),
                        )
                        .child(self.button(
                            format!("stage-{path}"),
                            "스테이징",
                            enabled && !self.workspaces.git.restore_preview,
                            cx,
                            move |s, _, _| {
                                s.git_command(GitCommand::Stage {
                                    path: path.clone(),
                                    staged: true,
                                    expected: version.clone(),
                                })
                            },
                        ))
                        .child(self.button(
                            format!("unstage-{path2}"),
                            "스테이징 해제",
                            enabled && !self.workspaces.git.restore_preview && f.index_status != " " && f.index_status != "?",
                            cx,
                            move |s, _, _| {
                                s.git_command(GitCommand::Stage {
                                    path: path2.clone(),
                                    staged: false,
                                    expected: version2.clone(),
                                })
                            },
                        )),
                );
            }
            if o.truncated {
                body = body.child(note(self.text("큰 diff의 일부만 표시합니다."), cx));
            }
            if show_manual {
                if let Some(diff) = self.git_file_review(cx) {
                    body = body.child(diff);
                }
                body = body.child(self.diff_view("git-overview-diff", &o.diff, cx)).child(self.field(
                    "commit-message",
                    "커밋 메시지",
                    self.workspaces.git.message.clone(),
                    FieldKind::Text,
                    enabled,
                    window,
                    cx,
                    |s, v| s.workspaces.git.message = v,
                ));
                let version = o.version;
                body = body.child(self.button(
                    "commit-review",
                    "커밋 확인",
                    enabled && !self.workspaces.git.restore_preview && !self.workspaces.git.message.trim().is_empty(),
                    cx,
                    move |s, _, _| {
                        s.workspaces.git.confirmation = Some(GitCommand::Commit {
                            message: s.workspaces.git.message.clone(),
                            expected: version.clone(),
                        })
                    },
                ));
            }
        }
        let changes = if self.workspaces.git.overview.is_some() {
            self.settings_section(self.text("변경 파일 · 커밋"), body, cx)
        } else {
            self.settings_section(self.text("변경 파일 · 커밋"), note(self.text("Git 상태를 새로고침하세요."), cx), cx)
        };
        let mut body = column();
        if let Some(command) = self.workspaces.git.confirmation.clone() {
            let restore = matches!(command, GitCommand::Restore { .. });
            body = body
                .child(heading(self.text(if restore {
                    "저장된 작업 상태로 되돌릴까요?"
                } else {
                    "스테이징된 변경을 커밋할까요?"
                })))
                .child(note(
                    self.text("현재 파일과 스테이징 상태를 바꿉니다. 먼저 복구용 복원 지점을 자동 저장합니다. HEAD와 대화는 유지합니다."),
                    cx,
                ))
                .child(
                    row()
                        .child(self.button("git-cancel", "취소", true, cx, |s, _, _| {
                            s.workspaces.git.confirmation = None;
                            s.workspaces.git.restore_preview = false;
                        }))
                        .child(self.button(
                            "git-confirm",
                            if restore { "되돌리기 실행" } else { "커밋 실행" },
                            enabled,
                            cx,
                            move |s, _, _| {
                                s.workspaces.git.confirmation = None;
                                s.git_command(command.clone());
                            },
                        )),
                );
        }
        let searching = !self.settings_search.trim().is_empty();
        let mut page = column().gap_6().child(header);
        if !self.workspaces.git.checkpoints_tab || searching {
            page = page.child(changes);
        }
        if self.workspaces.git.checkpoints_tab || searching {
            page = page.child(self.settings_section(self.text("작업 상태 저장 · 복원"), checkpoints, cx));
        }
        if self.workspaces.git.confirmation.is_some() {
            page = page.child(self.settings_section(self.text("검토 · 확인"), body, cx));
        }
        let mut tabs = row().w_full();
        for (checkpoints, label, id) in [
            (false, "변경 파일 · 커밋", "git-section-changes"),
            (true, "작업 상태 저장 · 복원", "git-section-checkpoints"),
        ] {
            let select = move |s: &mut Self, _: &mut Window, _: &mut Context<Self>| {
                s.workspaces.git.checkpoints_tab = checkpoints;
                s.panel_scroll.set_offset(Point::default());
            };
            let tab = self.button(id, label, true, cx, select).ghost();
            tabs = tabs.child(tab.selected(self.workspaces.git.checkpoints_tab == checkpoints));
        }
        let tabs = tabs.id("git-section-tabs").test_support();
        column().gap_4().child(tabs).child(page).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn given_restore_preview_when_applied_then_changes_are_shown_for_review() {
        let mut ui = super::GitUi {
            checkpoints_tab: true,
            ..Default::default()
        };
        let overview = gpui_agent_contracts::GitOverview {
            workspace_id: 1,
            version: gpui_agent_contracts::GitVersion {
                branch: None,
                head: None,
                index: String::new(),
                worktree: String::new(),
            },
            checkpoints: vec![],
            files: vec![],
            diff: "+change".into(),
            truncated: false,
            backup: None,
            commit: None,
            pr: None,
            proposal: None,
        };
        ui.apply(overview, Some("checkpoint".into()));
        assert!(!ui.checkpoints_tab);
        assert!(ui.confirmation.is_some());
    }
}
