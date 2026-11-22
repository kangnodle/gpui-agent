mod attachments;
mod mcp;
mod pull_requests;
use crate::{Settings,
            new_token};
use anyhow::{Context,
             Result};
use gpui_agent_application::{Application,
                             ApplicationError,
                             ApplicationResult,
                             ErrorKind,
                             Jobs,
                             Workspaces,
                             ports::JobRepository};
use gpui_agent_contracts::{AccountProfile,
                           ChatRequest,
                           Job,
                           JobDetail,
                           JobEvent,
                           JobScope,
                           JobState,
                           Provider,
                           StreamEvent,
                           provider_name};
use rusqlite::{Connection,
               OptionalExtension,
               params};
use std::{collections::HashMap,
          path::Path,
          sync::{Arc,
                 Mutex}};

type Factory = dyn Fn(&JobScope) -> Result<Application> + Send + Sync;

struct QueueOwner(std::fs::File);
impl Drop for QueueOwner {
    fn drop(&mut self) {
        // Closing alone keeps the lock alive while a concurrent fork owns a
        // duplicate.
        let _ = self.0.unlock();
    }
}

pub struct JobStore {
    oauth: gpui_agent::McpOAuth,
    database: Mutex<Connection>,
    home: std::path::PathBuf,
    workspaces: Workspaces,
    applications: Mutex<HashMap<(String, i64), Application>>,
    factory: Box<Factory>,
    _owner: QueueOwner,
}
impl JobStore {
    pub fn open(settings: Settings, workspaces: Workspaces) -> Result<Self> {
        std::fs::create_dir_all(&settings.home)?;
        let owner = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(settings.home.join("queue.lock"))?;
        owner
            .try_lock()
            .context("Another queue server owns this home; connect to its API or use a separate --home")?;
        let owner = QueueOwner(owner);
        let path = settings.home.join("jobs.sqlite3");
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(&path)?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS accounts (provider TEXT NOT NULL,id TEXT NOT NULL,label TEXT NOT NULL,PRIMARY KEY(provider,id));
            CREATE TABLE IF NOT EXISTS jobs (id INTEGER PRIMARY KEY AUTOINCREMENT,provider TEXT NOT NULL,account TEXT NOT NULL,workspace INTEGER NOT NULL,
                scope TEXT NOT NULL,request TEXT NOT NULL,state TEXT NOT NULL,conversation INTEGER,operation TEXT,preview TEXT NOT NULL,error TEXT,
                created INTEGER NOT NULL DEFAULT(unixepoch()),cancel INTEGER NOT NULL DEFAULT 0,priority INTEGER NOT NULL DEFAULT 0,pending TEXT);
            CREATE INDEX IF NOT EXISTS job_queue ON jobs(provider,account,state,priority,id);
            CREATE INDEX IF NOT EXISTS job_conversation ON jobs(provider,account,workspace,conversation,id);
            CREATE TABLE IF NOT EXISTS job_events (sequence INTEGER PRIMARY KEY AUTOINCREMENT,job INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,event TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS job_event_order ON job_events(job,sequence);")?;
        pull_requests::initialize(&connection)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS command_receipts (
            command TEXT PRIMARY KEY,job INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,fingerprint BLOB NOT NULL);",
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS uploaded_attachments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,scope TEXT NOT NULL,fingerprint BLOB NOT NULL,
            payload TEXT NOT NULL,name TEXT NOT NULL,kind TEXT NOT NULL,bytes INTEGER NOT NULL,
            UNIQUE(scope,fingerprint));",
        )?;
        let has_pending = connection
            .prepare("PRAGMA table_info(jobs)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|name| name == "pending");
        if !has_pending {
            connection.execute("ALTER TABLE jobs ADD COLUMN pending TEXT", [])?;
        }
        for provider in gpui_agent_domain::PROVIDERS {
            connection.execute(
                "INSERT OR IGNORE INTO accounts(provider,id,label) VALUES(?1,'default','Default')",
                [provider_name(provider)],
            )?;
        }
        // A crashed turn may have had side effects. Never automatically replay
        // it.
        let tx = connection.transaction()?;
        let message = "Server stopped during execution; this job was interrupted and will not be replayed";
        tx.execute(
            "INSERT INTO job_events(job,event) SELECT id,?1 FROM jobs WHERE state IN ('running','waiting')",
            [serde_json::to_string(&StreamEvent::Error {
                message: message.into(),
            })?],
        )?;
        tx.execute(
            "UPDATE jobs SET state='interrupted',pending=NULL,error=?1 WHERE state IN ('running','waiting')",
            [message],
        )?;
        tx.commit()?;
        retire_workflow_tables(&connection)?;
        // Retain records from removed providers without dispatching or decoding
        // them.
        connection.execute_batch("CREATE TEMP TABLE supported_providers (name TEXT PRIMARY KEY);")?;
        for provider in gpui_agent_domain::PROVIDERS {
            connection.execute("INSERT INTO supported_providers VALUES(?1)", [provider_name(provider)])?;
        }
        connection.execute_batch(
            "CREATE TEMP VIEW supported_jobs AS
            SELECT * FROM jobs WHERE provider IN (SELECT name FROM supported_providers);",
        )?;
        let shared = workspaces.clone();
        Ok(Self {
            oauth: gpui_agent::McpOAuth::default(),
            database: Mutex::new(connection),
            home: settings.home.clone(),
            workspaces,
            applications: Mutex::new(HashMap::new()),
            _owner: owner,
            factory: Box::new(move |scope| {
                if scope.account_id == "default" {
                    return Ok(shared.operation_application(scope.workspace_id)?);
                }
                let scoped = Settings {
                    account_id: scope.account_id.clone(),
                    cwd: Path::new(&scope.workspace_path).to_path_buf(),
                    home: settings.home.join("accounts").join(&scope.account_id),
                    ..settings.clone()
                };
                scoped.registered_application()
            }),
        })
    }

