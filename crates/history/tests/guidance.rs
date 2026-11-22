use gpui_agent_domain::{Completion,
                        GuidanceSnapshot,
                        PreparedGuidance,
                        Prompt,
                        Provider,
                        SkillReference};
use gpui_agent_history::History;
use gpui_agent_ports::ConversationRepository;

#[test]
fn given_repair_guidance_when_history_reopens_then_each_attempt_keeps_its_actual_receipt() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let database = root.path().join("history.sqlite3");
    let history = History::open(&database, root.path())?;
    let mut prompt = Prompt {
        text: "Question".into(),
        ..Default::default()
    };
    let first = history.begin(Provider::Claude, None, &mut prompt)?;
    history.finish(
        first,
        Some(&Completion {
            session_id: "fixture".into(),
            text: "Answer".into(),
        }),
    )?;
    prompt.guidance = Some(serde_json::from_value(serde_json::json!({
        "snapshot":{"revision":"repair","instructions":[],"skills":[],"diagnostics":[]},
        "selected":[],"implicit_available":false
    }))?);
    let repair = history.begin_repair(Provider::Claude, first.conversation_id, &mut prompt)?;
    history.finish(
        repair,
        Some(&Completion {
            session_id: "fixture".into(),
            text: "Repaired".into(),
        }),
    )?;
    drop(history);
    let history = History::open(&database, root.path())?;
    let saved = history.load(Provider::Claude, first.conversation_id)?.unwrap();
    assert_eq!(saved.messages.len(), 3, "Repair does not invent another user request");
    assert_eq!(saved.messages[2].guidance.as_ref().map(|r| r.revision.as_str()), Some("repair"));
    Ok(())
}

#[test]
fn given_selected_skills_when_history_reopens_and_forks_then_selection_and_source_receipts_survive() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let database = root.path().join("history.sqlite3");
    let history = History::open(&database, root.path())?;
    let mut snapshot = GuidanceSnapshot {
        revision: "fixture-revision".into(),
        ..Default::default()
    };
    snapshot.add_instruction("repo:AGENTS.md".into(), "Guidance body is not persisted".into());
    let mut prompt = Prompt {
        text: "Question".into(),
        skills: vec![SkillReference {
            id: "repo:.agents/skills/verify/SKILL.md".into(),
        }],
        guidance: Some(PreparedGuidance {
            extensions: Some(gpui_agent_domain::ConversationExtensions {
                plugins: vec!["repo:review".into()],
                mcp: vec!["local-tools".into()],
            }),
            snapshot,
            selected: vec![],
            implicit_available: false,
        }),
        ..Default::default()
    };
    let turn = history.begin(Provider::Claude, None, &mut prompt)?;
    history.finish(
        turn,
        Some(&Completion {
            session_id: "fixture-session".into(),
            text: "Answer".into(),
        }),
    )?;
    drop(history);
    let history = History::open(&database, root.path())?;
    let saved = history.load(Provider::Claude, turn.conversation_id)?.unwrap();
    assert_eq!(saved.messages[0].skills, prompt.skills);
    assert_eq!(saved.messages[0].guidance.as_ref().unwrap().sources, ["repo:AGENTS.md"]);
    assert_eq!(saved.messages[0].guidance.as_ref().unwrap().revision, "fixture-revision");
    assert_eq!(saved.messages[0].guidance.as_ref().unwrap().extensions.as_ref().unwrap().mcp, ["local-tools"]);
    assert!(!serde_json::to_string(&saved)?.contains("Guidance body is not persisted"));
    let mut next = Prompt {
        text: "Next question".into(),
        ..Default::default()
    };
    let next_turn = history.begin(Provider::Claude, Some(turn.conversation_id), &mut next)?;
    history.finish(
        next_turn,
        Some(&Completion {
            session_id: "fixture-session".into(),
            text: "Next answer".into(),
        }),
    )?;
    let saved = history.load(Provider::Claude, turn.conversation_id)?.unwrap();
    let fork = history.fork(Provider::Claude, turn.conversation_id, saved.messages[2].id.unwrap(), "Fork", &[])?;
    let gpui_agent_ports::ForkOutcome::Created(id) = fork else {
        panic!("Expected a completed-question fork")
    };
    let forked = history.load(Provider::Claude, id)?.unwrap();
    assert_eq!(forked.messages[0].skills, prompt.skills);
    assert_eq!(
        forked.messages[0].guidance.as_ref().unwrap().extensions.as_ref().unwrap().plugins,
        ["repo:review"]
    );
    Ok(())
}
