#[path = "checkpoints.rs"]
mod checkpoints;
#[path = "environment.rs"]
mod environment;
#[path = "pull_request_reads.rs"]
mod pull_request_reads;
#[path = "pull_requests.rs"]
mod pull_requests;
use crate::{Settings,
            git,
            new_token};
use gpui_agent_application::{Application,
                             ApplicationError,
                             ApplicationResult,
                             ErrorKind,
                             Workspaces,
                             ports::WorkspaceBackend};
use gpui_agent_domain::{FileDiff,
                        GitChanges,
                        Project,
                        ProjectCatalog,
                        Workspace};
use rusqlite::{Connection,
               OptionalExtension,
               params};
use std::{collections::HashMap,
          path::{Path,
                 PathBuf},
          sync::Mutex};
pub(crate) fn invalid(message: impl Into<String>) -> ApplicationError { ApplicationError(ErrorKind::Invalid, message.into()) }
pub(crate) fn failure(message: impl Into<String>) -> ApplicationError { ApplicationError(ErrorKind::Failure, message.into()) }
fn conflict(message: impl Into<String>) -> ApplicationError { ApplicationError(ErrorKind::Busy, message.into()) }
fn missing() -> ApplicationError { ApplicationError(ErrorKind::NotFound, "Project or workspace not found".into()) }
trait DbResult<T> {
    fn app(self) -> ApplicationResult<T>;
}
impl<T> DbResult<T> for rusqlite::Result<T> {
    fn app(self) -> ApplicationResult<T> { self.map_err(|error| failure(error.to_string())) }
}
struct State {
    db: Connection,
}
type ApplicationFactory = dyn Fn(&Path) -> anyhow::Result<Application> + Send + Sync;
pub struct Projects {
    settings: Settings,
    state: Mutex<State>,
    applications: Mutex<HashMap<i64, Application>>,
    default: Option<i64>,
    initial_application: Application,
    hooks: PathBuf,
    factory: Box<ApplicationFactory>,
}
impl Projects {
    pub fn open(settings: Settings, default_application: Application) -> ApplicationResult<Self> {
        let mut store = Self::open_catalog(settings, default_application.clone())?;
        let cwd = canonical(&std::path::absolute(&store.settings.cwd).map_err(|error| invalid(error.to_string()))?)?;
        let state = match store.state.get_mut() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let project = register_path(&state.db, &cwd, &store.hooks)?;
        let id = workspace_row(&state.db, project, &cwd, true, false)?;
        store.default = Some(id);
        (match store.applications.get_mut() {
            | Ok(applications) => applications,
            | Err(error) => return Err(failure(format!("Workspace application cache lock poisoned: {error}"))),
        })
        .insert(id, default_application);
        Ok(store)
    }

