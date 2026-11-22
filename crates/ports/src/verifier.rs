/// Runs a project's required verification commands for ordinary jobs.
pub trait ProjectVerifier: Send + Sync {
    fn revision<'a>(&'a self, _scope: &'a gpui_agent_domain::JobScope) -> crate::RuntimeFuture<'a, gpui_agent_domain::CodeVersion> {
        Box::pin(async { anyhow::bail!("Project version capture is unavailable") })
    }
    fn check<'a>(
        &'a self,
        _scope: &'a gpui_agent_domain::JobScope,
        _command: &'a [String],
        _timeout_seconds: u32,
    ) -> crate::RuntimeFuture<'a, gpui_agent_domain::VerificationCheck> {
        Box::pin(async { anyhow::bail!("Project verification is unavailable") })
    }
}