    pub fn with_application_factory(mut self, factory: impl Fn(&JobScope) -> Result<Application> + Send + Sync + 'static) -> Self {
        self.factory = Box::new(factory);
        self
    }

    pub fn service(self, workspaces: Workspaces) -> Jobs {
        let verifier = crate::project_verifier::Verifier {
            home: self.home.clone(),
        };
        Jobs::new(Arc::new(self), workspaces)
            .with_clock(Arc::new(crate::SystemClock))
            .with_diagnostics(Arc::new(crate::StderrDiagnostics))
            .with_project_verifier(Arc::new(verifier))
    }

    fn db<T>(&self, work: impl FnOnce(&mut Connection) -> Result<T>) -> ApplicationResult<T> {
        let mut db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(ApplicationError(ErrorKind::Failure, format!("Job database lock poisoned: {error}"))),
        };
        work(&mut db).map_err(|error| match error.downcast::<ApplicationError>() {
            | Ok(error) => error,
            | Err(error) => ApplicationError::from(error),
        })
    }
}
/// Workflows, delegation and repair memories were removed. Their queued jobs
/// must not start as ordinary jobs, and their tables are dropped so that
/// references to jobs (some without cascading deletes) cannot block deleting
/// conversations.
fn retire_workflow_tables(connection: &Connection) -> Result<()> {
    let exists = |table: &str| -> rusqlite::Result<bool> {
        connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [table], |row| {
            row.get(0)
        })
    };
    let mut owned = Vec::new();
    if exists("workflow_jobs")? {
        owned.push("SELECT job FROM workflow_jobs");
    }
    if exists("workflow_plans")? {
        owned.push("SELECT job FROM workflow_plans");
    }
    let tx = connection.unchecked_transaction()?;
    if !owned.is_empty() {
        let owned = owned.join(" UNION ");
        tx.execute(
            &format!("INSERT INTO job_events(job,event) SELECT id,?1 FROM jobs WHERE state='queued' AND id IN ({owned})"),
            [serde_json::to_string(&StreamEvent::Cancelled)?],
        )?;
        tx.execute(
            &format!("UPDATE jobs SET state='cancelled',cancel=1 WHERE state='queued' AND id IN ({owned})"),
            [],
        )?;
    }
    // Children before parents: dropping a referenced table checks its
    // foreign keys.
    tx.execute_batch(
        "DROP TABLE IF EXISTS task_notification_stops;
        DROP TABLE IF EXISTS task_delivery_cursors;
        DROP TABLE IF EXISTS task_notifications;
        DROP TABLE IF EXISTS task_followups;
        DROP TABLE IF EXISTS task_messages;
        DROP TABLE IF EXISTS delegation_parents;
        DROP TABLE IF EXISTS delegations;
        DROP TABLE IF EXISTS goal_reports;
        DROP TABLE IF EXISTS goal_verifications;
        DROP TABLE IF EXISTS workflow_plans;
        DROP TABLE IF EXISTS repair_memories;
        DROP TABLE IF EXISTS workflow_jobs;
        DROP TABLE IF EXISTS workflows;",
    )?;
    tx.commit()?;
    Ok(())
}
fn reject(kind: ErrorKind, message: &str) -> anyhow::Error { ApplicationError(kind, message.into()).into() }
fn json_error(error: serde_json::Error) -> rusqlite::Error { rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error)) }
fn job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: row.get(0)?,
        scope: serde_json::from_str(&row.get::<_, String>(1)?).map_err(json_error)?,
        state: serde_json::from_value(serde_json::json!(row.get::<_, String>(2)?)).map_err(json_error)?,
        conversation_id: row.get(3)?,
        operation_id: row.get::<_, Option<String>>(4)?.and_then(|id| id.parse().ok()),
        pending_request_id: row.get::<_, Option<String>>(9)?.and_then(|id| id.parse().ok()),
        last_sequence: row.get(10)?,
        preview: row.get(5)?,
        error: row.get(6)?,
        created_at: row.get(7)?,
        cancel_requested: row.get(8)?,
    })
}
const COLUMNS: &str =
    "id,scope,state,conversation,operation,preview,error,created,cancel,pending,coalesce((SELECT max(sequence) FROM job_events WHERE job=jobs.id),0)";
