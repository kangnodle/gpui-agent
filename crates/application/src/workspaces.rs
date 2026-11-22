use crate::{Application,
            ApplicationError,
            ApplicationResult,
            ErrorKind,
            ports::WorkspaceBackend};
use gpui_agent_domain::{FileDiff,
                        GitChanges,
                        ProjectCatalog};
use std::sync::Arc;
use tokio::sync::{OwnedRwLockReadGuard,
                  RwLock};

#[derive(Clone)]
pub struct Workspaces {
    backend: Arc<dyn WorkspaceBackend>,
    gate: Arc<RwLock<()>>,
    default_workspace: Option<i64>,
}
pub struct WorkspaceScope {
    pub application: Application,
    // Keep the directory alive until the request has reserved its operation.
    _lease: Arc<OwnedRwLockReadGuard<()>>,
}
impl Workspaces {
    pub async fn verification_plan(&self, workspace: Option<i64>) -> ApplicationResult<gpui_agent_domain::ProjectVerification> {
        let backend = self.backend.clone();
        let workspace = workspace.or(self.default_workspace);
        // HTTP middleware already retains the workspace lease; do not nest fair
        // read locks.
        blocking(move || {
            let catalog = backend.catalog()?;
            let id = workspace.or(catalog.default_workspace_id);
            catalog
                .projects
                .into_iter()
                .find(|project| project.workspaces.iter().any(|entry| Some(entry.id) == id && !entry.missing))
                .map(|project| project.verification)
                .ok_or_else(|| ApplicationError(ErrorKind::NotFound, "Project not found".into()))
        })
        .await
    }

