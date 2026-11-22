use crate::{ApplicationResult,
            Jobs};
use gpui_agent_contracts::*;

impl Jobs {
    pub async fn upload_attachment(&self, caller: Option<&McpScope>, request: AttachmentUploadRequest) -> ApplicationResult<UploadedAttachment> {
        request.attachment.validate()?;
        let (scope, _context) = self.conversation_scope(caller, request.target).await?;
        self.io(move |repo| repo.upload_attachment(&scope, &request.attachment)).await
    }

    pub async fn attachment_from_workspace(&self, caller: Option<&McpScope>, request: WorkspaceAttachmentRequest) -> ApplicationResult<UploadedAttachment> {
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        let attachment = context
            .application
            .file_attachment(FileContextRequest {
                path: request.path,
                start_line: request.start_line,
                end_line: request.end_line,
            })
            .await?;
        self.io(move |repo| repo.upload_attachment(&scope, &attachment)).await
    }

    pub async fn uploaded_attachments(&self, caller: Option<&McpScope>, request: AttachmentListRequest) -> ApplicationResult<Vec<UploadedAttachment>> {
        let (scope, _context) = self.conversation_scope(caller, request.target).await?;
        self.io(move |repo| repo.uploaded_attachments(&scope)).await
    }

    pub async fn delete_attachment(&self, caller: Option<&McpScope>, request: AttachmentDeleteRequest) -> ApplicationResult<()> {
        if request.id <= 0 {
            return Err("Attachment ID must be positive".into());
        }
        let (scope, _context) = self.conversation_scope(caller, request.target).await?;
        self.io(move |repo| repo.delete_attachment(&scope, request.id)).await
    }
}
