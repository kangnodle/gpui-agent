use crate::{ApplicationError,
            ApplicationResult,
            ErrorKind,
            Jobs};
use gpui_agent_contracts::*;
impl Jobs {
    pub async fn mcp_oauth(
        &self,
        provider: Provider,
        account: String,
        workspace: Option<i64>,
        name: String,
        command: McpOAuthCommand,
    ) -> ApplicationResult<McpOAuthStatus> {
        let (scope, _lease) = self.resolve(provider, account, workspace).await?;
        self.io(move |repository| repository.mcp_oauth(&scope, &name, command)).await
    }

    pub async fn common_mcp(&self, workspace: Option<i64>, servers: Option<Vec<McpServer>>) -> ApplicationResult<Vec<McpServer>> {
        self.mcp(Provider::Claude, "default".into(), workspace, servers).await
    }

    pub async fn mcp(&self, provider: Provider, account: String, workspace: Option<i64>, servers: Option<Vec<McpServer>>) -> ApplicationResult<Vec<McpServer>> {
        if let Some(servers) = &servers {
            validate_mcp(servers).map_err(|e| ApplicationError(ErrorKind::Invalid, e))?;
        }
        let (scope, _lease) = self.resolve(provider, account, workspace).await?;
        self.io(move |r| r.mcp(&scope, servers.as_deref())).await
    }
}
