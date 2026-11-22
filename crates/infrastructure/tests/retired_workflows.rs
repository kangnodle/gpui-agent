use anyhow::Result;
use gpui_agent_application::ports::JobRepository;
use gpui_agent_contracts::{ChatRequest,
                           JobScope,
                           JobState,
                           Provider};
use gpui_agent_infrastructure::{JobStore,
                                Settings};

/// Tables that only the removed workflow, delegation and repair-memory
/// features created.
const RETIRED: [&str; 13] = [
    "workflows",
    "workflow_jobs",
    "goal_reports",
    "goal_verifications",
    "delegations",
    "delegation_parents",
    "task_messages",
    "task_followups",
    "task_notifications",
    "task_delivery_cursors",
    "task_notification_stops",
    "workflow_plans",
    "repair_memories",
];

#[tokio::test]
async fn given_a_store_with_workflow_records_when_reopened_then_owned_queued_jobs_never_start_and_conversations_can_be_deleted() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let workspace = workspaces.default_workspace().unwrap();
    let scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspace),
        workspace_path: root.path().display().to_string(),
    };
    let request = ChatRequest {
        extensions: None,
        skills: vec![],
        computer_use: false,
        text: "queued work".into(),
        model: None,
        session_id: None,
        conversation_id: None,
        attachments: vec![],
        claude_permission_mode: None,
        codex_execution_policy: None,
    };
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    let workflow_child = store.enqueue(&scope, &request)?;
    let ordinary = store.enqueue(&scope, &request)?;
    let planning = store.enqueue(&scope, &request)?;
    drop(store);

    // Given records written by a build that still had workflows: a queued
    // workflow child and a finished planning job whose plan row references it
    // without a cascading delete.
    {
        let db = rusqlite::Connection::open(settings.home.join("jobs.sqlite3"))?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
            CREATE TABLE workflows (id INTEGER PRIMARY KEY AUTOINCREMENT,prepared TEXT NOT NULL,state TEXT NOT NULL,created INTEGER NOT NULL DEFAULT(unixepoch()),error TEXT,tokens TEXT NOT NULL DEFAULT '0');
            CREATE TABLE workflow_jobs (workflow INTEGER NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,job INTEGER NOT NULL UNIQUE,role TEXT NOT NULL,result TEXT,PRIMARY KEY(workflow,job));
            CREATE TABLE goal_reports (job INTEGER PRIMARY KEY REFERENCES workflow_jobs(job) ON DELETE CASCADE,report TEXT NOT NULL);
            CREATE TABLE goal_verifications (job INTEGER PRIMARY KEY REFERENCES workflow_jobs(job) ON DELETE CASCADE,result TEXT NOT NULL);
            CREATE TABLE delegations (command TEXT PRIMARY KEY,workflow INTEGER NOT NULL UNIQUE REFERENCES workflows(id) ON DELETE CASCADE,owner TEXT,fingerprint BLOB NOT NULL);
            CREATE TABLE delegation_parents (workflow INTEGER PRIMARY KEY REFERENCES delegations(workflow) ON DELETE CASCADE,parent INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE);
            CREATE TABLE task_messages (id INTEGER PRIMARY KEY AUTOINCREMENT,task INTEGER NOT NULL REFERENCES delegations(workflow) ON DELETE CASCADE,sender INTEGER REFERENCES jobs(id) ON DELETE SET NULL,recipient INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,text TEXT NOT NULL);
            CREATE TABLE task_followups (command TEXT PRIMARY KEY,task INTEGER NOT NULL REFERENCES delegations(workflow) ON DELETE CASCADE,job INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,fingerprint BLOB NOT NULL);
            CREATE TABLE task_notifications (worker INTEGER PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,job INTEGER REFERENCES jobs(id) ON DELETE SET NULL);
            CREATE TABLE task_delivery_cursors (job INTEGER PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,after INTEGER NOT NULL);
            CREATE TABLE task_notification_stops (scope TEXT NOT NULL,conversation INTEGER NOT NULL);
            CREATE TABLE workflow_plans (id INTEGER PRIMARY KEY AUTOINCREMENT,command TEXT NOT NULL UNIQUE,revision INTEGER NOT NULL DEFAULT 1,
                state TEXT NOT NULL,prepared TEXT NOT NULL,job INTEGER NOT NULL UNIQUE REFERENCES jobs(id),content TEXT,workflow INTEGER UNIQUE,error TEXT);
            CREATE TABLE repair_memories (id INTEGER PRIMARY KEY AUTOINCREMENT,scope TEXT NOT NULL,source_workflow INTEGER NOT NULL UNIQUE,payload TEXT NOT NULL,enabled INTEGER NOT NULL DEFAULT 1);
            INSERT INTO workflows(id,prepared,state) VALUES(1,'{}','running');",
        )?;
        db.execute("INSERT INTO workflow_jobs(workflow,job,role) VALUES(1,?1,'worker')", [workflow_child.id])?;
        db.execute("UPDATE jobs SET state='completed',conversation=9 WHERE id=?1", [planning.id])?;
        db.execute(
            "INSERT INTO workflow_plans(command,state,prepared,job) VALUES('plan','ready','{}',?1)",
            [planning.id],
        )?;
    }

    // When the store reopens.
    let store = JobStore::open(settings, workspaces)?;

    // Then the workflow child is cancelled instead of running as an ordinary
    // job, ordinary work is untouched and the retired tables are gone.
    assert_eq!(store.get(workflow_child.id)?.job.state, JobState::Cancelled);
    assert_eq!(store.get(ordinary.id)?.job.state, JobState::Queued);
    assert_eq!(store.claim(&scope)?.unwrap().job.id, ordinary.id);
    drop(store);
    {
        let db = rusqlite::Connection::open(root.path().join("home").join("jobs.sqlite3"))?;
        for table in RETIRED {
            let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [table], |row| {
                row.get(0)
            })?;
            assert!(!exists, "{table} must be dropped");
        }
    }

    // A conversation whose job a plan once referenced can now be deleted.
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let store = JobStore::open(settings.clone(), settings.workspaces(settings.application()?)?)?;
    store.purge_conversation(Provider::Codex, "default", workspace, 9)?;
    assert!(store.get(planning.id).is_err());
    Ok(())
}
