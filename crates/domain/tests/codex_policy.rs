use gpui_agent_domain::ChatRequest;
use serde_json::json;

#[test]
fn given_codex_execution_choices_when_requests_round_trip_then_only_known_policies_are_accepted_and_legacy_defaults_remain_unchanged() {
    let legacy = json!({"text":"Review","attachments":[],"model":null,"session_id":null,"conversation_id":null});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<ChatRequest>(legacy.clone()).unwrap()).unwrap(),
        legacy
    );
    for approval in ["untrusted", "on-request", "never"] {
        for sandbox in ["read-only", "workspace-write", "danger-full-access"] {
            let mut wire = legacy.clone();
            wire["codex_execution_policy"] = json!({"approval_policy":approval,"sandbox":sandbox});
            let request: ChatRequest = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), wire);
        }
    }
    for policy in [json!({"approval_policy":"always"}), json!({"sandbox":"all-files"}), json!({"network":true})] {
        let mut wire = legacy.clone();
        wire["codex_execution_policy"] = policy;
        assert!(serde_json::from_value::<ChatRequest>(wire).is_err());
    }
}
