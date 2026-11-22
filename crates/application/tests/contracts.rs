use serde_json::json;

#[test]
fn moved_models_preserve_public_paths_defaults_and_persisted_json() {
    let request: gpui_agent_contracts::ChatRequest = serde_json::from_value(json!({
        "text": "existing queued message", "model": null, "session_id": null, "conversation_id": 7
    }))
    .unwrap();
    let request: gpui_agent_domain::ChatRequest = request;
    assert!(request.attachments.is_empty());
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "text": "existing queued message", "model": null, "session_id": null, "conversation_id": 7, "attachments": []
        })
    );

    // Existing public domain-module re-exports must not be shadowed by private
    // contract modules.
    let _: gpui_agent_contracts::provider::Provider = gpui_agent_domain::Provider::Claude;
    let _: Option<gpui_agent_contracts::workspace::ProjectCatalog> = None;

    let persisted = json!({"message_id": 3, "text": "edited", "model": null, "claude_permission_mode": "plan"});
    let revision: gpui_agent_contracts::ReviseRequest = serde_json::from_value(persisted.clone()).unwrap();
    let revision: gpui_agent_domain::ReviseRequest = revision;
    assert_eq!(serde_json::to_value(revision).unwrap(), persisted);
    assert!(
        serde_json::from_value::<gpui_agent_contracts::ReviseRequest>(json!({
            "message_id": 3, "text": "edited", "unexpected": true
        }))
        .is_err()
    );
}