    /// Restore saved projects without registering the process working
    /// directory.
    pub fn open_catalog(settings: Settings, initial_application: Application) -> ApplicationResult<Self> {
        std::fs::create_dir_all(&settings.home).map_err(|error| failure(error.to_string()))?;
        let database = settings.home.join("projects.sqlite3");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(&database)
                .map_err(|error| failure(error.to_string()))?;
        }
        let db = Connection::open(database).app()?;
        db.busy_timeout(std::time::Duration::from_secs(5)).app()?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS projects(id INTEGER PRIMARY KEY AUTOINCREMENT,path TEXT NOT NULL UNIQUE,name TEXT NOT NULL,common_dir TEXT);
            CREATE TABLE IF NOT EXISTS workspaces(id INTEGER PRIMARY KEY AUTOINCREMENT,project_id INTEGER NOT NULL REFERENCES projects(id),path TEXT NOT NULL UNIQUE,main INTEGER NOT NULL,managed INTEGER NOT NULL DEFAULT 0,removed INTEGER NOT NULL DEFAULT 0);").app()?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS checkpoints(id TEXT PRIMARY KEY,workspace INTEGER NOT NULL REFERENCES workspaces(id),label TEXT NOT NULL,created INTEGER NOT NULL DEFAULT(unixepoch()),head TEXT,snapshot TEXT NOT NULL,index_tree TEXT NOT NULL);").app()?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS commit_proposals(workspace INTEGER PRIMARY KEY REFERENCES workspaces(id),proposal TEXT NOT NULL);")
            .app()?;
        environment::initialize(&db)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS project_verification(project INTEGER PRIMARY KEY REFERENCES projects(id),plan TEXT NOT NULL);")
            .app()?;
        let has_hidden: bool = db
            .query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('projects') WHERE name='hidden')", [], |row| {
                row.get(0)
            })
            .app()?;
        if !has_hidden {
            db.execute_batch("ALTER TABLE projects ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;").app()?;
        }
        let hooks = settings.home.join("empty-hooks");
        let factory_settings = settings.clone();
        let factory = Box::new(move |path: &Path| {
            Settings {
                cwd: path.into(),
                ..factory_settings.clone()
            }
            .registered_application()
        });
        Ok(Self {
            settings,
            factory,
            state: Mutex::new(State {
                db,
            }),
            applications: Mutex::new(HashMap::new()),
            default: None,
            initial_application,
            hooks,
        })
    }

    pub fn with_application_factory(mut self, factory: impl Fn(&Path) -> anyhow::Result<Application> + Send + Sync + 'static) -> Self {
        self.factory = Box::new(factory);
        self
    }

    pub fn service(self) -> Workspaces { Workspaces::new(self.default, std::sync::Arc::new(self)) }

    fn profile_locks(&self) -> ApplicationResult<Vec<std::fs::File>> {
        let mut profile_locks = Vec::new();
        let mut roots = vec![self.settings.home.clone()];
        let accounts = self.settings.home.join("accounts");
        if accounts.is_dir() {
            for entry in std::fs::read_dir(accounts).map_err(|error| failure(error.to_string()))? {
                let entry = entry.map_err(|error| failure(error.to_string()))?;
                if entry.file_type().map_err(|error| failure(error.to_string()))?.is_dir() {
                    roots.push(entry.path());
                }
            }
        }
        for root in roots {
            for provider in gpui_agent_domain::PROVIDERS {
                let profile = root.join(gpui_agent_domain::provider_name(provider));
                if profile.is_dir() {
                    let lock = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(profile.join(".operation.lock"))
                        .map_err(|error| failure(error.to_string()))?;
                    lock.try_lock()
                        .map_err(|_| conflict("A provider operation is active; stop it before a Git workspace operation"))?;
                    profile_locks.push(lock);
                }
            }
        }
        Ok(profile_locks)
    }

    fn project(&self, db: &Connection, id: i64) -> ApplicationResult<String> {
        db.query_row("SELECT path FROM projects WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .app()?
            .ok_or_else(missing)
    }

    fn workspace(&self, db: &Connection, id: i64) -> ApplicationResult<Workspace> {
        db.query_row(
            "SELECT id,project_id,path,main,managed FROM workspaces WHERE id=?1 AND removed=0",
            [id],
            |row| {
                Ok(Workspace {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    path: row.get(2)?,
                    main: row.get(3)?,
                    managed: row.get(4)?,
                    branch: None,
                    locked: false,
                    missing: false,
                })
            },
        )
        .optional()
        .app()?
        .ok_or_else(missing)
    }

    fn checked_workspace(&self, db: &Connection, id: i64) -> ApplicationResult<Workspace> {
        let workspace = self.workspace(db, id)?;
        if !Path::new(&workspace.path).is_dir() {
            return Err(missing());
        }
        if canonical(Path::new(&workspace.path))? != workspace.path {
            return Err(invalid(
                "Workspace path now resolves to a different directory; register that directory explicitly",
            ));
        }
        Ok(workspace)
    }

    fn catalog_locked(&self, state: &State) -> ApplicationResult<ProjectCatalog> {
        let mut projects = Vec::new();
        let mut query = state.db.prepare("SELECT id,path,name FROM projects WHERE hidden=0 ORDER BY id").app()?;
        let rows = query
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .app()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .app()?;
        for (id, path, name) in rows {
            let is_git = git::repository(Path::new(&path), &self.hooks);
            let trees = if is_git { git::trees(Path::new(&path), &self.hooks)? } else { Vec::new() };
            for tree in &trees {
                let path = canonical_or_absolute(Path::new(&tree.path))?;
                workspace_row(&state.db, id, &path, tree.main, false)?;
            }
            let mut query = state
                .db
                .prepare("SELECT id FROM workspaces WHERE project_id=?1 AND removed=0 ORDER BY main DESC,id")
                .app()?;
            let ids = query
                .query_map([id], |row| row.get::<_, i64>(0))
                .app()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .app()?;
            let mut workspaces = Vec::new();
            for workspace_id in ids {
                let mut workspace = self.workspace(&state.db, workspace_id)?;
                workspace.missing = !Path::new(&workspace.path).is_dir();
                if let Some(tree) = trees
                    .iter()
                    .find(|tree| canonical_or_absolute(Path::new(&tree.path)).ok().as_deref() == Some(&workspace.path))
                {
                    workspace.branch = tree.branch.clone();
                    workspace.locked = tree.locked;
                }
                workspaces.push(workspace);
            }
            projects.push(Project {
                verification: state
                    .db
                    .query_row("SELECT plan FROM project_verification WHERE project=?1", [id], |row| row.get::<_, String>(0))
                    .optional()
                    .app()?
                    .map(|raw| serde_json::from_str(&raw).map_err(|error| failure(error.to_string())))
                    .transpose()?
                    .unwrap_or_default(),
                id,
                name,
                path,
                git: is_git,
                workspaces,
            });
        }
        let available = || projects.iter().flat_map(|project| &project.workspaces).filter(|workspace| !workspace.missing);
        let default_workspace_id = available()
            .find(|workspace| Some(workspace.id) == self.default)
            .or_else(|| available().next())
            .map(|workspace| workspace.id);
        Ok(ProjectCatalog {
            default_workspace_id,
            registered_workspace_id: None,
            projects,
        })
    }
}
pub(crate) fn canonical(path: &Path) -> ApplicationResult<String> {
    if !path.is_absolute() {
        return Err(invalid("Project paths must be absolute"));
    }
    let path = std::fs::canonicalize(path).map_err(|error| invalid(format!("Cannot open project directory: {error}")))?;
    if !path.is_dir() {
        return Err(invalid("Project path must be a directory"));
    }
    path_string(&path)
}
fn path_string(path: &Path) -> ApplicationResult<String> {
    let value = path.to_str().ok_or_else(|| invalid("Paths must be valid UTF-8"))?;
    #[cfg(windows)]
    {
        if let Some(value) = value.strip_prefix(r"\\?\UNC\") {
            return Ok(format!(r"\\{value}"));
        }
        if let Some(value) = value.strip_prefix(r"\\?\") {
            return Ok(value.to_owned());
        }
    }
    Ok(value.to_owned())
}
fn canonical_or_absolute(path: &Path) -> ApplicationResult<String> {
    if path.exists() {
        canonical(path)
    } else {
        path_string(&std::path::absolute(path).map_err(|error| invalid(error.to_string()))?)
    }
}
fn register_path(db: &Connection, path: &str, hooks: &Path) -> ApplicationResult<i64> {
    let common = if git::repository(Path::new(path), hooks) {
        Some(canonical(Path::new(
            git::run(Path::new(path), hooks, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?
                .require()?
                .text()
                .trim(),
        ))?)
    } else {
        None
    };
    let existing: Option<i64> = db
        .query_row(
            "SELECT id FROM projects WHERE path=?1 OR (common_dir IS NOT NULL AND common_dir=?2) ORDER BY id LIMIT 1",
            params![path, common],
            |row| row.get(0),
        )
        .optional()
        .app()?;
    if let Some(id) = existing {
        db.execute("UPDATE projects SET common_dir=?1 WHERE id=?2", params![common, id]).app()?;
        return Ok(id);
    }
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.into());
    db.execute("INSERT INTO projects(path,name,common_dir) VALUES (?1,?2,?3)", params![path, name, common])
        .app()?;
    Ok(db.last_insert_rowid())
}
fn workspace_row(db: &Connection, project: i64, path: &str, main: bool, managed: bool) -> ApplicationResult<i64> {
    db.execute(
        "INSERT INTO workspaces(project_id,path,main,managed) VALUES (?1,?2,?3,?4) ON CONFLICT(path) DO NOTHING",
        params![project, path, main, managed],
    )
    .app()?;
    db.query_row("SELECT id FROM workspaces WHERE path=?1", [path], |row| row.get(0)).app()
}
impl WorkspaceBackend for Projects {
    fn set_project_verification(&self, project: i64, plan: &gpui_agent_domain::ProjectVerification) -> ApplicationResult<ProjectCatalog> {
        plan.validate()?;
        let state = self.state.lock().map_err(|_| failure("Project database unavailable"))?;
        self.project(&state.db, project)?;
        state
            .db
            .execute(
                "INSERT INTO project_verification(project,plan) VALUES(?1,?2) ON CONFLICT(project) DO UPDATE SET plan=excluded.plan",
                params![project, serde_json::to_string(plan).map_err(|error| failure(error.to_string()))?],
            )
            .app()?;
        self.catalog_locked(&state)
    }