fn load_job(connection: &Connection, id: i64) -> Result<Job> {
    connection
        .query_row(&format!("SELECT {COLUMNS} FROM supported_jobs AS jobs WHERE id=?1"), [id], job)
        .optional()?
        .ok_or_else(|| reject(ErrorKind::NotFound, "Job not found"))
}
fn load(connection: &Connection, id: i64) -> Result<JobDetail> {
    let job = load_job(connection, id)?;
    let request = serde_json::from_str(&connection.query_row("SELECT request FROM supported_jobs AS jobs WHERE id=?1", [id], |row| row.get::<_, String>(0))?)?;
    Ok(JobDetail {
        job,
        request,
    })
}
fn account(connection: &Connection, scope: &JobScope) -> Result<()> {
    gpui_agent_domain::validate_account_id(&scope.account_id).map_err(anyhow::Error::msg)?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE provider=?1 AND id=?2)",
        params![provider_name(scope.provider), scope.account_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(reject(ErrorKind::NotFound, "Account profile not found for this provider"));
    }
    Ok(())
}
impl JobRepository for JobStore {
    fn pull_request_guide(
        &self,
        scope: &JobScope,
        target: &gpui_agent_domain::PullRequestTarget,
    ) -> ApplicationResult<Option<gpui_agent_domain::PullRequestGuide>> {
        self.pr_guide(scope, target)
    }

    fn start_pull_request_guide(
        &self,
        scope: &JobScope,
        target: &gpui_agent_domain::PullRequestTarget,
        head: &str,
        base: &str,
        request: &ChatRequest,
    ) -> ApplicationResult<(gpui_agent_domain::PullRequestGuide, bool)> {
        self.pr_start_guide(scope, target, head, base, request)
    }

    fn due_pull_requests(&self, now: i64) -> ApplicationResult<Vec<gpui_agent_domain::PullRequestLink>> { self.pr_due(now) }

    fn apply_pull_request_poll(
        &self,
        previous: &gpui_agent_domain::PullRequestLink,
        next: &gpui_agent_domain::PullRequestLink,
        notification: Option<&ChatRequest>,
    ) -> ApplicationResult<Option<Job>> {
        self.pr_apply(previous, next, notification)
    }

    fn stop_pull_request_watches(&self, scope: &JobScope, conversation: i64) -> ApplicationResult<()> { self.pr_stop(scope, conversation) }

