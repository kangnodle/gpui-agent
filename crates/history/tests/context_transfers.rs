use anyhow::Result;
use gpui_agent_domain::{Attachment,
                        Completion,
                        ContextTransferCommand,
                        ContextTransferState,
                        Event,
                        Prompt,
                        Provider};
use gpui_agent_history::History;
use gpui_agent_ports::ConversationRepository;

fn prompt(text: &str) -> Prompt {
    Prompt {
        text: text.into(),
        ..Default::default()
    }
}

fn command(source: i64, target: Option<i64>, key: &str) -> ContextTransferCommand {
    ContextTransferCommand {
        source_id: source,
        target_id: target,
        through_message_id: None,
        title: None,
        command_id: key.into(),
    }
}

fn complete(history: &History, provider: Provider, id: Option<i64>, text: &str) -> Result<i64> {
    let turn = history.begin(provider, id, &mut prompt(text))?;
    history.finish(
        turn,
        Some(&Completion {
            session_id: "private-native-session".into(),
            text: format!("answer: {text}"),
        }),
    )?;
    Ok(turn.conversation_id)
}

#[test]
fn given_tool_results_and_approvals_when_forking_twice_then_only_tool_context_is_preserved() -> Result<()> {
    let root = tempfile::tempdir()?;
    let history = History::open(&root.path().join("history.sqlite3"), root.path())?;
    let provider = Provider::Codex;
    let turn = history.begin(provider, None, &mut prompt("inspect files"))?;
    history.event(
        turn,
        &Event::Tool {
            call: serde_json::from_value(serde_json::json!({"id":"read-tool","title":"read","status":"completed","output":"durable tool result"}))?,
        },
    )?;
    history.event(
        turn,
        &Event::Permission {
            method: "permission".into(),
            details: serde_json::json!({"private":"approval marker"}),
        },
    )?;
    history.finish(
        turn,
        Some(&Completion {
            session_id: "private-session".into(),
            text: "done".into(),
        }),
    )?;
    let branch = history.transfer(provider, &command(turn.conversation_id, None, "first-fork"))?.target_id;
    let saved = history.load(provider, branch)?.unwrap();
    assert_eq!(saved.timeline.len(), 1);
    assert!(matches!(saved.timeline[0].event, Event::Tool { .. }));
    let leaf = history.transfer(provider, &command(branch, None, "nested-fork"))?.target_id;
    let mut input = prompt("use previous findings");
    let turn = history.begin(provider, Some(leaf), &mut input)?;
    let context = input.attachments[0].text_context().unwrap();
    assert!(context.contains("durable tool result"));
    assert!(!context.contains("approval marker") && !context.contains("private-session"));
    history.finish(turn, None)?;
    Ok(())
}

#[test]
fn given_an_active_source_when_forking_a_completed_boundary_then_the_idle_branch_survives_source_deletion() -> Result<()> {
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = History::open(&path, root.path())?;
        let source = complete(&history, provider, None, "original marker")?;
        let boundary = history.load(provider, source)?.unwrap().messages[1].id;
        let mut active_prompt = prompt("unfinished marker");
        active_prompt.model = Some("saved-model".into());
        let active = history.begin(provider, Some(source), &mut active_prompt)?;
        assert!(history.transfer(provider, &command(source, None, "unfinished")).is_err());
        let mut fork = command(source, None, "stable-prefix");
        fork.through_message_id = boundary;
        let transfer = history.transfer(provider, &fork)?;
        let branch = history.load(provider, transfer.target_id)?.unwrap();
        assert_eq!(branch.messages.len(), 2);
        assert!(branch.session_id.is_none() && branch.timeline.is_empty());
        history.finish(active, None)?;
        history.delete(provider, source)?;
        drop(history);

        let history = History::open(&path, root.path())?;
        assert_eq!(history.transfer(provider, &fork)?.id, transfer.id);
        assert_eq!(history.transfers(provider, branch.id, 0)?[0].state, ContextTransferState::Pending);
        let mut first = prompt("independent work");
        first.session_id = Some("caller-must-not-resume".into());
        let turn = history.begin(provider, Some(branch.id), &mut first)?;
        assert!(first.session_id.is_none());
        assert_eq!(first.model.as_deref(), Some("saved-model"));
        let context = first.attachments[0].text_context().unwrap();
        assert!(context.contains("original marker"));
        assert!(!context.contains("unfinished marker") && !context.contains("private-native-session"));
        history.finish(
            turn,
            Some(&Completion {
                session_id: "independent-session".into(),
                text: "independent answer".into(),
            }),
        )?;
        let mut next = prompt("next turn");
        let turn = history.begin(provider, Some(branch.id), &mut next)?;
        assert_eq!(next.session_id.as_deref(), Some("independent-session"));
        assert!(next.attachments.is_empty());
        history.finish(turn, None)?;
    }
    Ok(())
}