    fn pull_request(
        &self,
        workspace: i64,
        target: &gpui_agent_domain::PullRequestTarget,
        kind: gpui_agent_domain::PullRequestReadKind,
    ) -> ApplicationResult<gpui_agent_domain::PullRequestData> {
        self.read_pr(workspace, target, kind)
    }

    fn update_environment(&self, patch: gpui_agent_domain::EnvironmentPreferencesPatch) -> ApplicationResult<gpui_agent_domain::EnvironmentPreferences> {
        self.patch_environment(patch)
    }

    fn environment(&self) -> ApplicationResult<gpui_agent_domain::EnvironmentDescriptor> {
        let state = self.state.lock().map_err(|_| failure("Project database unavailable"))?;
        environment::read(&state.db)
    }

    fn runtime(&self, provider: gpui_agent_domain::Provider, command: &gpui_agent_domain::RuntimeCommand) -> ApplicationResult<gpui_agent_domain::RuntimeInfo> {
        let _locks = if matches!(command, gpui_agent_domain::RuntimeCommand::Install { .. }) {
            Some(self.profile_locks()?)
        } else {
            None
        };
        crate::runtimes::run(&self.settings, provider, command).map_err(ApplicationError::from)
    }

    fn git(&self, workspace: i64, command: &gpui_agent_domain::GitCommand) -> ApplicationResult<gpui_agent_domain::GitOverview> {
        checkpoints::execute(self, workspace, command)
    }

