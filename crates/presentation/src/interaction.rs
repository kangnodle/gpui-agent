use gpui_agent_contracts::{ClaudePermissionMode,
                           Event,
                           InteractionResponse,
                           TimelineEntry,
                           ToolCall};
use std::collections::BTreeMap;

pub fn is_work_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Tool { .. }
            | Event::Verification { .. }
            | Event::InputDelivery { .. }
            | Event::RuntimeStatus {
                status: gpui_agent_contracts::RuntimeStatus::Retrying,
                ..
            }
    )
}

pub fn verification_reports(entries: &[TimelineEntry], message: i64) -> Vec<&gpui_agent_contracts::VerificationReport> {
    let mut reports = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.message_id == message) {
        if let Event::Verification {
            report,
        } = &entry.event
        {
            reports.insert((report.job_id, report.attempt), report);
        }
    }
    reports.into_values().collect()
}

pub struct Pending {
    pub operation_id: u64,
    pub request_id: u64,
    pub event: Event,
    pub sending: bool,
    pub mode: Option<ClaudePermissionMode>,
    pub answers: BTreeMap<String, Vec<String>>,
    pub free_text: BTreeMap<String, String>,
}
impl Pending {
    pub fn computer_tools(&self, entries: &[TimelineEntry], message: i64) -> Vec<ToolCall> {
        if !matches!(&self.event, Event::Elicitation { server_name, .. } if server_name == "gpui-agent-session") {
            return Vec::new();
        }
        tools(entries, message)
            .into_iter()
            .filter(|tool| {
                tool.status.as_deref() == Some("in_progress") && matches!(tool.title.as_deref(), Some("computer_capture" | "computer_act" | "computer_close"))
            })
            .collect()
    }

    pub fn new(operation_id: u64, request_id: u64, event: Event) -> Self {
        Self {
            operation_id,
            request_id,
            event,
            sending: false,
            mode: None,
            answers: BTreeMap::new(),
            free_text: BTreeMap::new(),
        }
    }

    pub fn confirmation(&self) -> bool {
        matches!(&self.event, Event::Elicitation { schema: Some(schema), .. }
            if schema["type"] == "object" && schema["properties"].as_object().is_some_and(|fields| fields.is_empty())
                && self.event.elicitation_response("{}").is_some())
    }

    pub fn response(&self) -> Option<InteractionResponse> {
        if matches!(self.event, Event::Elicitation { .. }) {
            return self.event.elicitation_response(
                self.free_text
                    .get("content")
                    .map_or(if self.confirmation() { "{}" } else { "" }, String::as_str),
            );
        }
        let Event::UserInput {
            questions,
        } = &self.event
        else {
            return None;
        };
        let mut answers = self.answers.clone();
        for question in questions {
            let values = answers.entry(question.id.clone()).or_default();
            if let Some(text) = self.free_text.get(&question.id).filter(|text| !text.trim().is_empty()) {
                if !question.multi_select {
                    values.clear();
                }
                values.push(text.clone());
            }
            if values.is_empty() {
                return None;
            }
        }
        Some(InteractionResponse::Answers {
            answers,
        })
    }
}
pub fn tools(entries: &[TimelineEntry], message_id: i64) -> Vec<ToolCall> {
    let mut result: Vec<ToolCall> = Vec::new();
    let mut ended = false;
    for entry in entries.iter().filter(|entry| entry.message_id == message_id) {
        match &entry.event {
            | Event::Tool {
                call,
            } =>
                if let Some(old) = result.iter_mut().find(|old| old.id == call.id) {
                    old.merge(call);
                } else {
                    let mut initial = call.clone();
                    initial.output_delta = None;
                    initial.activity.clear();
                    initial.merge(call);
                    result.push(initial);
                },
            | Event::TurnEnd {
                ..
            } => ended = true,
            | _ => {},
        }
    }
    if ended {
        for tool in &mut result {
            if matches!(tool.status.as_deref(), None | Some("pending" | "in_progress")) {
                tool.status = Some("interrupted".into());
            }
        }
    }
    result
}

pub fn computer_observation(tool: &ToolCall) -> Option<gpui_agent_contracts::Attachment> {
    if !matches!(tool.title.as_deref(), Some("computer_capture" | "computer_act")) {
        return None;
    }
    let image = tool.output.as_ref()?.get("content")?.as_array()?.iter().find(|item| item["type"] == "image")?;
    let attachment = gpui_agent_contracts::Attachment::Image {
        name: "Computer observation".into(),
        mime_type: image["mimeType"].as_str()?.into(),
        data: image["data"].as_str()?.into(),
    };
    attachment.validate().ok()?;
    Some(attachment)
}

