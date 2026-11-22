use gpui_agent_contracts::{GuidanceSnapshot,
                           SkillMetadata};
use gpui_agent_presentation::GuidanceState;

#[test]
fn given_an_unchanged_empty_saved_selection_when_switching_chats_then_no_blank_draft_is_created() {
    let mut conversation = gpui_agent_presentation::Conversation {
        id: Some(1),
        ..Default::default()
    };
    let request = serde_json::from_value(serde_json::json!({"text":"hello","extensions":{"plugins":[],"mcp":[]}})).unwrap();
    conversation.begin_turn(&request);
    conversation.save_draft();
    assert!(conversation.drafts.is_empty());
    conversation.messages[0].guidance = Some(
        serde_json::from_value(serde_json::json!({
            "revision":"r1","sources":[],"extensions":{"plugins":["repo:review"],"mcp":[]}
        }))
        .unwrap(),
    );
    conversation.save_draft();
    assert_eq!(
        conversation.drafts.len(),
        1,
        "an explicit deselection must override the recorded choice on reopen"
    );
    assert!(conversation.drafts[0].extensions.plugins.is_empty());
}

#[test]
fn given_conversation_choices_when_turns_and_new_chats_start_then_only_the_original_conversation_keeps_them() {
    use gpui_agent_presentation::{Conversation,
                                  ExtensionKind};
    let mut conversation = Conversation::default();
    conversation.guidance.toggle_extension(ExtensionKind::Plugin, "repo:review").unwrap();
    conversation.guidance.toggle_extension(ExtensionKind::Mcp, "local-tools").unwrap();
    let request = serde_json::from_value(serde_json::json!({"text":"hello", "extensions":{"plugins":["repo:review"],"mcp":["local-tools"]}})).unwrap();
    conversation.begin_turn(&request);
    assert_eq!(conversation.guidance.extensions.plugins, ["repo:review"]);
    assert_eq!(conversation.guidance.extensions.mcp, ["local-tools"]);
    conversation.save_draft();
    conversation.new_chat();
    assert!(conversation.guidance.extensions.plugins.is_empty());
    assert!(conversation.guidance.extensions.mcp.is_empty());
    assert_eq!(conversation.drafts[0].extensions.plugins, ["repo:review"]);
    assert_eq!(conversation.drafts[0].extensions.mcp, ["local-tools"]);
    assert!(conversation.guidance.toggle_extension(ExtensionKind::Mcp, "bad name").is_err());
    assert!(conversation.guidance.extensions.mcp.is_empty());
}

#[test]
fn given_clean_and_dirty_skill_editors_when_switching_scope_then_only_clean_files_close() {
    use gpui_agent_contracts::GuidanceFile;
    use gpui_agent_presentation::GuidanceEditor;
    let mut editor = GuidanceEditor::default();
    editor.load(GuidanceFile {
        id: "repo:.agents/skills/test/SKILL.md".into(),
        text: "Original".into(),
        revision: Some("v1".into()),
    });
    editor.text = "Draft".into();
    assert!(!editor.select_scope(true));
    assert_eq!(editor.text, "Draft");
    assert!(!editor.user);
    editor.text = "Original".into();
    editor.search = "local".into();
    editor.creating = true;
    assert!(editor.select_scope(true));
    assert!(editor.file.is_none());
    assert!(editor.search.is_empty());
    assert!(!editor.creating);
    editor.name = "new".into();
    assert_eq!(editor.new_id(true), "user:skills/new/SKILL.md");
    assert!(editor.select_scope(false));
    editor.name = "new".into();
    assert_eq!(editor.new_id(true), "repo:.agents/skills/new/SKILL.md");
}

