use gpui_agent_contracts::{GitCommand,
                           GitVersion,
                           PullRequest};
pub struct PrUi {
    pub open: bool,
    pub(super) remote: String,
    pub(super) base: String,
    pub(super) title: String,
    pub(super) body: String,
    pub(super) draft: bool,
    pub(super) push: bool,
    pub(super) preview: Option<(GitVersion, PullRequest)>,
    pub(super) confirm: bool,
}
impl Default for PrUi {
    fn default() -> Self {
        Self {
            open: false,
            remote: "origin".into(),
            base: "main".into(),
            title: String::new(),
            body: String::new(),
            draft: true,
            push: false,
            preview: None,
            confirm: false,
        }
    }
}
impl PrUi {
    pub fn apply(&mut self, version: GitVersion, pr: PullRequest) {
        self.open = true;
        self.preview = Some((version, pr));
        self.confirm = false;
    }
}

use crate::{app::ChatApp,
            ui::*};
use gpui::{self,
           *};
impl ChatApp {
    pub(super) fn pr_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let enabled = !self.busy && self.workspaces.selected.is_some();
        let body = column()
            .child(note(
                self.text("GitHub CLI(gh) 설치·로그인이 필요합니다. 현재 브랜치의 커밋으로 PR을 생성합니다."),
                cx,
            ))
            .child(self.field(
                "pr-remote",
                "원격",
                self.workspaces.git.pr.remote.clone(),
                FieldKind::Line,
                enabled,
                window,
                cx,
                |s, v| {
                    s.workspaces.git.pr.remote = v;
                    s.workspaces.git.pr.preview = None;
                    s.workspaces.git.pr.confirm = false;
                },
            ))
            .child(self.field(
                "pr-base",
                "대상 브랜치",
                self.workspaces.git.pr.base.clone(),
                FieldKind::Line,
                enabled,
                window,
                cx,
                |s, v| {
                    s.workspaces.git.pr.base = v;
                    s.workspaces.git.pr.preview = None;
                    s.workspaces.git.pr.confirm = false;
                },
            ))
            .child(self.button("pr-preview", "PR 미리보기", enabled, cx, |s, _, _| {
                s.git_command(GitCommand::PreviewPr {
                    remote: s.workspaces.git.pr.remote.clone(),
                    base: s.workspaces.git.pr.base.clone(),
                })
            }))
            .child(self.field(
                "pr-title",
                "PR 제목",
                self.workspaces.git.pr.title.clone(),
                FieldKind::Line,
                enabled,
                window,
                cx,
                |s, v| {
                    s.workspaces.git.pr.title = v;
                    s.workspaces.git.pr.confirm = false;
                },
            ))
            .child(self.field(
                "pr-body",
                "PR 설명",
                self.workspaces.git.pr.body.clone(),
                FieldKind::Text,
                enabled,
                window,
                cx,
                |s, v| {
                    s.workspaces.git.pr.body = v;
                    s.workspaces.git.pr.confirm = false;
                },
            ))
            .child(self.check("pr-draft", "Draft PR", self.workspaces.git.pr.draft, enabled, cx, |s, v| {
                s.workspaces.git.pr.draft = v;
                s.workspaces.git.pr.confirm = false;
            }))
            .child(self.check(
                "pr-push",
                "필요하면 현재 브랜치를 원격에 push",
                self.workspaces.git.pr.push,
                enabled,
                cx,
                |s, v| {
                    s.workspaces.git.pr.push = v;
                    s.workspaces.git.pr.confirm = false;
                },
            ));
        let page = column().gap_6().child(self.settings_section(self.text("PR 설정"), body, cx));
        let mut body = column();
        if let Some((version, pr)) = self.workspaces.git.pr.preview.clone() {
            body = body
                .child(heading(format!("{} · {} → {}", pr.repository, pr.branch, pr.base)))
                .child(note(pr.commits.clone(), cx))
                .child(self.diff_view("pr-diff", &pr.diff, cx));
            if let Some(url) = &pr.url {
                let url = url.clone();
                body = body.child(self.button("existing-pr", "기존 PR 열기", true, cx, move |_, _, cx| cx.open_url(&url)));
            }
            if pr.truncated {
                body = body.child(note(self.text("큰 diff의 일부만 표시합니다."), cx));
            }
            let valid = enabled && !self.workspaces.git.pr.title.trim().is_empty() && (!pr.needs_push || self.workspaces.git.pr.push);
            body = body.child(self.button("pr-review", "PR 생성 확인", valid, cx, |s, _, _| s.workspaces.git.pr.confirm = true));
            if self.workspaces.git.pr.confirm {
                body = body
                    .child(heading(self.text("GitHub에 PR을 생성할까요?")))
                    .child(note(
                        format!(
                            "{} · {} → {} · draft={} · push={}",
                            pr.repository, pr.branch, pr.base, self.workspaces.git.pr.draft, self.workspaces.git.pr.push
                        ),
                        cx,
                    ))
                    .child(self.button("pr-create", "PR 생성 실행", valid, cx, move |s, _, _| {
                        let p = &s.workspaces.git.pr;
                        let command = GitCommand::CreatePr {
                            remote: pr.remote.clone(),
                            base: pr.base.clone(),
                            title: p.title.clone(),
                            body: p.body.clone(),
                            draft: p.draft,
                            push: p.push,
                            expected: version.clone(),
                            expected_base: pr.base_oid.clone(),
                            expected_branch: pr.branch.clone(),
                            expected_repository: pr.repository.clone(),
                        };
                        s.workspaces.git.pr.confirm = false;
                        s.git_command(command);
                    }))
                    .child(self.button("pr-cancel", "취소", true, cx, |s, _, _| s.workspaces.git.pr.confirm = false));
            }
        }
        if self.workspaces.git.pr.preview.is_some() {
            page.child(self.settings_section(self.text("검토 · 확인"), body, cx)).into_any_element()
        } else {
            page.into_any_element()
        }
    }
}
