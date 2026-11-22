use crate::{app::ChatApp,
            client::InteractionTarget,
            content,
            ui::*};
use gpui::{self,
           component::Disableable as _,
           *};
use gpui_agent_contracts::*;
use gpui_agent_presentation::{Pending,
                              tools};

#[derive(Clone, Copy)]
pub(super) enum PendingTarget {
    Current,
}

impl ChatApp {
    fn computer_tools(&self, target: PendingTarget) -> Vec<ToolCall> {
        let conversation = match target {
            | PendingTarget::Current => Some(&self.conversations[self.selected]),
        };
        let Some((pending, conversation)) = self.pending(target).zip(conversation) else {
            return Vec::new();
        };
        let message = conversation.assistant.and_then(|i| conversation.messages[i].id).unwrap_or(-1);
        pending.computer_tools(&conversation.timeline, message)
    }

    pub(super) fn computer_pending_job(&self) -> Option<i64> {
        let target = self.pending_target()?;
        if self.computer_tools(target).is_empty() {
            return None;
        }
        match target {
            | PendingTarget::Current => self.background_job,
        }
    }

    fn computer_approval_ready(&self, target: PendingTarget) -> bool {
        if !self.computer_tools(target).iter().any(|tool| tool.title.as_deref() != Some("computer_close")) {
            return true;
        }
        let job = match target {
            | PendingTarget::Current => self.background_job,
        };
        self.computer.pending.is_none()
            && job.is_some()
            && self.computer.status.as_ref().is_some_and(|status| {
                if status.automatic {
                    status.available && (status.job_id.is_none() || status.job_id == job)
                } else {
                    self.computer.remaining() > 0 && status.job_id == job
                }
            })
    }

    pub(super) fn computer_gate(&self) -> Option<(PendingTarget, i64)> {
        let target = self.pending_target()?;
        let job = match target {
            | PendingTarget::Current => self.background_job?,
        };
        matches!(&self.pending(target)?.event,Event::Permission {method,details} if method=="computer/request_control" && details["job_id"].as_i64()==Some(job))
            .then_some((target, job))
    }

    pub(super) fn pending_target(&self) -> Option<PendingTarget> {
        (self.panel.is_none() && self.conversations[self.selected].pending.is_some()).then_some(PendingTarget::Current)
    }

    pub(super) fn pending(&self, target: PendingTarget) -> Option<&Pending> {
        match target {
            | PendingTarget::Current => self.conversations[self.selected].pending.as_ref(),
        }
    }

    fn pending_mut(&mut self, target: PendingTarget) -> Option<&mut Pending> {
        match target {
            | PendingTarget::Current => self.conversations[self.selected].pending.as_mut(),
        }
    }

    pub(super) fn answer_pending(&mut self, target: PendingTarget, request: u64, response: InteractionResponse) {
        if matches!(
            &response,
            InteractionResponse::Elicitation {
                action: ElicitationAction::Accept,
                ..
            }
        ) && !self.computer_approval_ready(target)
        {
            return;
        }
        let job = match target {
            | PendingTarget::Current => self.background_job,
        };
        let computer_approval = self.pending(target).is_some_and(|pending| {
            pending.request_id == request
                && !pending.sending
                && matches!(&pending.event, Event::Elicitation { server_name, .. } if server_name == "gpui-agent-session")
                && matches!(
                    &response,
                    InteractionResponse::Elicitation {
                        action: ElicitationAction::Accept,
                        ..
                    }
                )
        });
        if computer_approval
            && job.is_some()
            && self.computer.remaining() > 0
            && let Some(status) = self.computer.status.as_ref().filter(|status| status.job_id == job)
            && let Some(identity) = &status.foreground
        {
            gpui_agent_control::allow_computer_foreground(identity);
        }
        let Some(p) = self.pending_mut(target).filter(|p| p.request_id == request && !p.sending) else {
            return;
        };
        p.sending = true;
        let operation = p.operation_id;
        let execution = self
            .background_job
            .map(InteractionTarget::Job)
            .unwrap_or(InteractionTarget::Operation(operation));
        let wake = self.wake.clone();
        match target {
            | PendingTarget::Current => self
                .client
                .respond(PROVIDERS[self.selected], execution, request, response, self.tx.clone(), wake),
        }
    }