    fn catalog(&self) -> ApplicationResult<ProjectCatalog> {
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        self.catalog_locked(&state)
    }

    fn register(&self, path: &str) -> ApplicationResult<ProjectCatalog> {
        let path = canonical(Path::new(path))?;
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let project = register_path(&state.db, &path, &self.hooks)?;
        state.db.execute("UPDATE projects SET hidden=0 WHERE id=?1", [project]).app()?;
        let trees = if git::repository(Path::new(&path), &self.hooks) {
            git::trees(Path::new(&path), &self.hooks)?
        } else {
            Vec::new()
        };
        let main = trees
            .iter()
            .find(|tree| canonical_or_absolute(Path::new(&tree.path)).ok().as_deref() == Some(&path))
            .is_none_or(|tree| tree.main);
        let id = workspace_row(&state.db, project, &path, main, false)?;
        let mut catalog = self.catalog_locked(&state)?;
        catalog.registered_workspace_id = Some(id);
        Ok(catalog)
    }

    fn forget_project(&self, id: i64) -> ApplicationResult<ProjectCatalog> {
        let state = self.state.lock().map_err(|error| failure(format!("Project database lock poisoned: {error}")))?;
        self.project(&state.db, id)?;
        let mut query = state.db.prepare("SELECT id FROM workspaces WHERE project_id=?1").app()?;
        let ids = query
            .query_map([id], |row| row.get::<_, i64>(0))
            .app()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .app()?;
        if ids
            .iter()
            .any(|id| self.active_application(Some(*id)).is_some_and(|application| application.is_busy()))
        {
            return Err(conflict("Finish or cancel active project operations before removing it from the list"));
        }
        state.db.execute("UPDATE projects SET hidden=1 WHERE id=?1", [id]).app()?;
        self.catalog_locked(&state)
    }

