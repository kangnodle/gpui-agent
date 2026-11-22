use gpui_agent_domain::ChatRequest;

#[test]
fn given_invalid_conversation_extensions_when_submitted_then_the_request_is_rejected() {
    // Given an untrusted selection, When it crosses the chat boundary,
    // Then invalid package IDs and server names cannot be silently ignored.
    for extensions in [
        serde_json::json!({"plugins":["../outside"],"mcp":[]}),
        serde_json::json!({"plugins":[],"mcp":["bad name"]}),
        serde_json::json!({"plugins":[],"mcp":["repeat","repeat"]}),
    ] {
        let request: ChatRequest = serde_json::from_value(serde_json::json!({
            "text":"hello","model":null,"session_id":null,"conversation_id":null,
            "extensions":extensions
        }))
        .unwrap();
        assert!(request.validate().is_err());
    }
}

#[test]
fn given_an_explicit_empty_selection_when_serialized_then_it_stays_distinct_from_legacy_requests() {
    for (extensions, expected) in [
        (serde_json::Value::Null, serde_json::Value::Null),
        (serde_json::json!({"plugins":[],"mcp":[]}), serde_json::json!({"plugins":[],"mcp":[]})),
    ] {
        let request: ChatRequest = serde_json::from_value(serde_json::json!({
            "text":"hello","extensions":extensions
        }))
        .unwrap();
        request.validate().unwrap();
        assert_eq!(serde_json::to_value(request).unwrap()["extensions"], expected);
    }
}

#[test]
fn given_a_selected_package_when_resolved_then_only_its_servers_and_explicit_skills_are_available() {
    use gpui_agent_domain::{ConversationExtensions,
                            PluginCatalog,
                            select_mcp_servers};
    let catalog: PluginCatalog = serde_json::from_value(serde_json::json!({"plugins":[
        {"id":"repo:review","manifest":{"name":"review","description":"Review","mcp_servers":[{"name":"packaged","command":"C:/fixture.exe","args":[]}]},"enabled":true,"skills":[]},
        {"id":"user:disabled","manifest":{"name":"disabled","description":"Disabled"},"enabled":false,"skills":[]}
    ],"diagnostics":[]})).unwrap();
    let selection = ConversationExtensions {
        plugins: vec!["repo:review".into()],
        mcp: vec!["standalone".into()],
    };
    assert_eq!(selection.mcp_names(&catalog).unwrap(), ["standalone", "packaged"]);
    assert!(selection.allows_skill("plugin:repo:review/skills/check/SKILL.md", &[]));
    assert!(!selection.allows_skill("plugin:user:review/skills/check/SKILL.md", &[]));
    assert!(!selection.allows_skill("repo:.agents/skills/check/SKILL.md", &[]));
    let mut unavailable = selection.clone();
    unavailable.plugins = vec!["user:disabled".into()];
    assert!(unavailable.mcp_names(&catalog).is_err());
    unavailable.plugins = vec!["repo:missing".into()];
    assert!(unavailable.mcp_names(&catalog).is_err());
    let servers = catalog.plugins[0].manifest.mcp_servers.clone();
    assert!(select_mcp_servers(servers.clone(), Some(&[])).unwrap().is_empty());
    assert_eq!(select_mcp_servers(servers.clone(), None).unwrap().len(), 1);
    assert!(select_mcp_servers(servers, Some(&["missing".into()])).is_err());
}

#[test]
fn given_an_edited_question_when_extensions_are_changed_then_revision_uses_the_new_selection() {
    let saved: gpui_agent_domain::SavedConversation = serde_json::from_value(serde_json::json!({
        "id":1,"messages":[{"id":2,"user":true,"text":"before","incomplete":false},
        {"id":3,"user":false,"text":"answer","incomplete":false}]
    }))
    .unwrap();
    let revision = serde_json::from_value(serde_json::json!({
        "message_id":2,"text":"after","model":null,"claude_permission_mode":null,
        "extensions":{"plugins":[],"mcp":["local-tools"]}
    }))
    .unwrap();
    let prepared = saved.prepare_revision(revision).unwrap();
    assert_eq!(
        serde_json::to_value(prepared.request).unwrap()["extensions"]["mcp"],
        serde_json::json!(["local-tools"])
    );
}
