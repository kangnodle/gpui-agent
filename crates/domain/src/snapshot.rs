//! Pure construction of a bounded conversation snapshot; no host I/O.
use crate::*;

#[derive(Debug)]
pub enum SnapshotError {
    MessageNotFound,
    Incomplete,
    Invalid(&'static str),
    Encoding(serde_json::Error),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            | Self::MessageNotFound => f.write_str("Source message not found"),
            | Self::Incomplete => f.write_str("Choose a completed source message; source snapshot contains an unfinished response"),
            | Self::Invalid(message) => f.write_str(message),
            | Self::Encoding(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for SnapshotError {}
impl From<serde_json::Error> for SnapshotError {
    fn from(error: serde_json::Error) -> Self { Self::Encoding(error) }
}
impl From<&'static str> for SnapshotError {
    fn from(message: &'static str) -> Self { Self::Invalid(message) }
}
pub fn conversation_snapshot(conversation: &SavedConversation, through: Option<i64>) -> Result<Vec<Attachment>, SnapshotError> {
    let end = match through {
        | Some(id) => conversation
            .messages
            .iter()
            .position(|m| m.id == Some(id))
            .map(|n| n + 1)
            .ok_or(SnapshotError::MessageNotFound)?,
        | None => conversation.messages.len(),
    };
    if end == 0 {
        return Err("Source conversation is empty".into());
    }
    let mut transcript = Vec::new();
    let mut images = Vec::new();
    for message in &conversation.messages[.. end] {
        if message.incomplete {
            return Err(SnapshotError::Incomplete);
        }
        let mut texts = vec![];
        for attachment in &message.attachments {
            if let Some(text) = attachment.text_context() {
                texts.push(text);
            } else {
                images.push(attachment.clone());
            }
        }
        let mut tools: Vec<ToolCall> = vec![];
        for entry in conversation.timeline.iter().filter(|e| Some(e.message_id) == message.id) {
            if let Event::Tool {
                call,
            } = &entry.event
            {
                if let Some(previous) = tools.iter_mut().find(|t| t.id == call.id) {
                    previous.merge(call);
                } else {
                    tools.push(call.clone());
                }
            }
        }
        transcript.push(serde_json::json!({"id":message.id,"role":if message.user{"user"}else{"assistant"},"text":message.text,"files":texts,"tools":tools}));
    }
    let text = bounded_transcript(&transcript, Some(conversation.id))?;
    let mut attachments = vec![Attachment::Text {
        name: "conversation-context.json".into(),
        text,
        source: None,
    }];
    attachments.extend(images);
    validate_attachments(&attachments)?;
    Ok(attachments)
}

pub(crate) fn bounded_transcript(items: &[serde_json::Value], source: Option<i64>) -> Result<String, SnapshotError> {
    const LIMIT: usize = 192 * 1024;
    let full = serde_json::to_string(items)?;
    if full.len() <= LIMIT {
        return Ok(full);
    }
    let excerpt = |text: &str, limit: usize| {
        if text.len() <= limit {
            return text.to_owned();
        }
        let mut start = text.len() - limit / 2;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        format!("{}\n[content omitted; original retained]\n{}", utf8_prefix(text, limit / 2), &text[start ..])
    };
    let bounded=items.iter().map(|item| {
        let raw=serde_json::to_string(item)?;
        let item=if raw.len()>LIMIT/3 {
            serde_json::json!({"id":item["id"],"role":item["role"],"truncated":true,"original_bytes":raw.len(),"text":excerpt(item["text"].as_str().unwrap_or(""),8192),"details_excerpt":excerpt(&raw,4096)})
        } else {item.clone()};
        Ok((item.clone(),serde_json::to_vec(&item)?.len()+1))
    }).collect::<Result<Vec<_>,serde_json::Error>>()?;
    let mut remaining = LIMIT - 1024;
    let mut selected = std::collections::BTreeSet::new();
    for index in std::iter::once(0).chain((0 .. bounded.len()).rev()) {
        if selected.contains(&index) {
            continue;
        }
        if let Some((_, bytes)) = bounded.get(index)
            && *bytes <= remaining
        {
            selected.insert(index);
            remaining -= bytes;
        }
    }
    let messages: Vec<_> = selected.iter().map(|index| &bounded[*index].0).collect();
    let text = serde_json::to_string(
        &serde_json::json!({"truncated":true,"source_conversation_id":source,"omitted_messages":items.len()-selected.len(),"messages":messages,"notice":"Context was shortened by selecting the objective and recent state. This is not a semantic summary. Original messages and tool results remain in source history; inspect them before relying on omitted details."}),
    )?;
    if text.len() > LIMIT {
        return Err("Cannot fit bounded source context".into());
    }
    Ok(text)
}

/// The longest prefix of at most `max_bytes` that ends on a char boundary.
fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[.. end]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_merges_tool_results_and_does_not_carry_previous_permissions() {
        let call = |status: &str, output| Event::Tool {
            call: ToolCall {
                id: "tool".into(),
                title: Some("read".into()),
                status: Some(status.into()),
                input: None,
                output,
                output_delta: None,
                activity: vec![],
            },
        };
        let c = SavedConversation {
            id: 1,
            model: None,
            session_id: Some("native-secret-session".into()),
            messages: vec![SavedMessage {
                skills: vec![],
                guidance: None,
                id: Some(2),
                user: false,
                text: "done".into(),
                incomplete: false,
                attachments: vec![],
            }],
            timeline: vec![
                TimelineEntry {
                    message_id: 2,
                    event: call("pending", None),
                },
                TimelineEntry {
                    message_id: 2,
                    event: call("completed", Some(serde_json::json!("tool result"))),
                },
                TimelineEntry {
                    message_id: 2,
                    event: Event::Permission {
                        method: "must-not-transfer-permission".into(),
                        details: serde_json::json!({"allow":true}),
                    },
                },
            ],
        };
        let context = conversation_snapshot(&c, None).unwrap()[0].text_context().unwrap();
        assert!(context.contains("tool result") && context.contains("completed"));
        assert!(!context.contains("native-secret-session") && !context.contains("must-not-transfer-permission"));
    }
    #[test]
    fn source_snapshot_respects_the_message_boundary_and_rejects_incomplete_context() {
        let message = |id, text: &str, incomplete| SavedMessage {
            skills: vec![],
            guidance: None,
            id: Some(id),
            user: id == 1,
            text: text.into(),
            incomplete,
            attachments: vec![],
        };
        let c = SavedConversation {
            id: 1,
            model: None,
            session_id: Some("must-not-transfer".into()),
            timeline: vec![],
            messages: vec![message(1, "original", false), message(2, "later", true)],
        };
        let text = conversation_snapshot(&c, Some(1)).unwrap()[0].text_context().unwrap();
        assert!(text.contains("original"));
        assert!(!text.contains("later") && !text.contains("must-not-transfer"));
        assert!(matches!(conversation_snapshot(&c, None), Err(SnapshotError::Incomplete)));
        assert!(matches!(conversation_snapshot(&c, Some(9)), Err(SnapshotError::MessageNotFound)));
    }

    #[test]
    fn context_limits_mark_shortened_text_and_reject_excess_images() {
        let mut conversation = SavedConversation {
            id: 1,
            model: None,
            session_id: None,
            timeline: vec![],
            messages: vec![],
        };
        assert!(matches!(conversation_snapshot(&conversation, None), Err(SnapshotError::Invalid(_))));
        conversation.messages.push(SavedMessage {
            skills: vec![],
            guidance: None,
            id: Some(1),
            user: true,
            text: "x".repeat(192 * 1024),
            incomplete: false,
            attachments: vec![],
        });
        let snapshot = conversation_snapshot(&conversation, None).unwrap();
        let Attachment::Text {
            text, ..
        } = &snapshot[0]
        else {
            panic!("Expected text")
        };
        assert!(text.contains("\"truncated\":true"));
        conversation.messages[0].text = "short".into();
        let image = Attachment::Image {
            name: "image.png".into(),
            mime_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };
        conversation.messages[0].attachments = vec![image.clone(); 7];
        assert_eq!(conversation_snapshot(&conversation, None).unwrap().len(), 8);
        conversation.messages[0].attachments.push(image);
        assert!(matches!(conversation_snapshot(&conversation, None), Err(SnapshotError::Invalid(_))));
    }
}