    pub(super) fn permission_modes(&self, target: Option<PendingTarget>, mode: Option<ClaudePermissionMode>, enabled: bool, cx: &Context<Self>) -> AnyElement {
        if target.is_none() {
            use gpui::component::menu::{DropdownMenu,
                                        PopupMenuItem};
            let view = cx.weak_entity();
            let index = self.selected;
            let workspace = self.client.workspace_id();
            let label = format!("{}: {}", self.text("권한 모드"), mode.unwrap_or_default().as_str());
            return self
                .button("permission-picker", label, enabled, cx, |_, _, _| {})
                .ghost()
                .dropdown_menu(move |mut menu, _, _| {
                    for choice in ClaudePermissionMode::ALL {
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(choice.as_str())
                                .checked(mode.unwrap_or_default() == choice)
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |s, cx| {
                                        if !s.busy && s.selected == index && s.client.workspace_id() == workspace {
                                            s.conversations[index].permission_mode = (choice != ClaudePermissionMode::Default).then_some(choice);
                                            cx.notify();
                                        }
                                    });
                                }),
                        );
                    }
                    menu
                })
                .into_any_element();
        }
        let mut body = column().gap_1().child(note(self.text("권한 모드"), cx));
        let mut choices = row();
        for value in ClaudePermissionMode::ALL {
            let label = match value {
                | ClaudePermissionMode::Default => "기본 · 수동 승인",
                | ClaudePermissionMode::AcceptEdits => "편집 자동 허용",
                | ClaudePermissionMode::Plan => "계획 모드",
                | ClaudePermissionMode::Auto => "자동 판단",
                | ClaudePermissionMode::DontAsk => "승인된 도구만 허용",
                | ClaudePermissionMode::BypassPermissions => "권한 검사 생략",
            };
            choices = choices.child(self.choice(
                format!("mode-{}-{}", target.is_some(), value.as_str()),
                label,
                mode.unwrap_or_default() == value,
                enabled,
                cx,
                move |s, _, _| {
                    let mode = (value != ClaudePermissionMode::Default).then_some(value);
                    if let Some(target) = target {
                        if let Some(p) = s.pending_mut(target) {
                            p.mode = mode;
                        }
                    } else {
                        s.conversations[s.selected].permission_mode = mode;
                    }
                },
            ));
        }
        body = body.child(choices);
        if mode == Some(ClaudePermissionMode::BypassPermissions) {
            body = body.child(note(self.text("권한 검사를 생략하고 도구를 실행합니다."), cx));
        }
        body.into_any_element()
    }

    pub(super) fn interaction_panel(&mut self, target: PendingTarget, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(p) = self.pending(target) else {
            return div().into_any_element();
        };
        let (request, event, sending, mode) = (p.request_id, p.event.clone(), p.sending, p.mode);
        let mut body = column().child(heading(self.text("사용자 확인 필요")));
        if let Some((_, job)) = self.computer_gate() {
            let ready =
                self.computer.pending.is_none() && self.computer.status.as_ref().is_some_and(|v| v.job_id == Some(job)) && self.computer.remaining() > 0;
            return body
                .child(note(
                    self.text("작업 권한을 승인한 뒤 실행을 시작하세요. 네이티브 도구는 요청마다 별도로 승인합니다."),
                    cx,
                ))
                .child(self.computer_panel(cx))
                .child(self.button("computer-start", "실행 시작", !sending && ready, cx, move |s, _, _| {
                    s.answer_pending(
                        target,
                        request,
                        InteractionResponse::Approval {
                            allow: true,
                        },
                    )
                }))
                .child(self.button("computer-deny", "작업 실행 거절", !sending, cx, move |s, _, _| {
                    s.answer_pending(
                        target,
                        request,
                        InteractionResponse::Approval {
                            allow: false,
                        },
                    )
                }))
                .into_any_element();
        }
        match event {
            | Event::Elicitation {
                server_name,
                message,
                schema,
                url,
            } => {
                body = body.child(heading(format!("MCP · {server_name}"))).child(message);
                let confirmation = self.pending(target).is_some_and(Pending::confirmation);
                let computer_tools = self.computer_tools(target);
                if !computer_tools.is_empty() {
                    body = body.child(self.computer_state_panel(cx));
                    if let Some(job) = self.computer_pending_job() {
                        body = body.child(
                            self.button("computer-job-cancel", "작업 취소", !sending, cx, move |s, _, _| {
                                if s.computer_pending_job() != Some(job) {
                                    return;
                                }
                                let Some(pending) = s.pending_mut(target).filter(|pending| pending.request_id == request && !pending.sending) else {
                                    return;
                                };
                                pending.sending = true;
                                match target {
                                    | PendingTarget::Current => s.client.cancel_job(PROVIDERS[s.selected], job, Some(request), s.tx.clone(), s.wake.clone()),
                                }
                            })
                            .danger(),
                        );
                    }
                    for tool in computer_tools {
                        body = body.child(
                            column()
                                .id(format!("computer-pending-{}", tool.id))
                                .test_support()
                                .child(note(self.text("진행 중인 컴퓨터 도구"), cx))
                                .child(tool.title.unwrap_or_default())
                                .child(content::markdown(
                                    format!("computer-pending-input-{}", tool.id),
                                    gpui_agent_presentation::plain(&serde_json::to_string_pretty(&tool.input).unwrap_or_default()),
                                )),
                        );
                    }
                }
                if let Some(schema) = schema.filter(|_| !confirmation) {
                    body = body.child(content::markdown(
                        "mcp-schema",
                        gpui_agent_presentation::plain(&serde_json::to_string_pretty(&schema).unwrap_or_default()),
                    ));
                    let value = self.pending(target).and_then(|p| p.free_text.get("content")).cloned().unwrap_or_default();
                    body = body
                        .child(self.field(
                            format!("mcp-content-{request}"),
                            "MCP 입력 (JSON)",
                            value,
                            FieldKind::Text,
                            !sending,
                            window,
                            cx,
                            move |s, value| {
                                if let Some(p) = s.pending_mut(target).filter(|p| p.request_id == request) {
                                    p.free_text.insert("content".into(), value);
                                }
                            },
                        ))
                        .child(note(self.text("요청한 형식에 맞는 JSON을 입력하세요."), cx));
                }
                if let Some(url) = url {
                    let safe = url.starts_with("https://") || url.starts_with("http://");
                    body = body
                        .child(url.clone())
                        .child(self.button("mcp-open", "요청 페이지 열기", !sending && safe, cx, move |_, _, cx| cx.open_url(&url)));
                }
                let ready = self.pending(target).and_then(Pending::response).is_some() && self.computer_approval_ready(target);
                body = body.child(
                    row()
                        .child(self.button("mcp-submit", "수락", !sending && ready, cx, move |s, _, _| {
                            if let Some(response) = s.pending(target).and_then(Pending::response) {
                                s.answer_pending(target, request, response);
                            }
                        }))
                        .child(self.button("mcp-decline", "거절", !sending, cx, move |s, _, _| {
                            s.answer_pending(
                                target,
                                request,
                                InteractionResponse::Elicitation {
                                    action: ElicitationAction::Decline,
                                    content: None,
                                },
                            )
                        })),
                );
            },
            | Event::Permission {
                method,
                details,
            } => {
                let allows = Event::Permission {
                    method: method.clone(),
                    details: details.clone(),
                }
                .allows_once();
                let title = details
                    .pointer("/input/command")
                    .or_else(|| details.get("command"))
                    .or_else(|| details.pointer("/toolCall/title"))
                    .or_else(|| details.get("tool_name"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&method);
                body = body
                    .child(heading(self.text("이 요청을 허용할까요?")))
                    .child(title.to_owned())
                    .child(content::markdown(
                        "permission-details",
                        gpui_agent_presentation::plain(&serde_json::to_string_pretty(&details).unwrap_or_default()),
                    ))
                    .child(
                        row()
                            .child(self.button(
                                "allow-once",
                                if method == "item/permissions/requestApproval" {
                                    "현재 턴 허용"
                                } else {
                                    "한 번 허용"
                                },
                                !sending && allows,
                                cx,
                                move |s, _, _| {
                                    s.answer_pending(
                                        target,
                                        request,
                                        InteractionResponse::Approval {
                                            allow: true,
                                        },
                                    )
                                },
                            ))
                            .child(self.button("deny", "거절", !sending, cx, move |s, _, _| {
                                s.answer_pending(
                                    target,
                                    request,
                                    InteractionResponse::Approval {
                                        allow: false,
                                    },
                                )
                            })),
                    );
                if method == "claude/can_use_tool" {
                    body = body
                        .child(self.permission_modes(Some(target), mode, !sending, cx))
                        .child(note(self.text("선택한 모드는 이후 요청에 적용됩니다. 현재 요청은 한 번 허용합니다."), cx))
                        .child(self.button("apply-mode", "모드 적용 후 한 번 허용", !sending, cx, move |s, _, _| {
                            let mode = s.pending(target).and_then(|p| p.mode).unwrap_or_default();
                            s.answer_pending(
                                target,
                                request,
                                InteractionResponse::ClaudeMode {
                                    mode,
                                },
                            );
                        }));
                    if details["suppress_always_allow_rule"] != true
                        && let Some(suggestions) = details["permission_suggestions"].as_array()
                    {
                        for (index, suggestion) in suggestions.iter().enumerate() {
                            body = body
                                .child(content::markdown(
                                    format!("permission-rule-{index}"),
                                    gpui_agent_presentation::plain(&serde_json::to_string_pretty(suggestion).unwrap_or_default()),
                                ))
                                .child(self.button(
                                    format!("apply-rule-{index}"),
                                    format!("{} {}", self.text("제안한 권한 규칙 적용"), index + 1),
                                    !sending,
                                    cx,
                                    move |s, _, _| {
                                        s.answer_pending(
                                            target,
                                            request,
                                            InteractionResponse::ClaudeSuggestion {
                                                index,
                                            },
                                        )
                                    },
                                ));
                        }
                    }
                }
            },
            | Event::UserInput {
                questions,
            } => {
                for q in questions {
                    body = body.child(heading(q.question));
                    for (n, option) in q.options.into_iter().enumerate() {
                        let selected = self.pending(target).and_then(|p| p.answers.get(&q.id)).is_some_and(|a| a.contains(&option));
                        let id = q.id.clone();
                        let multi = q.multi_select;
                        let label = option.clone();
                        let control_id = gpui::ElementId::Name(format!("question-{request}-{id}-{n}").into());
                        let change = cx.listener(move |s, _: &bool, _, cx| {
                            if let Some(p) = s.pending_mut(target).filter(|p| p.request_id == request) {
                                let values = p.answers.entry(id.clone()).or_default();
                                if multi {
                                    if values.contains(&option) {
                                        values.retain(|v| v != &option);
                                    } else {
                                        values.push(option.clone());
                                    }
                                } else {
                                    *values = vec![option.clone()];
                                    p.free_text.remove(&id);
                                }
                            }
                            cx.notify();
                        });
                        body = body.child(if multi {
                            gpui::component::checkbox::Checkbox::new(control_id)
                                .label(label)
                                .checked(selected)
                                .disabled(sending)
                                .on_change(change)
                                .into_any_element()
                        } else {
                            gpui::component::radio::Radio::new(control_id)
                                .label(label)
                                .checked(selected)
                                .disabled(sending)
                                .on_change(change)
                                .into_any_element()
                        });
                    }
                    let id = q.id;
                    let value = self.pending(target).and_then(|p| p.free_text.get(&id)).cloned().unwrap_or_default();
                    body = body.child(self.field(
                        format!("answer-{request}-{id}"),
                        "직접 입력",
                        value,
                        FieldKind::Text,
                        !sending,
                        window,
                        cx,
                        move |s, v| {
                            if let Some(p) = s.pending_mut(target).filter(|p| p.request_id == request) {
                                p.free_text.insert(id.clone(), v);
                            }
                        },
                    ));
                }
                let ready = self.pending(target).and_then(Pending::response).is_some();
                body = body.child(self.button("send-answers", "답변 보내기", !sending && ready, cx, move |s, _, _| {
                    if let Some(response) = s.pending(target).and_then(Pending::response) {
                        s.answer_pending(target, request, response);
                    }
                }));
            },
            | _ => {},
        }
        body.child(self.button("cancel-request", "요청 취소", !sending, cx, move |s, _, _| {
            s.answer_pending(target, request, InteractionResponse::Cancelled)
        }))
        .into_any_element()
    }

    pub(super) fn timeline_view(&self, id: &str, entries: &[TimelineEntry], message: i64, cx: &Context<Self>) -> AnyElement {
        self.timeline_view_with(id, entries, message, &tools(entries, message), cx)
    }

    /// Like `timeline_view` with the tool calls already merged, so callers that
    /// cache them avoid rescanning the whole timeline every frame.
    pub(super) fn timeline_view_with(&self, id: &str, entries: &[TimelineEntry], message: i64, tools: &[ToolCall], cx: &Context<Self>) -> AnyElement {
        let mut body = column().gap_1().child(self.verification_view(entries, message, cx));
        for (index, entry) in entries.iter().enumerate().filter(|(_, entry)| entry.message_id == message) {
            if let Event::RuntimeStatus {
                status,
                message: status_message,
                ..
            } = &entry.event
            {
                body = body.child(
                    div()
                        .id(format!("{id}-{message}-status-{index}"))
                        .test_support()
                        .child(note(format!("{status:?} · {status_message}"), cx)),
                );
            }
            if let Event::InputDelivery {
                message_id,
                delivered,
            } = &entry.event
            {
                body = body.child(note(
                    format!("#{message_id} · {}", self.text(if *delivered { "런타임 접수" } else { "수신함 유지" })),
                    cx,
                ));
            }
        }
        for tool in tools {
            let key = format!("{id}-{message}-{}", tool.id);
            let key_prefix = key.clone();
            let open = self.expanded_tools.contains(&key);
            let label = format!(
                "{} · {}",
                tool.status.as_deref().unwrap_or("pending"),
                tool.title.as_deref().unwrap_or(&tool.id)
            );
            let toggle = self.sidebar_button(
                format!("{key}-toggle"),
                &label,
                if open {
                    gpui::assets::IconName::ChevronDown
                } else {
                    gpui::assets::IconName::ChevronRight
                },
                true,
                cx,
                move |s, _, _| {
                    if !s.expanded_tools.remove(&key) {
                        s.expanded_tools.insert(key.clone());
                    }
                },
            );
            let mut details = column()
                .id(format!("{id}-{message}-{}-details", tool.id))
                .test_support()
                .p_2()
                .border_l_2()
                .border_color(cx.theme().border);
            if open && let Some(image) = gpui_agent_presentation::computer_observation(tool) {
                details = details.child(
                    column()
                        .id(format!("computer-observation-{}", tool.id))
                        .test_support()
                        .child(self.attachments_view(&format!("{id}-{}-observation", tool.id), &[image], cx)),
                );
            }
            let path = tool
                .input
                .as_ref()
                .and_then(|input| input.get("file_path").or_else(|| input.get("path")).and_then(serde_json::Value::as_str))
                .map(str::to_owned);
            // Collapsed rows skip cloning and pretty-printing the payloads.
            let output = open.then(|| {
                if gpui_agent_presentation::is_computer_observation(tool) {
                    tool.output.as_ref().and_then(|v| v.get("structuredContent")).cloned()
                } else {
                    tool.output.clone()
                }
            });
            let input = open.then(|| tool.input.clone());
            for (label, value) in [("입력", input.flatten()), ("실행 결과", output.flatten())].into_iter().filter(|_| open) {
                if let Some(value) = value {
                    let full_key = format!("{key_prefix}-{label}-full");
                    let text = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| serde_json::to_string_pretty(&value).unwrap_or_default());
                    let (shown, hidden) = content::truncate_output(&text, self.expanded_tools.contains(&full_key));
                    details = details
                        .child(note(self.text(label), cx))
                        .child(content::terminal(format!("{id}-{}-{label}", tool.id), shown, path.as_deref()));
                    if hidden > 0 {
                        details = details.child(self.sidebar_button(
                            format!("{full_key}-toggle"),
                            &format!("{} ({} KB)", self.text("전체 출력 보기"), hidden / 1024 + 1),
                            gpui::assets::IconName::ChevronRight,
                            true,
                            cx,
                            move |s, _, _| {
                                s.expanded_tools.insert(full_key.clone());
                            },
                        ));
                    }
                }
            }
            for (index, activity) in tool.activity.iter().enumerate().filter(|_| open) {
                let (label, text) = match activity {
                    | ToolActivity::Progress {
                        message,
                    } => ("진행 메시지", message.clone()),
                    | ToolActivity::TerminalInput {
                        process_id,
                        stdin,
                    } => (
                        "터미널 입력",
                        serde_json::to_string_pretty(&serde_json::json!({"process_id":process_id,"stdin":stdin})).unwrap_or_default(),
                    ),
                };
                let activity_id = format!("{id}-{message}-{}-activity-{index}", tool.id);
                details = details.child(
                    column()
                        .id(activity_id.clone())
                        .test_support()
                        .child(note(self.text(label), cx))
                        .child(content::terminal(format!("{activity_id}-text"), &text, None)),
                );
            }
            body = body.child(gpui::component::collapsible::Collapsible::new().open(open).child(toggle).content(details));
        }
        for (index, entry) in entries.iter().enumerate().filter(|(_, e)| e.message_id == message) {
            if let Event::InteractionResolved {
                response, ..
            } = &entry.event
            {
                body = body.child(
                    div()
                        .id(format!("{id}-{message}-confirmation-{index}"))
                        .test_support()
                        .child(note(format!("{}: {:?}", self.text("사용자 확인"), response), cx)),
                );
            }
        }
        body.into_any_element()
    }

    pub(super) fn verification_view(&self, entries: &[TimelineEntry], message: i64, cx: &Context<Self>) -> AnyElement {
        let mut body = column().gap_4();
        for report in gpui_agent_presentation::verification_reports(entries, message) {
            let key = format!("verification-{}-{}", report.job_id, report.attempt);
            let label = if report.passed() {
                "필수 검증 통과"
            } else if report.finished_revision.is_some() || report.checks.iter().any(|check| check.status == VerificationStatus::Failed) {
                "필수 검증 실패"
            } else {
                "필수 검증 중"
            };
            let version = format!(
                "HEAD: {}\nSHA-256: {}",
                report.revision.head.as_deref().unwrap_or("unborn / plain folder"),
                report.revision.fingerprint
            );
            let mut run = column()
                .id(format!("verification-run-{}-{}", report.job_id, report.attempt))
                .test_support()
                .aria_label(self.text(label))
                .gap_2()
                .child(heading(format!("{} · {} {}", self.text(label), self.text("시도"), report.attempt)))
                .child(
                    row()
                        .child(note(
                            format!(
                                "{} · {}",
                                report.revision.head.as_deref().unwrap_or("unborn").chars().take(12).collect::<String>(),
                                report.revision.fingerprint.chars().take(12).collect::<String>()
                            ),
                            cx,
                        ))
                        .child(
                            self.clipboard(format!("{key}-copy-version"), "코드 버전 복사", version)
                                .accessibility_label(self.text("코드 버전 복사")),
                        ),
                );
            if report.finished_revision.as_ref().is_some_and(|finished| finished != &report.revision) {
                run = run.child(error(self.text("검증 중 코드가 변경되어 다시 검사해야 합니다."), cx));
            }
            for (index, check) in report.checks.iter().enumerate() {
                let check_key = format!("verification-check-{}-{}-{index}", report.job_id, report.attempt);
                let open = self.expanded_tools.contains(&check_key);
                let toggle_key = check_key.clone();
                let status = match check.status {
                    | VerificationStatus::Pending => "대기",
                    | VerificationStatus::Running => "실행 중",
                    | VerificationStatus::Passed => "통과",
                    | VerificationStatus::Failed => "실패",
                };
                let label = format!(
                    "{} · {} · {}",
                    self.text(status),
                    check.exit_code.map(|code| code.to_string()).unwrap_or_else(|| "—".into()),
                    check.command.join(" ")
                );
                let toggle = self.sidebar_button(
                    format!("{check_key}-toggle"),
                    &label,
                    if open {
                        gpui::assets::IconName::ChevronDown
                    } else {
                        gpui::assets::IconName::ChevronRight
                    },
                    true,
                    cx,
                    move |s, _, _| {
                        if !s.expanded_tools.remove(&toggle_key) {
                            s.expanded_tools.insert(toggle_key.clone());
                        }
                    },
                );
                let details = column()
                    .id(format!("{check_key}-details"))
                    .test_support()
                    .p_2()
                    .child(content::markdown(
                        format!("{check_key}-command"),
                        gpui_agent_presentation::plain(&serde_json::to_string_pretty(&check.command).unwrap_or_default()),
                    ))
                    .child(content::markdown(format!("{check_key}-output"), gpui_agent_presentation::plain(&check.output)));
                run = run.child(gpui::component::collapsible::Collapsible::new().open(open).child(toggle).content(details));
            }
            body = body.child(run);
        }
        body.into_any_element()
    }
}
