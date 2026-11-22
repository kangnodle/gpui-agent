use crate::{ApplicationResult,
            Jobs};
use gpui_agent_contracts::{ConversationSendRequest,
                           ConversationTarget};
use gpui_agent_dci::proposal_governance::ProposalOriginContext;
use gpui_agent_domain::McpScope;

impl Jobs {
    pub async fn latest_user_request(&self, scope: &McpScope, conversation_id: i64) -> ApplicationResult<Option<String>> {
        let (scope, context) = self
            .conversation_scope(
                Some(scope),
                ConversationTarget {
                    provider: scope.provider,
                    account: Some(scope.account_id.clone()),
                    workspace: None,
                },
            )
            .await?;
        let conversation = context.application.conversation(scope.provider, conversation_id).await?;
        Ok(ProposalOriginContext::latest_user_request(&conversation))
    }

    pub async fn continue_after_proposal(&self, scope: &McpScope, conversation_id: i64, proposal_id: &str, text: String) -> ApplicationResult<()> {
        self.conversation_send(
            Some(scope),
            ConversationSendRequest {
                skills: vec![],
                target: ConversationTarget {
                    provider: scope.provider,
                    account: Some(scope.account_id.clone()),
                    workspace: None,
                },
                id: Some(conversation_id),
                text,
                attachments: vec![],
                attachment_ids: vec![],
                model: None,
                command_id: format!("proposal-{proposal_id}"),
            },
        )
        .await
        .map(|_| ())
    }
}
