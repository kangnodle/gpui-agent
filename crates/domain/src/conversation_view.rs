use crate::{ConversationSummary,
            SavedConversation};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationArchiveFilter {
    #[default]
    Active,
    Archived,
    All,
}

#[derive(Debug, Serialize)]
pub struct ConversationListPage {
    pub items: Vec<ConversationSummary>,
    pub next_offset: Option<usize>,
}

impl ConversationListPage {
    pub fn new(items: Vec<ConversationSummary>, archived: ConversationArchiveFilter, offset: usize, limit: usize) -> Result<Self, &'static str> {
        if !(1 ..= 50).contains(&limit) {
            return Err("Conversation list limit must be 1–50");
        }
        let mut items: Vec<_> = items
            .into_iter()
            .filter(|item| match archived {
                | ConversationArchiveFilter::Active => !item.archived,
                | ConversationArchiveFilter::Archived => item.archived,
                | ConversationArchiveFilter::All => true,
            })
            .skip(offset)
            .take(limit + 1)
            .collect();
        let next_offset = (items.len() > limit).then(|| offset + limit);
        items.truncate(limit);
        Ok(Self {
            items,
            next_offset,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationCursor {
    pub message: usize,
    pub text: usize,
}

#[derive(Debug, Serialize)]
pub struct ConversationMessage {
    pub id: Option<i64>,
    pub user: bool,
    pub text: String,
    pub incomplete: bool,
    pub attachment_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ConversationPage {
    pub id: i64,
    pub model: Option<String>,
    pub messages: Vec<ConversationMessage>,
    pub next_cursor: Option<ConversationCursor>,
}

impl SavedConversation {
    pub fn page(&self, cursor: ConversationCursor, limit: usize) -> Result<ConversationPage, &'static str> {
        if !(1 ..= 20).contains(&limit) || cursor.message > self.messages.len() || (cursor.message == self.messages.len() && cursor.text != 0) {
            return Err("Invalid conversation cursor or message limit (1–20)");
        }
        let mut page = ConversationPage {
            id: self.id,
            model: self.model.clone(),
            messages: Vec::new(),
            next_cursor: None,
        };
        let mut remaining = 16_000;
        for (index, message) in self.messages.iter().enumerate().skip(cursor.message) {
            let start = if index == cursor.message { cursor.text } else { 0 };
            let total = message.text.chars().count();
            if start > total {
                return Err("Text cursor is outside the message");
            }
            let text: String = message.text.chars().skip(start).take(remaining).collect();
            let consumed = text.chars().count();
            remaining -= consumed;
            page.messages.push(ConversationMessage {
                id: message.id,
                user: message.user,
                text,
                incomplete: message.incomplete,
                attachment_count: message.attachments.len(),
            });
            if start + consumed < total {
                page.next_cursor = Some(ConversationCursor {
                    message: index,
                    text: start + consumed,
                });
                break;
            }
            if page.messages.len() == limit || remaining == 0 {
                if index + 1 < self.messages.len() {
                    page.next_cursor = Some(ConversationCursor {
                        message: index + 1,
                        text: 0,
                    });
                }
                break;
            }
        }
        Ok(page)
    }
}