    fn application(&self, workspace: Option<i64>) -> ApplicationResult<Application> {
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let id = match workspace {
            | Some(id) => id,
            | None => match self.catalog_locked(&state)?.default_workspace_id {
                | Some(id) => id,
                | None => return Ok(self.initial_application.clone()),
            },
        };
        let workspace = self.checked_workspace(&state.db, id)?;
        if let Some(application) = self.active_application(Some(id)) {
            return Ok(application);
        }
        let application = (self.factory)(Path::new(&workspace.path)).map_err(|error| failure(error.to_string()))?;
        match self.applications.lock() {
            | Ok(mut applications) => {
                applications.insert(id, application.clone());
            },
            | Err(error) => return Err(failure(format!("Workspace application cache lock poisoned: {error}"))),
        }
        Ok(application)
    }

    fn create_worktree(&self, project: i64, branch: &str, base: &str) -> ApplicationResult<ProjectCatalog> {
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let root = self.project(&state.db, project)?;
        let root = Path::new(&root);
        if !git::repository(root, &self.hooks) {
            return Err(invalid("Select a Git working directory to create a worktree"));
        }
        if git::run(root, &self.hooks, &["check-ref-format", "--branch", branch])?.code != 0 {
            return Err(invalid("Invalid Git branch name"));
        }
        let base = git::run(root, &self.hooks, &["rev-parse", "--verify", "--end-of-options", &format!("{base}^{{commit}}")])?;
        if base.code != 0 {
            return Err(invalid("Base revision must identify an existing commit"));
        }
        let base = base.text().trim().to_owned();
        let home = std::fs::canonicalize(&self.settings.home).map_err(|error| failure(error.to_string()))?;
        let parent = home.join("worktrees").join(project.to_string());
        std::fs::create_dir_all(&parent).map_err(|error| failure(error.to_string()))?;
        let parent = std::fs::canonicalize(parent).map_err(|error| failure(error.to_string()))?;
        if !parent.starts_with(home.join("worktrees")) {
            return Err(invalid("Worktree storage must remain inside the application home"));
        }
        let path = path_string(&parent.join(&new_token().map_err(|error| failure(error.to_string()))?[.. 16]))?;
        let out = git::run(root, &self.hooks, &["worktree", "add", "--no-track", "-b", branch, "--", &path, &base])?;
        if out.code != 0 {
            return Err(conflict(format!("Cannot create worktree: {}", out.error.trim())));
        }
        workspace_row(&state.db, project, &canonical(Path::new(&path))?, false, true)?;
        self.catalog_locked(&state)
    }

