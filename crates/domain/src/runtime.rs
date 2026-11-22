use serde::{Deserialize,
            Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub account: Option<String>,
    pub method: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct Prompt {
    pub mcp_servers: Option<Vec<String>>,
    pub skills: Vec<crate::SkillReference>,
    pub guidance: Option<crate::PreparedGuidance>,
    pub attachments: Vec<crate::Attachment>,

    pub claude_permission_mode: Option<ClaudePermissionMode>,
    pub codex_execution_policy: Option<crate::CodexExecutionPolicy>,

    pub text: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
}

impl Prompt {
    pub fn runtime_text(&self) -> String {
        match &self.guidance {
            | Some(guidance) => format!("{}\n\nUser request:\n{}", guidance.render(), self.text),
            | None => self.text.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Completion {
    pub session_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Verification {
        report: crate::VerificationReport,
    },
    InputDelivery {
        message_id: i64,
        delivered: bool,
    },
    RuntimeStatus {
        status: RuntimeStatus,
        message: String,
        code: Option<String>,
    },
    Usage {
        usage: crate::Usage,
    },
    Authorization {
        url: String,
        code: Option<String>,
    },
    Session {
        id: String,
    },
    Text {
        delta: String,
    },
    /// Return true to grant this individual tool request. All other return
    /// values are ignored.
    Tool {
        call: ToolCall,
    },
    UserInput {
        questions: Vec<Question>,
    },
    Elicitation {
        server_name: String,
        message: String,
        schema: Option<Value>,
        url: Option<String>,
    },
    InteractionResolved {
        request_id: u64,
        response: InteractionResponse,
    },
    TurnEnd {
        completed: bool,
    },
    Permission {
        method: String,
        details: Value,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Retrying,
    Warning,
    Error,
    UsageLimited,
}

impl Event {
    pub fn automatic_computer_response(&self) -> Option<InteractionResponse> {
        let Self::Permission {
            method,
            details,
        } = self
        else {
            return None;
        };
        let tool = match method.as_str() {
            | "computer/native_tool" => details["tool"].as_str()?,
            | "claude/can_use_tool" => details["tool_name"].as_str()?.strip_prefix("mcp__gpui-agent-session__")?,
            | "session/request_permission" if details["toolCall"]["_meta"]["mcp"]["server"] == "gpui-agent-session" =>
                details["toolCall"]["_meta"]["mcp"]["tool"].as_str()?,
            | _ => return None,
        };
        (matches!(tool, "computer_capture" | "computer_act" | "computer_close") && self.allows_once()).then_some(InteractionResponse::Approval {
            allow: true,
        })
    }

    pub fn elicitation_response(&self, input: &str) -> Option<InteractionResponse> {
        let Self::Elicitation {
            schema, ..
        } = self
        else {
            return None;
        };
        let content = if schema.is_some() { Some(serde_json::from_str(input).ok()?) } else { None };
        let response = InteractionResponse::Elicitation {
            action: ElicitationAction::Accept,
            content,
        };
        crate::validate_response(self, &response).ok()?;
        Some(response)
    }

    pub fn allows_once(&self) -> bool {
        match self {
            | Event::Permission {
                method,
                details,
            } if method == "session/request_permission" => details["options"]
                .as_array()
                .is_some_and(|options| options.iter().any(|option| option["kind"] == "allow_once")),
            | Event::Permission {
                ..
            } => true,
            | _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaudePermissionMode {
    #[default]
    Default,
    AcceptEdits,
    Plan,
    Auto,
    DontAsk,
    BypassPermissions,
}
impl ClaudePermissionMode {
    pub const ALL: [Self; 6] = [Self::Default, Self::AcceptEdits, Self::Plan, Self::Auto, Self::DontAsk, Self::BypassPermissions];

    pub fn as_str(self) -> &'static str {
        match self {
            | Self::Default => "default",
            | Self::AcceptEdits => "acceptEdits",
            | Self::Plan => "plan",
            | Self::Auto => "auto",
            | Self::DontAsk => "dontAsk",
            | Self::BypassPermissions => "bypassPermissions",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InteractionResponse {
    Approval { allow: bool },
    ClaudeMode { mode: ClaudePermissionMode },
    ClaudeSuggestion { index: usize },
    Answers { answers: std::collections::BTreeMap<String, Vec<String>> },
    Elicitation { action: ElicitationAction, content: Option<Value> },
    Cancelled,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub multi_select: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolActivity {
    Progress { message: String },
    TerminalInput { process_id: String, stdin: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub output_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<ToolActivity>,
}
impl ToolCall {
    pub fn merge(&mut self, patch: &Self) {
        self.activity.extend_from_slice(&patch.activity);
        if patch.title.is_some() {
            self.title.clone_from(&patch.title);
        }
        if patch.status.is_some() {
            self.status.clone_from(&patch.status);
        }
        if patch.input.is_some() {
            self.input.clone_from(&patch.input);
        }
        if patch.output.is_some() {
            self.output.clone_from(&patch.output);
        }
        if let Some(delta) = &patch.output_delta {
            let old = self.output.as_ref().and_then(Value::as_str).unwrap_or("");
            self.output = Some(Value::String(format!("{old}{delta}")));
        }
    }
}
pub type ResponseFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = InteractionResponse> + Send + 'a>>;
pub type EmitFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>>;
pub trait EventHandler: Send {
    fn job_id(&self) -> Option<i64> { None }
    fn conversation_id(&self) -> Option<i64> { None }
    fn emit(&mut self, event: Event) -> bool;
    /// Runtime streams await delivery so bounded consumers can apply
    /// backpressure.
    fn emit_async(&mut self, event: Event) -> EmitFuture<'_> { Box::pin(async move { self.emit(event) }) }
    fn respond(&mut self, event: Event) -> ResponseFuture<'_> {
        let permission = matches!(event, Event::Permission { .. });
        let allow = self.emit(event);
        Box::pin(async move {
            if permission {
                InteractionResponse::Approval {
                    allow,
                }
            } else {
                InteractionResponse::Cancelled
            }
        })
    }
}
impl<F: FnMut(Event) -> bool + Send> EventHandler for F {
    fn emit(&mut self, event: Event) -> bool { self(event) }
}
pub type Handler<'a> = dyn EventHandler + 'a;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acp_only_advertises_one_time_approval_when_the_runtime_offers_it() {
        for (kind, expected) in [("allow_once", true), ("allow_always", false), ("reject_once", false)] {
            let event = Event::Permission {
                method: "session/request_permission".into(),
                details: serde_json::json!({"options":[{"kind":kind,"optionId":"choice"}]}),
            };
            assert_eq!(event.allows_once(), expected);
        }
    }
}