pub fn is_computer_observation(tool: &ToolCall) -> bool {
    matches!(tool.title.as_deref(), Some("computer_capture" | "computer_act"))
        && tool.status.as_deref() != Some("failed")
        && tool.output.as_ref().is_some_and(|output| {
            output["isError"] != true
                && output["content"]
                    .as_array()
                    .is_some_and(|content| content.iter().any(|item| item["type"] == "image"))
        })
}
#[cfg(test)]
mod tests {
    #[test]
    fn given_a_native_computer_error_when_presented_then_the_failure_message_is_preserved() {
        let output = serde_json::json!({"content":[{"type":"text","text":"Windows prevented target focus"}],"isError":true});
        let tool: ToolCall = serde_json::from_value(serde_json::json!({"id":"input","title":"computer_act","status":"failed","output":output})).unwrap();
        assert!(
            !super::is_computer_observation(&tool),
            "A text-only native error must retain its output instead of being replaced by image metadata"
        );
        let mut tool = tool;
        tool.output = Some(serde_json::json!({"content":[{"type":"image","data":"fixture"}],"structuredContent":{"revision":"1"}}));
        assert!(!super::is_computer_observation(&tool));
        tool.status = Some("completed".into());
        assert!(super::is_computer_observation(&tool));
    }
    #[test]
    fn given_an_empty_mcp_form_when_confirmed_then_no_json_entry_is_required() {
        let pending = super::Pending::new(
            1,
            2,
            gpui_agent_contracts::Event::Elicitation {
                server_name: "gpui-agent-session".into(),
                message: "Approve computer_act".into(),
                url: None,
                schema: Some(serde_json::json!({"type":"object","properties":{}})),
            },
        );
        assert!(
            matches!(pending.response(), Some(gpui_agent_contracts::InteractionResponse::Elicitation { content: Some(value), .. }) if value == serde_json::json!({}))
        );
    }
    use super::*;
    #[test]
    fn given_an_mcp_form_submission_is_ready_only_for_valid_json_and_url_requests_need_no_content() {
        let mut pending = Pending::new(
            1,
            2,
            Event::Elicitation {
                server_name: "fixture".into(),
                message: "Configure".into(),
                url: None,
                schema: Some(serde_json::json!({"type":"object","properties":{"count":{"type":"integer","minimum":1}},"required":["count"]})),
            },
        );
        for invalid in ["", "not json", "{}", "{\"count\":0}", "{\"count\":\"two\"}"] {
            pending.free_text.insert("content".into(), invalid.into());
            assert!(pending.response().is_none());
        }
        pending.free_text.insert("content".into(), "{\"count\":2}".into());
        assert_eq!(
            pending.response(),
            Some(InteractionResponse::Elicitation {
                action: gpui_agent_contracts::ElicitationAction::Accept,
                content: Some(serde_json::json!({"count":2}))
            })
        );
        pending.event = Event::Elicitation {
            server_name: "fixture".into(),
            message: "Continue in browser".into(),
            schema: None,
            url: Some("https://example.invalid/verify".into()),
        };
        assert_eq!(
            pending.response(),
            Some(InteractionResponse::Elicitation {
                action: gpui_agent_contracts::ElicitationAction::Accept,
                content: None
            })
        );
    }
    #[test]
    fn answers_require_every_question_and_preserve_single_and_multiple_selection() {
        for multi_select in [false, true] {
            let mut pending = Pending::new(
                1,
                2,
                Event::UserInput {
                    questions: vec![gpui_agent_contracts::Question {
                        id: "q".into(),
                        question: "Choose".into(),
                        options: vec!["A".into()],
                        multi_select,
                    }],
                },
            );
            assert!(pending.response().is_none());
            pending.free_text.insert("q".into(), "  ".into());
            assert!(pending.response().is_none());
            pending.answers.insert("q".into(), vec!["A".into()]);
            pending.free_text.insert("q".into(), "custom 🦀".into());
            let Some(InteractionResponse::Answers {
                answers,
            }) = pending.response()
            else {
                panic!("missing answers")
            };
            let expected = if multi_select { vec!["A", "custom 🦀"] } else { vec!["custom 🦀"] };
            assert_eq!(answers["q"], expected);
        }
        assert!(
            Pending::new(
                1,
                2,
                Event::Permission {
                    method: "tool".into(),
                    details: serde_json::json!({})
                }
            )
            .response()
            .is_none()
        );
    }
    #[test]
    fn timeline_merges_output_and_keeps_completed_results_when_a_turn_is_cancelled() {
        let make = |id: &str, status: &str, output: Option<&str>, delta: Option<&str>| Event::Tool {
            call: ToolCall {
                id: id.into(),
                title: Some("command".into()),
                status: Some(status.into()),
                input: None,
                output: output.map(|s| serde_json::json!(s)),
                output_delta: delta.map(str::to_owned),
                activity: vec![],
            },
        };
        let events = [
            make("done", "in_progress", None, Some("first")),
            make("done", "completed", Some("final"), None),
            make("running", "in_progress", None, Some("partial")),
            Event::TurnEnd {
                completed: false,
            },
        ];
        let entries = events
            .into_iter()
            .map(|event| TimelineEntry {
                message_id: 5,
                event,
            })
            .collect::<Vec<_>>();
        let calls = tools(&entries, 5);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].status.as_deref(), Some("completed"));
        assert_eq!(calls[0].output, Some(serde_json::json!("final")));
        assert_eq!(calls[1].status.as_deref(), Some("interrupted"));
        assert_eq!(calls[1].output, Some(serde_json::json!("partial")));
        assert!(tools(&entries, 6).is_empty());
    }
}