    fn remove_worktree(&self, id: i64) -> ApplicationResult<ProjectCatalog> {
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let workspace = self.workspace(&state.db, id)?;
        if workspace.main || !workspace.managed {
            return Err(invalid("Only worktrees created by this app can be removed"));
        }
        if self.active_application(Some(id)).is_some_and(|application| application.is_busy()) {
            return Err(conflict("Stop the active workspace operation before removing its worktree"));
        }
        let home = std::fs::canonicalize(&self.settings.home).map_err(|error| failure(error.to_string()))?;
        let allowed = home.join("worktrees").join(workspace.project_id.to_string());
        let target = std::fs::canonicalize(&workspace.path).map_err(|error| invalid(error.to_string()))?;
        if !target.starts_with(&allowed) || target == allowed || path_string(&target)? != workspace.path {
            return Err(invalid("Refusing to remove a worktree outside its managed directory"));
        }
        let _profile_locks = self.profile_locks()?;
        let root = self.project(&state.db, workspace.project_id)?;
        let trees = git::trees(Path::new(&root), &self.hooks)?;
        let tree = trees
            .iter()
            .find(|tree| canonical_or_absolute(Path::new(&tree.path)).ok().as_deref() == Some(&workspace.path))
            .ok_or_else(missing)?;
        if tree.main || tree.locked {
            return Err(conflict("The main or a locked worktree cannot be removed"));
        }
        if !git::changes(Path::new(&workspace.path), &self.hooks, true)?.is_empty() {
            return Err(conflict(
                "Worktree contains changed, untracked or ignored files; keep or clean them before removal",
            ));
        }
        let out = git::run(Path::new(&root), &self.hooks, &["worktree", "remove", "--", &workspace.path])?;
        if out.code != 0 {
            return Err(conflict(format!("Cannot remove worktree: {}", out.error.trim())));
        }
        state.db.execute("UPDATE workspaces SET removed=1 WHERE id=?1", [id]).app()?;
        (match self.applications.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                eprintln!("Workspace application cache lock poisoned; recovering registered entries for cleanup: {error}");
                error.into_inner()
            },
        })
        .remove(&id);
        self.catalog_locked(&state)
    }

    fn changes(&self, id: i64) -> ApplicationResult<GitChanges> {
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let workspace = self.checked_workspace(&state.db, id)?;
        let path = Path::new(&workspace.path);
        if !git::repository(path, &self.hooks) {
            return Err(invalid("This workspace is not a Git working directory"));
        }
        Ok(GitChanges {
            workspace_id: id,
            branch: git::branch(path, &self.hooks),
            files: git::changes(path, &self.hooks, false)?,
        })
    }

    fn diff(&self, id: i64, path: &str) -> ApplicationResult<FileDiff> {
        let state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => return Err(failure(format!("Project database lock poisoned: {error}"))),
        };
        let workspace = self.checked_workspace(&state.db, id)?;
        let cwd = Path::new(&workspace.path);
        let files = git::changes(cwd, &self.hooks, false)?;
        let file = files.iter().find(|file| file.path == path).ok_or_else(missing)?;
        let prefix = git::run(cwd, &self.hooks, &["rev-parse", "--show-prefix"])?.require()?.text();
        let prefix = prefix.trim_end();
        let local_path = path.strip_prefix(prefix).ok_or_else(missing)?;
        let untracked = file.index_status == "?";
        let mut staged = String::new();
        let mut truncated = false;
        let unstaged = if untracked {
            let absolute = cwd.join(local_path);
            let absolute = path_string(&absolute)?;
            let out = git::run(
                cwd,
                &self.hooks,
                &[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--",
                    "/dev/null",
                    &absolute,
                ],
            )?;
            if out.code != 0 && out.code != 1 {
                return Err(failure(out.error));
            }
            truncated = out.truncated;
            out.text()
        } else {
            let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--cached", "--", local_path];
            if let Some(original) = &file.original_path {
                args.push(original.strip_prefix(prefix).ok_or_else(missing)?);
            }
            let out = git::run(cwd, &self.hooks, &args)?.require()?;
            staged = out.text();
            truncated |= out.truncated;
            args.remove(4);
            let out = git::run(cwd, &self.hooks, &args)?.require()?;
            truncated |= out.truncated;
            out.text()
        };
        Ok(FileDiff {
            workspace_id: id,
            path: path.into(),
            staged,
            unstaged,
            untracked,
            truncated,
        })
    }

    fn active_application(&self, workspace: Option<i64>) -> Option<Application> {
        let Some(id) = workspace else {
            return Some(self.initial_application.clone());
        };
        // Keep registered applications visible to busy checks and cancellation.
        (match self.applications.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                eprintln!("Workspace application cache lock poisoned; recovering registered entries for cleanup: {error}");
                error.into_inner()
            },
        })
        .get(&id)
        .cloned()
    }

    fn cancel_all(&self) {
        self.initial_application.cancel_all();
        for application in (match self.applications.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                eprintln!("Workspace application cache lock poisoned; recovering registered entries for cleanup: {error}");
                error.into_inner()
            },
        })
        .values()
        {
            application.cancel_all();
        }
    }
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
