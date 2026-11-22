use anyhow::Result;

pub trait FileContextReader: Send + Sync {
    fn read(&self, request: &gpui_agent_domain::FileContextRequest) -> Result<gpui_agent_domain::Attachment>;
    fn attachment(&self, request: &gpui_agent_domain::FileContextRequest) -> Result<gpui_agent_domain::Attachment> { self.read(request) }
    fn list(&self, request: &gpui_agent_domain::WorkspaceFilesRequest) -> Result<gpui_agent_domain::WorkspaceFiles>;
}
