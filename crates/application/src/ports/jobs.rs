use gpui_agent_domain::Provider;
/// Durable task journal. Claims and queued edits must be atomic.
pub trait JobRepository: Send + Sync {
    fn mcp_oauth(
        &self,
        _scope: &gpui_agent_domain::JobScope,
        _name: &str,
        _command: gpui_agent_domain::McpOAuthCommand,
    ) -> crate::ApplicationResult<gpui_agent_domain::McpOAuthStatus> {
        Err(crate::ApplicationError(crate::ErrorKind::Unavailable, "MCP OAuth is unavailable".into()))
    }
    fn cancel_mcp_oauth(&self) {}
    fn pull_request_guide(
        &self,
        scope: &gpui_agent_domain::JobScope,
        target: &gpui_agent_domain::PullRequestTarget,
    ) -> crate::ApplicationResult<Option<gpui_agent_domain::PullRequestGuide>>;
    /// Atomically reuse a successful/active head/base run, or enqueue and save
    /// a new one.
    fn start_pull_request_guide(
        &self,
        scope: &gpui_agent_domain::JobScope,
        target: &gpui_agent_domain::PullRequestTarget,
        head: &str,
        base: &str,
        request: &gpui_agent_domain::ChatRequest,
    ) -> crate::ApplicationResult<(gpui_agent_domain::PullRequestGuide, bool)>;
    fn due_pull_requests(&self, now: i64) -> crate::ApplicationResult<Vec<gpui_agent_domain::PullRequestLink>>;
    /// Compare-and-swap the baseline and enqueue its notification in one
    /// transaction.
    fn apply_pull_request_poll(
        &self,
        previous: &gpui_agent_domain::PullRequestLink,
        next: &gpui_agent_domain::PullRequestLink,
        notification: Option<&gpui_agent_domain::ChatRequest>,
    ) -> crate::ApplicationResult<Option<gpui_agent_domain::Job>>;
    fn stop_pull_request_watches(&self, scope: &gpui_agent_domain::JobScope, conversation: i64) -> crate::ApplicationResult<()>;
    fn set_pull_request_watch(
        &self,
        scope: &gpui_agent_domain::JobScope,
        conversation: i64,
        target: &gpui_agent_domain::PullRequestTarget,
        baseline: Option<&gpui_agent_domain::PullRequestData>,
        now: i64,
    ) -> crate::ApplicationResult<bool>;
    fn pull_request_links(&self, scope: &gpui_agent_domain::JobScope, conversation: i64) -> crate::ApplicationResult<Vec<gpui_agent_domain::PullRequestLink>>;
    fn set_pull_request_link(
        &self,
        scope: &gpui_agent_domain::JobScope,
        conversation: i64,
        target: &gpui_agent_domain::PullRequestTarget,
        linked: bool,
    ) -> crate::ApplicationResult<bool>;
    fn upload_attachment(
        &self,
        scope: &gpui_agent_domain::JobScope,
        attachment: &gpui_agent_domain::Attachment,
    ) -> crate::ApplicationResult<gpui_agent_domain::UploadedAttachment>;
    fn uploaded_attachments(&self, scope: &gpui_agent_domain::JobScope) -> crate::ApplicationResult<Vec<gpui_agent_domain::UploadedAttachment>>;
    fn delete_attachment(&self, scope: &gpui_agent_domain::JobScope, id: i64) -> crate::ApplicationResult<()>;
    fn enqueue_uploaded_once(
        &self,
        scope: &gpui_agent_domain::JobScope,
        request: &gpui_agent_domain::ChatRequest,
        ids: &[i64],
        command_id: &str,
    ) -> crate::ApplicationResult<gpui_agent_domain::Job>;
    fn mcp(
        &self,
        scope: &gpui_agent_domain::JobScope,
        servers: Option<&[gpui_agent_domain::McpServer]>,
    ) -> crate::ApplicationResult<Vec<gpui_agent_domain::McpServer>>;
    fn accounts(&self, provider: Provider) -> crate::ApplicationResult<Vec<gpui_agent_domain::AccountProfile>>;
    fn create_account(&self, provider: Provider, label: &str) -> crate::ApplicationResult<gpui_agent_domain::AccountProfile>;
    fn application(&self, scope: &gpui_agent_domain::JobScope) -> crate::ApplicationResult<crate::Application>;
    fn enqueue(&self, scope: &gpui_agent_domain::JobScope, request: &gpui_agent_domain::ChatRequest) -> crate::ApplicationResult<gpui_agent_domain::Job>;
    fn enqueue_once(
        &self,
        scope: &gpui_agent_domain::JobScope,
        request: &gpui_agent_domain::ChatRequest,
        command_id: &str,
    ) -> crate::ApplicationResult<gpui_agent_domain::Job>;
    fn list(&self) -> crate::ApplicationResult<Vec<gpui_agent_domain::Job>>;
    fn get(&self, id: i64) -> crate::ApplicationResult<gpui_agent_contracts::JobDetail>;
    fn job(&self, id: i64) -> crate::ApplicationResult<gpui_agent_domain::Job>;
    fn conversation_job(
        &self,
        scope: &gpui_agent_domain::JobScope,
        conversation: i64,
        job_id: Option<i64>,
        active_only: bool,
    ) -> crate::ApplicationResult<Option<gpui_agent_domain::Job>>;
    fn claim(&self, scope: &gpui_agent_domain::JobScope) -> crate::ApplicationResult<Option<gpui_agent_contracts::JobDetail>>;
    fn append(&self, id: i64, event: &gpui_agent_contracts::StreamEvent) -> crate::ApplicationResult<()>;
    fn append_batch(&self, id: i64, events: &[gpui_agent_contracts::StreamEvent]) -> crate::ApplicationResult<()>;
    fn events(&self, id: i64, after: i64) -> crate::ApplicationResult<Vec<gpui_agent_contracts::JobEvent>>;
    fn cancel_queued(&self, id: i64) -> crate::ApplicationResult<bool>;
    fn purge_conversation(&self, provider: Provider, account: &str, workspace: i64, conversation: i64) -> crate::ApplicationResult<()>;
    fn workspace_busy(&self, id: i64) -> crate::ApplicationResult<bool>;
    fn workspace_has_uploads(&self, id: i64) -> crate::ApplicationResult<bool>;
    fn cancel_all(&self);
}
