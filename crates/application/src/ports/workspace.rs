use gpui_agent_domain::Provider;
/// Host-owned project registry, Git operations and per-directory application
/// composition.
pub trait WorkspaceBackend: Send + Sync {
    fn set_project_verification(
        &self,
        _project: i64,
        _plan: &gpui_agent_domain::ProjectVerification,
    ) -> crate::ApplicationResult<gpui_agent_domain::ProjectCatalog> {
        Err("Project verification settings unavailable".into())
    }
    fn pull_request(
        &self,
        _workspace: i64,
        _target: &gpui_agent_domain::PullRequestTarget,
        _kind: gpui_agent_domain::PullRequestReadKind,
    ) -> crate::ApplicationResult<gpui_agent_domain::PullRequestData> {
        Err(crate::ApplicationError(crate::ErrorKind::Unavailable, "PR reads unavailable".into()))
    }
    fn environment(&self) -> crate::ApplicationResult<gpui_agent_domain::EnvironmentDescriptor> {
        Err(crate::ApplicationError(
            crate::ErrorKind::Unavailable,
            "Environment settings unavailable".into(),
        ))
    }
    fn update_environment(
        &self,
        _patch: gpui_agent_domain::EnvironmentPreferencesPatch,
    ) -> crate::ApplicationResult<gpui_agent_domain::EnvironmentPreferences> {
        Err(crate::ApplicationError(
            crate::ErrorKind::Unavailable,
            "Environment settings unavailable".into(),
        ))
    }
    fn runtime(&self, provider: Provider, command: &gpui_agent_domain::RuntimeCommand) -> crate::ApplicationResult<gpui_agent_domain::RuntimeInfo>;
    fn git(&self, workspace: i64, command: &gpui_agent_domain::GitCommand) -> crate::ApplicationResult<gpui_agent_domain::GitOverview>;
    fn catalog(&self) -> crate::ApplicationResult<gpui_agent_domain::ProjectCatalog>;
    fn register(&self, path: &str) -> crate::ApplicationResult<gpui_agent_domain::ProjectCatalog>;
    fn forget_project(&self, project: i64) -> crate::ApplicationResult<gpui_agent_domain::ProjectCatalog>;
    fn create_worktree(&self, project: i64, branch: &str, base: &str) -> crate::ApplicationResult<gpui_agent_domain::ProjectCatalog>;
    fn remove_worktree(&self, workspace: i64) -> crate::ApplicationResult<gpui_agent_domain::ProjectCatalog>;
    fn changes(&self, workspace: i64) -> crate::ApplicationResult<gpui_agent_domain::GitChanges>;
    fn diff(&self, workspace: i64, path: &str) -> crate::ApplicationResult<gpui_agent_domain::FileDiff>;
    fn application(&self, workspace: Option<i64>) -> crate::ApplicationResult<crate::Application>;
    fn active_application(&self, workspace: Option<i64>) -> Option<crate::Application>;
    fn cancel_all(&self);
}
