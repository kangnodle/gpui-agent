use crate::{Application,
            ApplicationResult,
            Jobs};
use gpui_agent_contracts::*;

impl Application {
    pub async fn context_transfer(&self, provider: Provider, command: ContextTransferCommand) -> ApplicationResult<ContextTransfer> {
        command.validate()?;
        let history = self.0.history.clone();
        tokio::task::spawn_blocking(move || history.transfer(provider, &command))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(Into::into)
    }

    pub async fn context_transfers(&self, provider: Provider, id: i64, after: i64) -> ApplicationResult<Vec<ContextTransfer>> {
        if id <= 0 || after < 0 {
            return Err("Conversation ID must be positive and cursor nonnegative".into());
        }
        let history = self.0.history.clone();
        tokio::task::spawn_blocking(move || history.transfers(provider, id, after))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(Into::into)
    }
}

impl Jobs {
    pub async fn conversation_fork(&self, caller: Option<&McpScope>, request: ConversationForkRequest) -> ApplicationResult<ContextTransfer> {
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        context
            .application
            .context_transfer(
                scope.provider,
                ContextTransferCommand {
                    source_id: request.id,
                    target_id: None,
                    through_message_id: request.through_message_id,
                    title: request.title,
                    command_id: request.command_id,
                },
            )
            .await
    }

    pub async fn conversation_merge_back(&self, caller: Option<&McpScope>, request: ConversationMergeBackRequest) -> ApplicationResult<ContextTransfer> {
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        context
            .application
            .context_transfer(
                scope.provider,
                ContextTransferCommand {
                    source_id: request.source_id,
                    target_id: Some(request.target_id),
                    through_message_id: request.through_message_id,
                    title: None,
                    command_id: request.command_id,
                },
            )
            .await
    }

    pub async fn conversation_transfers(&self, caller: Option<&McpScope>, request: ConversationTransfersRequest) -> ApplicationResult<Vec<ContextTransfer>> {
        let (scope, context) = self.conversation_scope(caller, request.target).await?;
        context.application.context_transfers(scope.provider, request.id, request.after).await
    }
}
