use anyhow::Result;
use gpui_agent_domain::{Completion,
                        ConversationSummary,
                        ConversationVersion,
                        Event,
                        Prompt,
                        Provider,
                        SavedConversation};
#[derive(Clone, Copy, Debug)]
pub struct Turn {
    pub conversation_id: i64,
    pub message_id: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditOutcome {
    Done,
    NotFound,
    Busy,
}

pub enum ForkOutcome {
    Created(i64),
    NotFound,
    Busy,
}

/// Implementations atomically reserve a turn and enforce provider/workspace
/// ownership. Event text is persisted before delivery; finish releases the
/// reservation on every terminal outcome. `begin` restores the authoritative
/// session into the prompt; `finish(None)` preserves an incomplete response.
pub trait ConversationRepository: Send + Sync {
    fn begin_repair(&self, _provider: Provider, _conversation: i64, _prompt: &mut Prompt) -> Result<Turn> {
        anyhow::bail!("Internal verification repairs are unavailable")
    }
    fn list(&self, provider: Provider, search: &str) -> Result<Vec<ConversationSummary>>;
    fn load(&self, provider: Provider, id: i64) -> Result<Option<SavedConversation>>;
    fn transfer(&self, provider: Provider, command: &gpui_agent_domain::ContextTransferCommand) -> Result<gpui_agent_domain::ContextTransfer>;
    fn transfers(&self, provider: Provider, id: i64, after: i64) -> Result<Vec<gpui_agent_domain::ContextTransfer>>;
    fn fork(&self, provider: Provider, id: i64, message_id: i64, title: &str, context: &[gpui_agent_domain::Attachment]) -> Result<ForkOutcome>;
    fn versions(&self, provider: Provider, id: i64) -> Result<Vec<ConversationVersion>>;
    fn begin(&self, provider: Provider, conversation_id: Option<i64>, prompt: &mut Prompt) -> Result<Turn>;
    fn rename(&self, provider: Provider, id: i64, title: &str) -> Result<EditOutcome>;
    fn set_state(&self, provider: Provider, id: i64, state: &gpui_agent_domain::ConversationState) -> Result<EditOutcome>;
    fn delete(&self, provider: Provider, id: i64) -> Result<EditOutcome>;
    fn event(&self, turn: Turn, event: &Event) -> Result<()>;
    fn finish(&self, turn: Turn, completion: Option<&Completion>) -> Result<()>;
}