#[test]
fn given_nested_branches_when_merging_then_only_the_ancestor_delta_is_delivered_atomically_once() -> Result<()> {
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = History::open(&path, root.path())?;
        let original = complete(&history, provider, None, "root-only marker")?;
        let branch = history.transfer(provider, &command(original, None, "branch"))?.target_id;
        complete(&history, provider, Some(branch), "middle marker")?;
        let leaf = history.transfer(provider, &command(branch, None, "leaf"))?.target_id;
        complete(&history, provider, Some(leaf), "leaf marker")?;
        let sibling = history.transfer(provider, &command(original, None, "sibling"))?.target_id;
        assert!(history.transfer(provider, &command(leaf, Some(sibling), "unrelated")).is_err());
        let merge = command(leaf, Some(original), "root-merge");
        let receipt = history.transfer(provider, &merge)?;
        history.transfer(provider, &command(leaf, Some(branch), "middle-merge"))?;
        history.delete(provider, leaf)?;
        drop(history);

        let history = History::open(&path, root.path())?;
        assert_eq!(history.transfer(provider, &merge)?.id, receipt.id);
        let mut too_many = prompt("rejected turn");
        too_many.attachments = (0 .. 8)
            .map(|i| Attachment::Text {
                name: format!("file-{i}"),
                text: "content".into(),
                source: None,
            })
            .collect();
        assert!(history.begin(provider, Some(original), &mut too_many).is_err());
        assert_eq!(history.load(provider, original)?.unwrap().messages.len(), 2);
        assert_eq!(history.transfers(provider, original, receipt.id - 1)?[0].state, ContextTransferState::Pending);

        let mut consume = prompt("accept merge");
        let turn = history.begin(provider, Some(original), &mut consume)?;
        assert_eq!(consume.attachments.len(), 1);
        let context = consume.attachments[0].text_context().unwrap();
        assert!(context.contains("middle marker") && context.contains("leaf marker"));
        assert!(!context.contains("root-only marker") && !context.contains("private-native-session"));
        let attached = history.transfers(provider, original, receipt.id - 1)?.remove(0);
        assert_eq!(attached.state, ContextTransferState::Attached);
        assert_eq!(attached.attached_message_id, history.load(provider, original)?.unwrap().messages[2].id);
        history.finish(
            turn,
            Some(&Completion {
                session_id: "original-session".into(),
                text: "merged answer".into(),
            }),
        )?;
        let mut next = prompt("no replay");
        let turn = history.begin(provider, Some(original), &mut next)?;
        assert!(next.attachments.is_empty());
        history.finish(turn, None)?;

        let mut middle = prompt("accept leaf delta");
        let turn = history.begin(provider, Some(branch), &mut middle)?;
        let context = middle.attachments[0].text_context().unwrap();
        assert!(context.contains("leaf marker"));
        assert!(!context.contains("middle marker") && !context.contains("root-only marker"));
        history.finish(turn, None)?;
    }
    Ok(())
}
