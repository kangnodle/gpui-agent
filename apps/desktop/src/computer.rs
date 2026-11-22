use crate::{app::ChatApp,
            client::ApiClient,
            ui::*};
use gpui::{AnyElement,
           InteractiveElement,
           IntoElement,
           ParentElement,
           StatefulInteractiveElement,
           TestSupportExt};
use gpui_agent_contracts::{ComputerGrantRequest,
                           ComputerStatus,
                           Job};
use std::{sync::mpsc::{self,
                       Receiver},
          time::{Duration,
                 Instant}};

#[derive(Default)]
pub(super) struct ComputerUi {
    pub status: Option<ComputerStatus>,
    pub pending: Option<Receiver<Result<ComputerStatus, String>>>,
    pub confirm: Option<Job>,
    pub target: Option<i64>,
    pub last: Option<Instant>,
    workspace: Option<i64>,
    error: Option<String>,
}

impl ComputerUi {
    pub fn follow_workspace(&mut self, workspace: Option<i64>) {
        if self.workspace != workspace {
            self.workspace = workspace;
            self.confirm = None;
            self.target = None;
        }
    }

    fn request(&mut self, ctx: &Wake, client: &ApiClient, grant: Option<ComputerGrantRequest>) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error = None;
        let client = client.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<ComputerStatus> {
                if let Some(grant) = grant {
                    client.grant_computer(grant)?;
                }
                client.computer_status()
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.notify();
        });
    }

    pub fn poll(&mut self, ctx: &Wake, client: &ApiClient, open: bool) -> bool {
        let mut changed = self.workspace != client.workspace_id();
        self.follow_workspace(client.workspace_id());
        if let Some(rx) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            changed = true;
            self.pending = None;
            self.last = Some(Instant::now());
            match result {
                | Ok(status) => {
                    self.status = Some(status);
                    self.error = None;
                },
                | Err(error) => {
                    self.status = None;
                    self.error = Some(error);
                },
            }
        }
        if open && self.pending.is_none() && self.last.is_none_or(|last| last.elapsed() >= Duration::from_secs(2)) {
            self.request(ctx, client, None);
            changed = true;
        }
        changed
    }

    pub(super) fn remaining(&self) -> u64 {
        self.status.as_ref().map_or(0, |status| {
            if status.automatic && status.job_id.is_some() {
                return u64::MAX;
            }
            status.expires_in_seconds.saturating_sub(self.last.map_or(0, |last| last.elapsed().as_secs()))
        })
    }
}

impl ChatApp {
    pub(super) fn computer_state_panel(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let enabled = self.computer.pending.is_none();
        let remaining = self.computer.remaining();
        let label = match self.computer.status.as_ref() {
            | None if self.computer.error.is_some() => self.text("제어 상태를 확인하지 못했습니다.").to_owned(),
            | None => self.text("제어 상태 확인 중…").to_owned(),
            | Some(status) if !status.available => self.text("이 호스트는 Windows computer use를 지원하지 않습니다.").to_owned(),
            | Some(status) if status.automatic => format!(
                "{}{}",
                self.text("호스트 실행 중 모든 프로젝트에 컴퓨터 사용 자동 허용"),
                status.job_id.map_or(String::new(), |id| format!(" · #{}", id))
            ),
            | Some(status) if status.job_id.is_some() => format!(
                "{} #{} · {} · {} {}",
                self.text("권한 보유 작업"),
                status.job_id.unwrap_or_default(),
                self.text(if remaining == 0 {
                    "만료 상태 확인 중"
                } else if status.execution_bound {
                    "실행 세션 연결됨"
                } else {
                    "실행 세션 대기"
                }),
                remaining,
                self.text("초 남음")
            ),
            | Some(status) if status.manual_enabled => self.text("수동 Windows 제어가 켜져 있습니다. 먼저 수동 제어를 끄세요.").to_owned(),
            | Some(_) => self.text("권한을 보유한 작업이 없습니다.").to_owned(),
        };
        let mut body = column()
            .child(
                note(label.clone(), cx)
                    .id("computer-status")
                    .role(gpui::Role::Status)
                    .aria_label(label)
                    .test_support(),
            )
            .child(
                self.button("computer-refresh", "상태 새로고침", enabled, cx, |s, _, _| {
                    s.computer.request(&s.wake, &s.client, None)
                })
                .ghost(),
            );
        if let Some(message) = &self.computer.error {
            body = body.child(
                error(message.clone(), cx)
                    .id("computer-error")
                    .role(gpui::Role::Alert)
                    .aria_label(message.clone())
                    .test_support(),
            );
        }
        if let Some(id) = self.computer.status.as_ref().and_then(|status| status.job_id) {
            body = body.child(
                self.button(
                    "computer-revoke",
                    format!("{} #{id}", self.text("작업 권한 철회")),
                    enabled,
                    cx,
                    move |s, _, _| {
                        s.computer.confirm = None;
                        s.computer.request(
                            &s.wake,
                            &s.client,
                            Some(ComputerGrantRequest {
                                job_id: id,
                                enabled: false,
                            }),
                        );
                    },
                )
                .danger(),
            );
        }
        body
    }

