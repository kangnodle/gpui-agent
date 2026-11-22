use anyhow::Result;
use gpui_agent_domain::McpGrant;

pub trait McpDiagnosticSink: Send + Sync {
    fn record(&self, record: &gpui_agent_domain::McpDiagnostic);
}

/// Transient runtime input. Never serialize into prompts or provider settings.
#[derive(Clone)]
pub struct McpConnection {
    pub url: String,
    pub token: String,
}
impl std::fmt::Debug for McpConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpConnection")
            .field("url", &self.url)
            .field("token", &"[redacted]")
            .finish()
    }
}

pub trait McpSessionAuthorizer: Send + Sync {
    fn resolve(&self, token: &str) -> Result<Option<McpGrant>>;
    /// Revoke credentials and close admission before shutting down the host.
    fn revoke_all(&self);
}

pub trait McpSessionLease: Send {
    fn connection(&self) -> &McpConnection;
}

pub trait McpSessionIssuer: Send + Sync {
    fn issue_for_job(&self, conversation: Option<i64>, _job: Option<i64>) -> Result<Box<dyn McpSessionLease>> { self.issue_in_conversation(conversation) }
    fn issue(&self) -> Result<Box<dyn McpSessionLease>>;
    fn issue_in_conversation(&self, _conversation: Option<i64>) -> Result<Box<dyn McpSessionLease>> { self.issue() }
}