    pub async fn set_project_verification(
        &self,
        caller: Option<&gpui_agent_domain::McpScope>,
        project: i64,
        plan: gpui_agent_domain::ProjectVerification,
    ) -> ApplicationResult<ProjectCatalog> {
        if caller.is_some() {
            return Err("Project verification requires an operator credential".into());
        }
        plan.validate()?;
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.set_project_verification(project, &plan)
        })
        .await
    }

    pub fn new(default_workspace: Option<i64>, backend: Arc<dyn WorkspaceBackend>) -> Self {
        Self {
            backend,
            gate: Arc::new(RwLock::new(())),
            default_workspace,
        }
    }

    pub fn default_workspace(&self) -> Option<i64> { self.default_workspace }

    pub async fn environment_read(&self) -> ApplicationResult<gpui_agent_domain::EnvironmentDescriptor> {
        let lease = self.gate.clone().read_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.environment()
        })
        .await
    }

    pub async fn environment_preferences_update(
        &self,
        caller: Option<&gpui_agent_domain::McpScope>,
        patch: gpui_agent_domain::EnvironmentPreferencesPatch,
    ) -> ApplicationResult<gpui_agent_domain::EnvironmentPreferences> {
        if caller.is_some() {
            return Err("Environment preferences require an operator credential".into());
        }
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.update_environment(patch)
        })
        .await
    }

    pub async fn scope(&self, workspace: Option<i64>) -> ApplicationResult<WorkspaceScope> {
        let lease = self.gate.clone().read_owned().await;
        let backend = self.backend.clone();
        let id = workspace.or(self.default_workspace);
        let application = blocking(move || backend.application(id)).await?;
        Ok(WorkspaceScope {
            application,
            _lease: Arc::new(lease),
        })
    }

    pub async fn git(
        &self,
        id: i64,
        command: gpui_agent_domain::GitCommand,
        check: impl FnOnce() -> ApplicationResult<()> + Send + 'static,
    ) -> ApplicationResult<gpui_agent_domain::GitOverview> {
        command.validate()?;
        if matches!(
            command,
            gpui_agent_domain::GitCommand::Inspect | gpui_agent_domain::GitCommand::ProposeCommit { .. }
        ) {
            return self.read_workspace(Some(id), None, move |backend, id| backend.git(id, &command)).await;
        }
        let lease = tokio::time::timeout(std::time::Duration::from_secs(2), self.gate.clone().write_owned())
            .await
            .map_err(|_| ApplicationError(ErrorKind::Busy, "Finish the active workspace operation before changing Git state".into()))?;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            check()?;
            backend.git(id, &command)
        })
        .await
    }

    pub async fn runtime(
        &self,
        provider: gpui_agent_domain::Provider,
        command: gpui_agent_domain::RuntimeCommand,
        check: impl FnOnce() -> ApplicationResult<()> + Send + 'static,
    ) -> ApplicationResult<gpui_agent_domain::RuntimeInfo> {
        command.validate()?;
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            check()?;
            backend.runtime(provider, &command)
        })
        .await
    }

    pub(crate) async fn catalog_in_scope(&self, _scope: &WorkspaceScope) -> ApplicationResult<ProjectCatalog> {
        let backend = self.backend.clone();
        blocking(move || backend.catalog()).await
    }

    pub async fn catalog(&self) -> ApplicationResult<ProjectCatalog> {
        let lease = self.gate.clone().read_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.catalog()
        })
        .await
    }

    pub async fn catalog_for_session(&self, session: Option<&gpui_agent_domain::McpScope>) -> ApplicationResult<ProjectCatalog> {
        let catalog = self.catalog().await?;
        match session {
            | Some(session) => catalog.for_workspace(&session.workspace_path).ok_or_else(workspace_not_found),
            | None => Ok(catalog),
        }
    }

    pub async fn changes_for_session(&self, workspace: Option<i64>, session: Option<&gpui_agent_domain::McpScope>) -> ApplicationResult<GitChanges> {
        self.read_workspace(workspace, session, |backend, id| backend.changes(id)).await
    }

    pub async fn propose_commit_for_session(
        &self,
        workspace: Option<i64>,
        session: Option<&gpui_agent_domain::McpScope>,
        message: String,
        paths: Vec<String>,
    ) -> ApplicationResult<gpui_agent_domain::GitOverview> {
        let command = gpui_agent_domain::GitCommand::ProposeCommit {
            message,
            paths,
        };
        command.validate()?;
        self.read_workspace(workspace, session, move |backend, id| backend.git(id, &command)).await
    }

    pub async fn pull_request(
        &self,
        scope: &gpui_agent_domain::JobScope,
        lease: &WorkspaceScope,
        target: gpui_agent_domain::PullRequestTarget,
        kind: gpui_agent_domain::PullRequestReadKind,
    ) -> ApplicationResult<gpui_agent_domain::PullRequestData> {
        target.validate()?;
        let session = gpui_agent_domain::McpScope {
            provider: scope.provider,
            account_id: scope.account_id.clone(),
            workspace_path: scope.workspace_path.clone(),
        };
        self.read_workspace_with_lease(scope.workspace_id, Some(&session), lease._lease.clone(), move |backend, id| {
            backend.pull_request(id, &target, kind)
        })
        .await
    }

    pub async fn diff_for_session(&self, workspace: Option<i64>, session: Option<&gpui_agent_domain::McpScope>, path: String) -> ApplicationResult<FileDiff> {
        validate_diff_path(&path)?;
        self.read_workspace(workspace, session, move |backend, id| backend.diff(id, &path)).await
    }

    async fn read_workspace<T: Send + 'static>(
        &self,
        workspace: Option<i64>,
        session: Option<&gpui_agent_domain::McpScope>,
        read: impl FnOnce(&dyn WorkspaceBackend, i64) -> ApplicationResult<T> + Send + 'static,
    ) -> ApplicationResult<T> {
        let lease = Arc::new(self.gate.clone().read_owned().await);
        self.read_workspace_with_lease(workspace, session, lease, read).await
    }

    async fn read_workspace_with_lease<T: Send + 'static>(
        &self,
        workspace: Option<i64>,
        session: Option<&gpui_agent_domain::McpScope>,
        lease: Arc<OwnedRwLockReadGuard<()>>,
        read: impl FnOnce(&dyn WorkspaceBackend, i64) -> ApplicationResult<T> + Send + 'static,
    ) -> ApplicationResult<T> {
        let backend = self.backend.clone();
        let session = session.cloned();
        blocking(move || {
            let _lease = lease;
            let mut catalog = backend.catalog()?;
            if let Some(session) = session {
                catalog = catalog.for_workspace(&session.workspace_path).ok_or_else(workspace_not_found)?;
            }
            let id = workspace.or(catalog.default_workspace_id).ok_or_else(workspace_not_found)?;
            if !catalog
                .projects
                .iter()
                .flat_map(|project| &project.workspaces)
                .any(|workspace| workspace.id == id && !workspace.missing)
            {
                return Err(workspace_not_found());
            }
            read(backend.as_ref(), id)
        })
        .await
    }

    pub async fn register(&self, path: String) -> ApplicationResult<ProjectCatalog> {
        if path.trim().is_empty() || path.len() > 32768 || path.contains('\0') {
            return Err(ApplicationError(ErrorKind::Invalid, "Invalid project path".into()));
        }
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.register(&path)
        })
        .await
    }

    pub async fn forget_project(&self, id: i64, check: impl FnOnce(&[i64]) -> ApplicationResult<()> + Send + 'static) -> ApplicationResult<ProjectCatalog> {
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            let catalog = backend.catalog()?;
            let project = catalog
                .projects
                .iter()
                .find(|project| project.id == id)
                .ok_or_else(|| ApplicationError(ErrorKind::NotFound, "Project not found".into()))?;
            check(&project.workspaces.iter().map(|workspace| workspace.id).collect::<Vec<_>>())?;
            backend.forget_project(id)
        })
        .await
    }

    pub async fn forget_project_if_idle(&self, id: i64, jobs: Option<crate::Jobs>) -> ApplicationResult<ProjectCatalog> {
        self.forget_project(id, move |workspaces| {
            for workspace in workspaces {
                require_idle(jobs.as_ref(), *workspace)?;
            }
            Ok(())
        })
        .await
    }

    pub async fn remove_if_idle(&self, id: i64, jobs: Option<crate::Jobs>) -> ApplicationResult<ProjectCatalog> {
        self.remove_checked(id, move || {
            require_idle(jobs.as_ref(), id)?;
            if let Some(jobs) = jobs
                && jobs.workspace_has_uploads(id)?
            {
                return Err(ApplicationError(
                    ErrorKind::Busy,
                    "Workspace has staged attachments; delete its uploads before removing the worktree".into(),
                ));
            }
            Ok(())
        })
        .await
    }

    pub async fn create(&self, project: i64, branch: String, base: String) -> ApplicationResult<ProjectCatalog> {
        if branch.is_empty()
            || branch.len() > 256
            || branch.starts_with('-')
            || branch.contains("@{")
            || base.len() > 512
            || branch.contains('\0')
            || base.contains('\0')
        {
            return Err(ApplicationError(ErrorKind::Invalid, "Invalid branch or base revision".into()));
        }
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            let base = if base.is_empty() {
                backend.environment()?.preferences.default_worktree_base
            } else {
                base
            };
            backend.create_worktree(project, &branch, &base)
        })
        .await
    }

    pub async fn remove(&self, id: i64) -> ApplicationResult<ProjectCatalog> { self.remove_checked(id, || Ok(())).await }

    pub async fn remove_checked(&self, id: i64, check: impl FnOnce() -> ApplicationResult<()> + Send + 'static) -> ApplicationResult<ProjectCatalog> {
        let lease = self.gate.clone().write_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            check()?;
            backend.remove_worktree(id)
        })
        .await
    }

    pub async fn changes(&self, id: i64) -> ApplicationResult<GitChanges> {
        let lease = self.gate.clone().read_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.changes(id)
        })
        .await
    }

    pub async fn diff(&self, id: i64, path: String) -> ApplicationResult<FileDiff> {
        validate_diff_path(&path)?;
        let lease = self.gate.clone().read_owned().await;
        let backend = self.backend.clone();
        blocking(move || {
            let _lease = lease;
            backend.diff(id, &path)
        })
        .await
    }

    pub fn operation_application(&self, workspace: Option<i64>) -> ApplicationResult<Application> {
        let workspace = match workspace.or(self.default_workspace) {
            | Some(id) => Some(id),
            | None => self.backend.catalog()?.default_workspace_id,
        };
        self.backend
            .active_application(workspace)
            .ok_or_else(|| ApplicationError(ErrorKind::NotFound, "Workspace has no active application".into()))
    }

    /// Synchronous catalog for repository operations already holding a
    /// workspace lease.
    pub fn operation_catalog(&self) -> ApplicationResult<ProjectCatalog> { self.backend.catalog() }

    pub fn cancel_all(&self) { self.backend.cancel_all(); }
}
fn workspace_not_found() -> ApplicationError { ApplicationError(ErrorKind::NotFound, "Workspace not found in this session".into()) }
fn require_idle(jobs: Option<&crate::Jobs>, workspace: i64) -> ApplicationResult<()> {
    if let Some(jobs) = jobs
        && jobs.workspace_busy(workspace)?
    {
        return Err(ApplicationError(
            ErrorKind::Busy,
            "Workspace has pending or running jobs; finish or cancel them first".into(),
        ));
    }
    Ok(())
}
fn validate_diff_path(path: &str) -> ApplicationResult<()> {
    if path.is_empty() || path.len() > 32768 || path.contains('\0') {
        return Err(ApplicationError(ErrorKind::Invalid, "Invalid diff path".into()));
    }
    Ok(())
}
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> ApplicationResult<T> + Send + 'static) -> ApplicationResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ApplicationError(ErrorKind::Failure, error.to_string()))?
}