#[test]
fn given_an_editor_draft_when_saved_then_its_revision_is_retained_and_workspace_changes_cannot_overwrite_it() {
    use gpui_agent_contracts::GuidanceFile;
    use gpui_agent_presentation::GuidanceEditor;
    let mut editor = GuidanceEditor::default();
    assert!(editor.begin("workspace-a".into()));
    editor.load(GuidanceFile {
        id: "repo:AGENTS.md".into(),
        text: "Original".into(),
        revision: Some("v1".into()),
    });
    editor.text = "Edited".into();
    assert!(editor.dirty());
    assert!(!editor.begin("workspace-b".into()));
    assert_eq!(editor.request().unwrap().expected_revision.as_deref(), Some("v1"));
    editor.load(GuidanceFile {
        id: "repo:AGENTS.md".into(),
        text: "Edited".into(),
        revision: Some("v2".into()),
    });
    assert!(!editor.dirty());
    assert!(editor.begin("workspace-b".into()));
    assert!(editor.file.is_none());
}

#[test]
fn given_a_scope_change_when_a_late_catalog_arrives_then_it_cannot_restore_old_selections() {
    let mut state = GuidanceState::default();
    let skill = SkillMetadata::parse(
        "repo:.agents/skills/verify/SKILL.md",
        "verify",
        "---\ndescription: Verify changes\n---\nTests",
        None,
    )
    .unwrap()
    .0;
    let catalog = GuidanceSnapshot {
        skills: vec![skill.clone()],
        ..Default::default()
    };
    state.begin("workspace-a".into());
    assert!(state.receive("workspace-a", catalog.clone()));
    state.toggle(&skill.id).unwrap();
    assert_eq!(state.selected.len(), 1);
    state.begin("workspace-b".into());
    assert!(state.selected.is_empty());
    assert!(!state.receive("workspace-a", catalog.clone()));
    assert!(state.snapshot.is_none());
    assert!(state.receive("workspace-b", catalog));
    state.toggle(&skill.id).unwrap();
    state.toggle(&skill.id).unwrap();
    assert!(state.selected.is_empty());
}

#[test]
fn given_a_guidance_snapshot_when_reading_each_scope_then_status_reflects_what_reaches_the_turn() {
    use gpui_agent_contracts::{GuidanceDiagnostic,
                               InstructionDocument};
    use gpui_agent_presentation::{InstructionStatus,
                                  instruction_status};
    let document = |source: &str, truncated| InstructionDocument {
        source: source.into(),
        content: "Rules".into(),
        truncated,
    };
    assert_eq!(instruction_status(None, true), InstructionStatus::Checking);
    let mut snapshot = GuidanceSnapshot {
        instructions: vec![document("user:AGENTS.md", false)],
        ..Default::default()
    };
    assert_eq!(instruction_status(Some(&snapshot), true), InstructionStatus::Applied);
    assert_eq!(instruction_status(Some(&snapshot), false), InstructionStatus::Missing);
    snapshot.diagnostics.push(GuidanceDiagnostic {
        source: "repo:.agents/skills/a/SKILL.md".into(),
        message: "bad skill".into(),
    });
    assert_eq!(
        instruction_status(Some(&snapshot), false),
        InstructionStatus::Missing,
        "skill diagnostics must not mark project instructions as broken"
    );
    snapshot.diagnostics.push(GuidanceDiagnostic {
        source: "repo:AGENTS.md".into(),
        message: "not UTF-8".into(),
    });
    assert_eq!(instruction_status(Some(&snapshot), false), InstructionStatus::Problem);
    snapshot.instructions.push(document("repo:sub/AGENTS.md", true));
    assert_eq!(instruction_status(Some(&snapshot), false), InstructionStatus::Truncated);
    assert_eq!(instruction_status(Some(&snapshot), true), InstructionStatus::Applied);
}

#[test]
fn given_guidance_ids_when_displayed_then_internal_scope_prefix_is_hidden() {
    use gpui_agent_presentation::guidance_display_path;
    assert_eq!(guidance_display_path("user:AGENTS.md"), "AGENTS.md");
    assert_eq!(guidance_display_path("repo:apps/web/AGENTS.md"), "apps/web/AGENTS.md");
    assert_eq!(guidance_display_path("AGENTS.md"), "AGENTS.md");
}
