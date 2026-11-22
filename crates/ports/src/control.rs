use super::RuntimeFuture;
pub type ControlValidity = std::sync::Arc<dyn Fn() -> bool + Send + Sync>;

/// Rechecked at the native input boundary, including after waiting for its
/// lock.
#[derive(Clone)]
pub struct ComputerSession {
    pub job_id: i64,
    pub execution_id: u64,
    pub active: ControlValidity,
}
/// Host-authenticated controls. Implementations own browser processes, isolate
/// automation scopes, limit inputs and require an expiring manual desktop
/// grant.
pub trait HostControl: Send + Sync {
    fn automatic_computer_enabled(&self) -> bool { false }
    fn computer_status(&self) -> RuntimeFuture<'_, gpui_agent_contracts::ComputerStatus> {
        Box::pin(async { Ok(gpui_agent_contracts::ComputerStatus::default()) })
    }
    fn computer(
        &self,
        _session: ComputerSession,
        _command: gpui_agent_contracts::ComputerCommand,
    ) -> RuntimeFuture<'_, Option<gpui_agent_contracts::ComputerFrame>> {
        Box::pin(async { anyhow::bail!("Windows computer use is unavailable on this host") })
    }
    fn grant_computer(&self, _grant: gpui_agent_contracts::ComputerGrantRequest, _active: ControlValidity) -> RuntimeFuture<'_, ()> {
        Box::pin(async { anyhow::bail!("Windows computer use is unavailable on this host") })
    }
    fn device(
        &self,
        _scope: Option<gpui_agent_contracts::McpScope>,
        _command: gpui_agent_contracts::DeviceCommand,
    ) -> RuntimeFuture<'_, gpui_agent_contracts::DeviceReply> {
        Box::pin(async { anyhow::bail!("Android device control is unavailable on this host") })
    }
    fn grant_device(&self, _grant: gpui_agent_contracts::DeviceGrantRequest) -> RuntimeFuture<'_, ()> {
        Box::pin(async { anyhow::bail!("Android device control is unavailable on this host") })
    }
    /// Owned automation contexts must be isolated by scope and from manual
    /// controls.
    fn automate(
        &self,
        _scope: Option<gpui_agent_contracts::McpScope>,
        _command: gpui_agent_contracts::BrowserCommand,
    ) -> RuntimeFuture<'_, gpui_agent_contracts::BrowserReply> {
        Box::pin(async { anyhow::bail!("Browser automation is unavailable on this host") })
    }
    fn status(&self) -> RuntimeFuture<'_, gpui_agent_contracts::ControlStatus>;
    fn browser(&self, command: gpui_agent_contracts::ControlCommand) -> RuntimeFuture<'_, Option<gpui_agent_contracts::ControlFrame>>;
    fn desktop(&self, command: gpui_agent_contracts::ControlCommand) -> RuntimeFuture<'_, Option<gpui_agent_contracts::ControlFrame>>;
    fn enable_desktop(&self, enabled: bool) -> RuntimeFuture<'_, ()>;
    fn shutdown(&self);
}
