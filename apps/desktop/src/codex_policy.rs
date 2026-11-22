use crate::{app::ChatApp,
            ui::*};
use gpui::{component::menu::{DropdownMenu,
                             PopupMenuItem},
           *};
use gpui_agent_contracts::*;

#[derive(Clone, Copy)]
pub(super) enum PolicyTarget {
    Chat(usize),
}
impl PolicyTarget {
    fn key(self) -> &'static str {
        match self {
            | Self::Chat(_) => "chat",
        }
    }
}
impl ChatApp {
    fn set_codex_policy(&mut self, target: PolicyTarget, policy: CodexExecutionPolicy) {
        let policy = (policy != CodexExecutionPolicy::default()).then_some(policy);
        match target {
            | PolicyTarget::Chat(index) if self.selected == index && !self.busy => self.conversations[index].codex_execution_policy = policy,
            | _ => {},
        }
    }

    pub(super) fn codex_policy_picker(&self, target: PolicyTarget, policy: Option<CodexExecutionPolicy>, enabled: bool, cx: &Context<Self>) -> AnyElement {
        let policy = policy.unwrap_or_default();
        let workspace = self.client.workspace_id();
        let view = cx.weak_entity();
        let approval = self
            .button(
                format!("codex-{}-approval-picker", target.key()),
                format!("{}: {}", self.text("승인 정책"), policy.approval_policy.as_str()),
                enabled,
                cx,
                |_, _, _| {},
            )
            .ghost()
            .dropdown_menu(move |mut menu, _, _| {
                for choice in CodexApprovalPolicy::ALL {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(choice.as_str())
                            .checked(policy.approval_policy == choice)
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |s, cx| {
                                    if s.client.workspace_id() == workspace {
                                        s.set_codex_policy(
                                            target,
                                            CodexExecutionPolicy {
                                                approval_policy: choice,
                                                ..policy
                                            },
                                        );
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
                menu
            });
        let view = cx.weak_entity();
        let sandbox = self
            .button(
                format!("codex-{}-sandbox-picker", target.key()),
                format!("{}: {}", self.text("샌드박스"), policy.sandbox.as_str()),
                enabled,
                cx,
                |_, _, _| {},
            )
            .ghost()
            .dropdown_menu(move |mut menu, _, _| {
                for choice in CodexSandbox::ALL {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(choice.as_str()).checked(policy.sandbox == choice).on_click(move |_, _, cx| {
                        let _ = view.update(cx, |s, cx| {
                            if s.client.workspace_id() == workspace {
                                s.set_codex_policy(
                                    target,
                                    CodexExecutionPolicy {
                                        sandbox: choice,
                                        ..policy
                                    },
                                );
                                cx.notify();
                            }
                        });
                    }));
                }
                menu
            });
        let mut body = column().gap_1().child(row().child(approval).child(sandbox));
        if policy.sandbox == CodexSandbox::DangerFullAccess {
            body = body.child(note(self.text("전체 파일 시스템과 네트워크 접근을 허용합니다."), cx));
        }
        if policy.approval_policy == CodexApprovalPolicy::Never {
            body = body.child(note(self.text("승인 요청 없이 실행하며 샌드박스 제한은 유지합니다."), cx));
        }
        body.into_any_element()
    }
}