    fn set_pull_request_watch(
        &self,
        scope: &JobScope,
        conversation: i64,
        target: &gpui_agent_domain::PullRequestTarget,
        baseline: Option<&gpui_agent_domain::PullRequestData>,
        now: i64,
    ) -> ApplicationResult<bool> {
        self.pr_set_watch(scope, conversation, target, baseline, now)
    }

    fn pull_request_links(&self, scope: &JobScope, conversation: i64) -> ApplicationResult<Vec<gpui_agent_domain::PullRequestLink>> {
        self.pr_links(scope, conversation)
    }

    fn set_pull_request_link(
        &self,
        scope: &JobScope,
        conversation: i64,
        target: &gpui_agent_domain::PullRequestTarget,
        linked: bool,
    ) -> ApplicationResult<bool> {
        self.pr_set_link(scope, conversation, target, linked)
    }

    fn workspace_has_uploads(&self, id: i64) -> ApplicationResult<bool> {
        self.db(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM uploaded_attachments WHERE json_extract(scope,'$.workspace_id')=?1)",
                [id],
                |row| row.get(0),
            )?)
        })
    }

    fn upload_attachment(
        &self,
        scope: &JobScope,
        attachment: &gpui_agent_contracts::Attachment,
    ) -> ApplicationResult<gpui_agent_contracts::UploadedAttachment> {
        attachments::upload(self, scope, attachment)
    }

    fn uploaded_attachments(&self, scope: &JobScope) -> ApplicationResult<Vec<gpui_agent_contracts::UploadedAttachment>> { attachments::list(self, scope) }

    fn delete_attachment(&self, scope: &JobScope, id: i64) -> ApplicationResult<()> { attachments::delete(self, scope, id) }

    fn mcp(&self, scope: &JobScope, servers: Option<&[gpui_agent_contracts::McpServer]>) -> ApplicationResult<Vec<gpui_agent_contracts::McpServer>> {
        self.mcp_config(scope, servers)
    }

    fn mcp_oauth(
        &self,
        scope: &JobScope,
        name: &str,
        command: gpui_agent_contracts::McpOAuthCommand,
    ) -> ApplicationResult<gpui_agent_contracts::McpOAuthStatus> {
        use gpui_agent_contracts::McpOAuthCommand;
        let mut servers = self.mcp_config(scope, None)?;
        servers.extend(crate::PluginFiles::new(&self.home, Path::new(&scope.workspace_path))?.mcp_servers()?);
        gpui_agent_domain::validate_mcp(&servers).map_err(anyhow::Error::msg)?;
        let server = servers
            .into_iter()
            .find(|server| server.name() == name)
            .ok_or_else(|| ApplicationError(ErrorKind::NotFound, "MCP server not found".into()))?;
        if matches!(command, McpOAuthCommand::Login | McpOAuthCommand::Logout) {
            self.db(|db| {
                let pending: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM supported_jobs WHERE provider=?1 AND account=?2 AND state IN ('queued','running','waiting'))",
                    params![provider_name(scope.provider), scope.account_id],
                    |row| row.get(0),
                )?;
                if pending {
                    return Err(reject(
                        ErrorKind::Busy,
                        "Finish or cancel this profile's jobs before changing MCP authentication",
                    ));
                }
                Ok(())
            })?;
        }
        let home = if scope.account_id == "default" {
            self.home.clone()
        } else {
            self.home.join("accounts").join(&scope.account_id)
        }
        .join(provider_name(scope.provider));
        self.oauth.execute(&home, &server, command).map_err(Into::into)
    }

    fn cancel_mcp_oauth(&self) { self.oauth.cancel_all(); }

    fn accounts(&self, provider: Provider) -> ApplicationResult<Vec<AccountProfile>> {
        self.db(|db| {
            Ok(db
                .prepare("SELECT id,label FROM accounts WHERE provider=?1 ORDER BY id!='default',rowid")?
                .query_map([provider_name(provider)], |row| {
                    Ok(AccountProfile {
                        id: row.get(0)?,
                        provider,
                        label: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    fn create_account(&self, provider: Provider, label: &str) -> ApplicationResult<AccountProfile> {
        self.db(|db| {
            let count: i64 = db.query_row("SELECT count(*) FROM accounts WHERE provider=?1", [provider_name(provider)], |row| row.get(0))?;
            if count >= 16 {
                return Err(reject(ErrorKind::Invalid, "At most 16 account profiles per provider are supported"));
            }
            let id = new_token()?[.. 32].to_owned();
            db.execute(
                "INSERT INTO accounts(provider,id,label) VALUES(?1,?2,?3)",
                params![provider_name(provider), id, label],
            )?;
            Ok(AccountProfile {
                id,
                provider,
                label: label.into(),
            })
        })
    }

    fn application(&self, scope: &JobScope) -> ApplicationResult<Application> {
        self.db(|db| account(db, scope))?;
        let key = (
            format!("{}:{}", provider_name(scope.provider), scope.account_id),
            scope.workspace_id.unwrap_or_default(),
        );
        let mut applications = match self.applications.lock() {
            | Ok(applications) => applications,
            | Err(error) => return Err(ApplicationError(ErrorKind::Failure, format!("Job application cache lock poisoned: {error}"))),
        };
        if let Some(app) = applications.get(&key) {
            return Ok(app.clone());
        }
        let app = (self.factory)(scope)?;
        applications.insert(key, app.clone());
        Ok(app)
    }

    fn enqueue(&self, scope: &JobScope, request: &ChatRequest) -> ApplicationResult<Job> { self.db(|db| enqueue(db, scope, request)) }

    fn enqueue_once(&self, scope: &JobScope, request: &ChatRequest, command_id: &str) -> ApplicationResult<Job> {
        self.enqueue_uploaded_once(scope, request, &[], command_id)
    }

    fn enqueue_uploaded_once(&self, scope: &JobScope, request: &ChatRequest, ids: &[i64], command_id: &str) -> ApplicationResult<Job> {
        use sha2::{Digest,
                   Sha256};
        gpui_agent_domain::validate_command_id(command_id)?;
        request.validate_provider(scope.provider).map_err(|error| reject(ErrorKind::Invalid, error))?;
        request.validate_with_uploads(ids).map_err(|error| reject(ErrorKind::Invalid, error))?;
        self.db(|db| {
            let fingerprint = Sha256::digest(if ids.is_empty() {
                serde_json::to_vec(&(scope, request))?
            } else {
                serde_json::to_vec(&(scope, request, ids))?
            })
            .to_vec();
            let tx = db.transaction()?;
            let receipt = tx
                .query_row("SELECT job,fingerprint FROM command_receipts WHERE command=?1", [command_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
                })
                .optional()?;
            if let Some((id, stored)) = receipt {
                if stored != fingerprint {
                    return Err(reject(ErrorKind::Busy, "Command ID was already used with different input or scope"));
                }
                return load_job(&tx, id);
            }
            let mut request = request.clone();
            for id in ids {
                request.attachments.push(attachments::load(&tx, scope, *id)?);
            }
            request.validate().map_err(|error| reject(ErrorKind::Invalid, error))?;
            let job = enqueue(&tx, scope, &request)?;
            tx.execute(
                "INSERT INTO command_receipts(command,job,fingerprint) VALUES(?1,?2,?3)",
                params![command_id, job.id, fingerprint],
            )?;
            tx.commit()?;
            Ok(job)
        })
    }

    fn list(&self) -> ApplicationResult<Vec<Job>> {
        self.db(|db| {
            Ok(db
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM supported_jobs AS jobs ORDER BY state IN ('queued','running','waiting') DESC,id DESC LIMIT 1000"
                ))?
                .query_map([], job)?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    fn job(&self, id: i64) -> ApplicationResult<Job> { self.db(|db| load_job(db, id)) }

    fn conversation_job(&self, scope: &JobScope, conversation: i64, job_id: Option<i64>, active_only: bool) -> ApplicationResult<Option<Job>> {
        self.db(|db| {
            Ok(db
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM supported_jobs AS jobs
                WHERE provider=?1 AND account=?2 AND workspace=?3 AND conversation=?4 AND (?5 IS NULL OR id=?5)
                AND (?6=0 OR state IN ('queued','running','waiting'))
                ORDER BY CASE WHEN ?6 THEN state IN ('running','waiting') ELSE 0 END DESC,id DESC LIMIT 1"
                    ),
                    params![
                        provider_name(scope.provider),
                        scope.account_id,
                        scope.workspace_id,
                        conversation,
                        job_id,
                        active_only
                    ],
                    job,
                )
                .optional()?)
        })
    }

    fn get(&self, id: i64) -> ApplicationResult<JobDetail> { self.db(|db| load(db, id)) }

    fn claim(&self, scope: &JobScope) -> ApplicationResult<Option<JobDetail>> {
        self.db(|db| {
            let tx = db.transaction()?;
            let running: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM supported_jobs AS jobs WHERE provider=?1 AND account=?2 AND state IN ('running','waiting'))",
                params![provider_name(scope.provider), scope.account_id],
                |row| row.get(0),
            )?;
            if running {
                return Ok(None);
            }
            let id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM supported_jobs AS jobs WHERE provider=?1 AND account=?2 AND state='queued' ORDER BY priority,id LIMIT 1",
                    params![provider_name(scope.provider), scope.account_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(id) = id else {
                return Ok(None);
            };
            tx.execute("UPDATE jobs SET state='running' WHERE id=?1 AND state='queued'", [id])?;
            let detail = load(&tx, id)?;
            tx.commit()?;
            Ok(Some(detail))
        })
    }

    fn append(&self, id: i64, event: &StreamEvent) -> ApplicationResult<()> { self.append_batch(id, std::slice::from_ref(event)) }

    fn append_batch(&self, id: i64, events: &[StreamEvent]) -> ApplicationResult<()> {
        self.db(|db| {
            let tx = db.transaction()?;
            let mut current = load_job(&tx, id)?;
            let mut insert = tx.prepare("INSERT INTO job_events(job,event) VALUES(?1,?2)")?;
            for event in events {
                if !gpui_agent_application::apply_job_event(&mut current, event) {
                    break;
                }
                insert.execute(params![id, serde_json::to_string(event)?])?;
            }
            drop(insert);
            let state = serde_json::to_value(current.state)?;
            tx.execute(
                "UPDATE jobs SET state=?1,error=?2,conversation=?3,operation=?4,pending=?5 WHERE id=?6",
                params![
                    state.as_str(),
                    current.error,
                    current.conversation_id,
                    current.operation_id.map(|id| id.to_string()),
                    current.pending_request_id.map(|id| id.to_string()),
                    id
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    fn events(&self, id: i64, after: i64) -> ApplicationResult<Vec<JobEvent>> {
        self.db(|db| {
            load_job(db, id)?;
            Ok(db
                .prepare("SELECT sequence,event FROM job_events WHERE job=?1 AND sequence>?2 ORDER BY sequence LIMIT 64")?
                .query_map(params![id, after], |row| {
                    Ok(JobEvent {
                        sequence: row.get(0)?,
                        event: serde_json::from_str(&row.get::<_, String>(1)?).map_err(json_error)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    fn cancel_queued(&self, id: i64) -> ApplicationResult<bool> {
        self.db(|db| {
            let tx = db.transaction()?;
            let detail = load(&tx, id)?;
            if detail.job.state.terminal() {
                return Err(reject(ErrorKind::Busy, "Job has already ended"));
            }
            if let Some(conversation) = detail.job.conversation_id {
                pull_requests::stop(&tx, &detail.job.scope, conversation)?;
            }
            if detail.job.state != JobState::Queued {
                tx.execute("UPDATE jobs SET cancel=1 WHERE id=?1", [id])?;
                tx.commit()?;
                return Ok(false);
            }
            tx.execute("UPDATE jobs SET state='cancelled',cancel=1 WHERE id=?1", [id])?;
            tx.execute(
                "INSERT INTO job_events(job,event) VALUES(?1,?2)",
                params![id, serde_json::to_string(&StreamEvent::Cancelled)?],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    fn purge_conversation(&self, provider: Provider, account: &str, workspace: i64, conversation: i64) -> ApplicationResult<()> {
        self.db(|db| {
            let tx = db.transaction()?;
            let db = &tx;
            let active:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM supported_jobs AS jobs WHERE provider=?1 AND account=?2 AND workspace=?3 AND conversation=?4 AND state IN ('queued','running','waiting'))",params![provider_name(provider),account,workspace,conversation],|row|row.get(0))?;
            if active {return Err(reject(ErrorKind::Busy,"Conversation has pending jobs; cancel them before deleting"));}
            db.execute("DELETE FROM jobs WHERE provider=?1 AND account=?2 AND workspace=?3 AND conversation=?4",params![provider_name(provider),account,workspace,conversation])?;
            db.execute("DELETE FROM pull_request_links WHERE json_extract(scope,'$.provider')=?1 AND json_extract(scope,'$.account_id')=?2 AND json_extract(scope,'$.workspace_id')=?3 AND conversation=?4",params![provider_name(provider),account,workspace,conversation])?;
            tx.commit()?; Ok(())
        })
    }

    fn workspace_busy(&self, id: i64) -> ApplicationResult<bool> {
        let busy = match self.applications.lock() {
            | Ok(applications) => applications.iter().any(|((_, workspace), app)| *workspace == id && app.is_busy()),
            | Err(error) => return Err(ApplicationError(ErrorKind::Failure, format!("Job application cache lock poisoned: {error}"))),
        };
        if busy {
            return Ok(true);
        }
        self.db(|db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM supported_jobs AS jobs WHERE workspace=?1 AND state IN ('queued','running','waiting'))",
                [id],
                |row| row.get(0),
            )?)
        })
    }

    fn cancel_all(&self) {
        for app in (match self.applications.lock() {
            | Ok(guard) => guard,
            | Err(error) => {
                eprintln!("Job application cache lock poisoned; recovering registered entries for cleanup: {error}");
                error.into_inner()
            },
        })
        .values()
        {
            app.cancel_all();
        }
    }
}

fn enqueue(db: &Connection, scope: &JobScope, request: &ChatRequest) -> Result<Job> {
    account(db, scope)?;
    let pending: i64 = db.query_row(
        "SELECT count(*) FROM supported_jobs AS jobs WHERE state IN ('queued','running','waiting')",
        [],
        |row| row.get(0),
    )?;
    let account_pending: i64 = db.query_row(
        "SELECT count(*) FROM supported_jobs AS jobs WHERE provider=?1 AND account=?2 AND state IN ('queued','running','waiting')",
        params![provider_name(scope.provider), scope.account_id],
        |row| row.get(0),
    )?;
    gpui_agent_domain::validate_queue_capacity(pending, account_pending).map_err(ApplicationError::from)?;
    db.execute(
        "INSERT INTO jobs(provider,account,workspace,scope,request,state,conversation,preview) VALUES(?1,?2,?3,?4,?5,'queued',?6,?7)",
        params![
            provider_name(scope.provider),
            scope.account_id,
            scope.workspace_id,
            serde_json::to_string(scope)?,
            serde_json::to_string(request)?,
            request.conversation_id,
            request.preview()
        ],
    )?;
    Ok(load(db, db.last_insert_rowid())?.job)
}

#[cfg(all(test, unix))]
mod ownership_tests {
    use super::*;

    #[test]
    fn dropping_owner_releases_the_queue_even_when_a_descriptor_was_duplicated() -> Result<()> {
        let root = tempfile::tempdir()?;
        let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
        let workspaces = settings.workspaces(settings.application()?)?;
        let store = JobStore::open(settings.clone(), workspaces.clone())?;
        assert!(JobStore::open(settings.clone(), workspaces.clone()).is_err());
        // A concurrent fork can keep the same open file description until exec.
        let inherited = store._owner.0.try_clone()?;
        drop(store);
        let replacement = JobStore::open(settings.clone(), workspaces.clone())?;
        drop(inherited);
        assert!(JobStore::open(settings, workspaces).is_err());
        drop(replacement);
        Ok(())
    }
}

#[cfg(test)]
mod poison_tests {
    use super::*;
    #[test]
    fn poisoned_job_database_rejects_work_without_running_the_transaction() -> Result<()> {
        let root = tempfile::tempdir()?;
        let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
        let workspaces = settings.workspaces(settings.application()?)?;
        let store = JobStore::open(settings, workspaces)?;
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = store.database.lock().unwrap();
                panic!("poison job database");
            }))
            .is_err()
        );
        let mut called = false;
        let error = store
            .db(|_| {
                called = true;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.0, ErrorKind::Failure);
        assert!(error.1.contains("Job database lock poisoned"));
        assert!(!called);
        assert!(store.list().is_err());
        Ok(())
    }
}