    pub(super) fn computer_panel(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let enabled = self.computer.pending.is_none();
        let mut body = self.computer_state_panel(cx);
        if self.computer.status.as_ref().is_some_and(|status| status.automatic) {
            return self.settings_section("Computer use", body, cx).into_any_element();
        }
        let jobs: Vec<_> = self
            .job_monitor
            .jobs
            .iter()
            .filter(|job| job.scope.workspace_id == self.client.workspace_id() && gpui_agent_presentation::is_active(job))
            .filter(|job| self.computer_gate().is_none_or(|(_, id)| id == job.id))
            .cloned()
            .collect();
        if jobs.is_empty() {
            body = body.child(note(self.text("이 작업공간에 승인할 실행 중인 작업이 없습니다."), cx));
        }
        for job in &jobs {
            let id = job.id;
            body = body.child(self.choice(
                format!("computer-job-{id}"),
                format!(
                    "#{id} {} · {} · {}",
                    gpui_agent_contracts::display_name(job.scope.provider),
                    job.scope.account_id,
                    self.text(gpui_agent_presentation::state_label(job.state))
                ),
                self.computer.target == Some(id),
                enabled,
                cx,
                move |s, _, _| {
                    s.computer.target = Some(id);
                    s.computer.confirm = None;
                },
            ));
        }
        let target = jobs.iter().find(|job| Some(job.id) == self.computer.target).cloned();
        let can_grant = enabled
            && target.is_some()
            && self
                .computer
                .status
                .as_ref()
                .is_some_and(|status| status.available && !status.manual_enabled && status.job_id.is_none());
        body = body.child(self.button("computer-grant", "선택한 작업에 10분 허용…", can_grant, cx, move |s, _, _| {
            s.computer.confirm = target.clone();
        }));
        if let Some(job) = self.computer.confirm.clone() {
            let valid = jobs.iter().any(|current| current.id == job.id && current.scope == job.scope) && self.computer.target == Some(job.id);
            body = body
                .child(note(
                    format!(
                        "{} #{} · {} · {}",
                        self.text("승인 대상 작업"),
                        job.id,
                        gpui_agent_contracts::display_name(job.scope.provider),
                        job.scope.account_id
                    ),
                    cx,
                ))
                .child(error(
                    self.text("이 작업에 서버 전체 화면 캡처와 마우스·키보드 제어를 최대 10분 허용할까요?"),
                    cx,
                ))
                .child(
                    row()
                        .child(self.button("computer-cancel", "취소", true, cx, |s, _, _| s.computer.confirm = None).outline())
                        .child(self.button("computer-confirm", "허용", can_grant && valid, cx, move |s, _, _| {
                            if let Some(confirmed) = s.computer.confirm.take()
                                && Some(confirmed.id) == s.computer.target
                                && confirmed.scope.workspace_id == s.client.workspace_id()
                                && s.job_monitor.jobs.iter().any(|current| {
                                    current.id == confirmed.id && current.scope == confirmed.scope && gpui_agent_presentation::is_active(current)
                                })
                            {
                                s.computer.request(
                                    &s.wake,
                                    &s.client,
                                    Some(ComputerGrantRequest {
                                        job_id: confirmed.id,
                                        enabled: true,
                                    }),
                                );
                            }
                        })),
                );
        }
        self.settings_section("Computer use", body, cx).into_any_element()
    }
}
