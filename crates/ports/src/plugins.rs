use anyhow::Result;
use gpui_agent_domain::{McpServer,
                        PluginCatalog,
                        PluginImportRequest,
                        PluginSyncResult};

pub trait PluginSource: Send + Sync {
    fn catalog(&self) -> Result<PluginCatalog>;
    fn import(&self, request: &PluginImportRequest) -> Result<PluginCatalog>;
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<PluginCatalog> { self.set_enabled_reviewed(id, enabled, None) }
    fn set_enabled_reviewed(&self, id: &str, enabled: bool, expected: Option<&[McpServer]>) -> Result<PluginCatalog>;
    fn remove(&self, id: &str) -> Result<PluginCatalog>;
    /// Imports plugins installed by the Claude provider profile that are not
    /// installed here yet.
    fn sync_claude(&self, scope: &str, enable: bool) -> Result<PluginSyncResult>;
}
