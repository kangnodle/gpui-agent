use gpui_agent_domain::{Event,
                        InteractionResponse};
use serde_json::json;

#[test]
fn given_host_computer_tools_when_permission_is_requested_then_only_those_tools_are_approved() {
    for tool in ["computer_capture", "computer_act", "computer_close"] {
        let event = Event::Permission {
            method: "claude/can_use_tool".into(),
            details: json!({"tool_name":format!("mcp__gpui-agent-session__{tool}")}),
        };
        assert_eq!(
            event.automatic_computer_response(),
            Some(InteractionResponse::Approval {
                allow: true
            })
        );
    }
    for tool in ["Bash", "mcp__other__computer_act", "mcp__gpui-agent-session__cancel_job"] {
        let event = Event::Permission {
            method: "claude/can_use_tool".into(),
            details: json!({"tool_name":tool}),
        };
        assert!(event.automatic_computer_response().is_none());
    }
}

#[test]
fn given_computer_words_in_an_empty_form_when_no_tool_identity_is_bound_then_it_is_not_auto_approved() {
    let mut event = Event::Elicitation {
        server_name: "gpui-agent-session".into(),
        message: "Approve computer_act".into(),
        schema: Some(json!({"type":"object","properties":{}})),
        url: None,
    };
    assert!(event.automatic_computer_response().is_none());
    if let Event::Elicitation {
        message, ..
    } = &mut event
    {
        *message = "Approve cancel_job".into();
    }
    assert!(event.automatic_computer_response().is_none());
}

#[test]
fn given_acp_mcp_metadata_when_permission_is_requested_then_only_host_computer_tools_are_approved() {
    for (server, tool, once, expected) in [
        ("gpui-agent-session", "computer_capture", true, true),
        ("gpui-agent-session", "computer_act", true, true),
        ("gpui-agent-session", "computer_close", true, true),
        ("other", "computer_act", true, false),
        ("gpui-agent-session", "cancel_job", true, false),
        ("gpui-agent-session", "computer_act", false, false),
    ] {
        let event = Event::Permission {
            method: "session/request_permission".into(),
            details: json!({"options":[{"kind":if once {"allow_once"}else{"allow_always"},"optionId":"yes"}],"toolCall":{"_meta":{"mcp":{"server":server,"tool":tool}},"rawInput":{"arguments":{}}}}),
        };
        assert_eq!(event.automatic_computer_response().is_some(), expected);
    }
    let spoofed = Event::Permission {
        method: "session/request_permission".into(),
        details: json!({"options":[{"kind":"allow_once","optionId":"yes"}],"toolCall":{"title":"gpui-agent-session computer_act","rawInput":{"server":"gpui-agent-session","name":"computer_act"}}}),
    };
    assert!(spoofed.automatic_computer_response().is_none());
}
