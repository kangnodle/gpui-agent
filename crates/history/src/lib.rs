#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! SQLite conversation persistence. Native runtime and host composition are
//! external.
mod context_transfers;
use anyhow::{Context,
             Result,
             ensure};
use gpui_agent_domain::{Completion,
                        ConversationSummary,
                        Event,
                        MatchedMessage,
                        Prompt,
                        Provider,
                        SavedConversation,
                        SavedMessage,
                        provider_name};
use gpui_agent_ports::{ConversationRepository,
                       EditOutcome,
                       ForkOutcome,
                       Turn};
use rusqlite::{Connection,
               OptionalExtension,
               params};
use std::{collections::{HashMap,
                        HashSet},
          path::Path,
          sync::Mutex,
          time::{Duration,
                 Instant}};

/// Streaming deltas are coalesced so each one does not cost its own fsync.
const TEXT_FLUSH_INTERVAL: Duration = Duration::from_millis(200);

struct Database {
    connection: Connection,
    active: HashSet<i64>,
    pending_text: HashMap<i64, String>,
    last_flush: Instant,
}

impl Database {
    // shortcut: a hard crash loses text buffered since the last flush; upgrade
    // if partial streams must survive crashes.
    fn flush_text(&mut self) -> Result<()> {
        self.last_flush = Instant::now();
        if self.pending_text.is_empty() {
            return Ok(());
        }
        let tx = self.connection.transaction()?;
        for (message_id, text) in &self.pending_text {
            tx.execute("UPDATE messages SET text=text || ?1 WHERE id=?2", params![text, message_id])?;
        }
        tx.commit()?;
        self.pending_text.clear();
        Ok(())
    }
}

pub struct History {
    database: Mutex<Database>,
    /// Separate WAL reader so list/search/load never queue behind streaming
    /// writes.
    reader: Mutex<Connection>,
    workspace: String,
}

impl History {
    pub fn open(path: &Path, cwd: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(path)?;
        }
        let mut connection = Connection::open(path).context("Cannot open conversation database")?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS conversations (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 provider TEXT NOT NULL,
                 workspace TEXT NOT NULL,
                 title TEXT NOT NULL,
                 model TEXT,
                 session_id TEXT,
                 updated_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE INDEX IF NOT EXISTS conversation_list
                 ON conversations(provider, workspace, updated_at DESC, id DESC);
             CREATE TABLE IF NOT EXISTS messages (
                 id INTEGER PRIMARY KEY,
                 conversation_id INTEGER NOT NULL REFERENCES conversations(id),
                 user INTEGER NOT NULL,
                 text TEXT NOT NULL,
                 incomplete INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS message_order ON messages(conversation_id, id);",
        )?;
        let schema: String = connection.query_row("SELECT sql FROM sqlite_schema WHERE name='conversations'", [], |row| row.get(0))?;
        if !schema.to_ascii_uppercase().contains("AUTOINCREMENT") {
            // Deleted IDs must never identify a new conversation in another
            // client's stale list.
            connection.execute_batch("PRAGMA foreign_keys=OFF;")?;
            let tx = connection.transaction()?;
            tx.execute_batch(
                "CREATE TABLE conversations_new (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     provider TEXT NOT NULL,
                     workspace TEXT NOT NULL,
                     title TEXT NOT NULL,
                     model TEXT,
                     session_id TEXT,
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO conversations_new SELECT * FROM conversations;
                 DROP TABLE conversations;
                 ALTER TABLE conversations_new RENAME TO conversations;
                 CREATE INDEX conversation_list ON conversations(provider,workspace,updated_at DESC,id DESC);",
            )?;
            tx.commit()?;
            connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        }
        for column in ["pinned", "archived"] {
            let exists = connection
                .prepare("PRAGMA table_info(conversations)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .any(|name| name == column);
            if !exists {
                connection.execute_batch(&format!("ALTER TABLE conversations ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0"))?;
            }
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS timeline (
            id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
            event TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS timeline_message ON timeline(message_id,id);
            CREATE TABLE IF NOT EXISTS attachments (
                id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                content TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS attachments_message ON attachments(message_id,id);
            CREATE TABLE IF NOT EXISTS message_guidance (
                message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                skills TEXT NOT NULL,
                receipt TEXT
            );",
        )?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversation_versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                question_index INTEGER NOT NULL,
                group_id INTEGER NOT NULL,
                context TEXT,
                UNIQUE(conversation_id,question_index)
            );
            CREATE INDEX IF NOT EXISTS version_group ON conversation_versions(group_id);",
        )?;
        context_transfers::initialize(&connection)?;
        let reader = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .context("Cannot open conversation database for reading")?;
        reader.busy_timeout(Duration::from_secs(5))?;
        Ok(Self {
            database: Mutex::new(Database {
                connection,
                active: HashSet::new(),
                pending_text: HashMap::new(),
                last_flush: Instant::now(),
            }),
            reader: Mutex::new(reader),
            // Canonicalization is only a database key; native runtimes retain their normal cwd.
            workspace: std::fs::canonicalize(cwd)?.to_string_lossy().into_owned(),
        })
    }

    fn reader(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.reader
            .lock()
            .map_err(|error| anyhow::anyhow!("Conversation reader lock poisoned: {error}"))
    }
}

impl Drop for History {
    fn drop(&mut self) {
        if let Ok(mut db) = self.database.lock() {
            let _ = db.flush_text();
        }
    }
}

fn load_conversation(connection: &Connection, workspace: &str, provider: Provider, id: i64) -> Result<Option<SavedConversation>> {
    let saved = connection
        .query_row(
            "SELECT model, session_id FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3",
            params![id, provider_name(provider), workspace],
            |row| {
                Ok(SavedConversation {
                    timeline: Vec::new(),
                    id,
                    model: row.get(0)?,
                    session_id: row.get(1)?,
                    messages: Vec::new(),
                })
            },
        )
        .optional()?;
    let Some(mut saved) = saved else {
        return Ok(None);
    };
    let mut query = connection.prepare("SELECT user, text, incomplete, id FROM messages WHERE conversation_id=?1 ORDER BY id")?;
    saved.messages = query
        .query_map([id], |row| {
            Ok(SavedMessage {
                skills: vec![],
                guidance: None,
                attachments: Vec::new(),
                id: Some(row.get(3)?),
                user: row.get(0)?,
                text: row.get(1)?,
                incomplete: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut guidance =
        connection.prepare("SELECT g.message_id,g.skills,g.receipt FROM message_guidance g JOIN messages m ON m.id=g.message_id WHERE m.conversation_id=?1")?;
    for row in guidance.query_map([id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
    })? {
        let (message_id, skills, receipt) = row?;
        let message = saved
            .messages
            .iter_mut()
            .find(|message| message.id == Some(message_id))
            .context("Orphan guidance receipt")?;
        message.skills = serde_json::from_str(&skills)?;
        message.guidance = receipt.as_deref().map(serde_json::from_str).transpose()?;
    }
    let mut attachments =
        connection.prepare("SELECT a.message_id,a.content FROM attachments a JOIN messages m ON m.id=a.message_id WHERE m.conversation_id=?1 ORDER BY a.id")?;
    for row in attachments.query_map([id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))? {
        let (message_id, content) = row?;
        let message = saved
            .messages
            .iter_mut()
            .find(|message| message.id == Some(message_id))
            .context("Orphan attachment")?;
        message.attachments.push(serde_json::from_str(&content)?);
    }
    let mut events =
        connection.prepare("SELECT t.message_id,t.event FROM timeline t JOIN messages m ON m.id=t.message_id WHERE m.conversation_id=?1 ORDER BY t.id")?;
    for row in events.query_map([id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))? {
        let (message_id, event) = row?;
        saved.timeline.push(gpui_agent_domain::TimelineEntry {
            message_id,
            event: serde_json::from_str(&event)?,
        });
    }
    Ok(Some(saved))
}

impl History {
    fn begin_turn(&self, provider: Provider, conversation_id: Option<i64>, prompt: &mut Prompt, repair: bool) -> Result<Turn> {
        let mut db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(anyhow::anyhow!("Conversation database lock poisoned: {error}")),
        };
        if let Some(id) = conversation_id {
            ensure!(!db.active.contains(&id), "Conversation already has an active response");
        }
        let tx = db.connection.transaction()?;
        let mut context = Vec::new();
        let id = if let Some(id) = conversation_id {
            let session: Option<String> = tx
                .query_row(
                    "SELECT session_id FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3",
                    params![id, provider_name(provider), self.workspace],
                    |row| row.get(0),
                )
                .optional()?
                .context("Conversation not found for this provider and workspace")?;
            if session.is_none() {
                context = context_transfers::prepared_context(&tx, id)?.context("No native session was saved; start a new conversation")?;
                if prompt.model.is_none() {
                    prompt.model = tx
                        .query_row(
                            "SELECT c.model FROM conversations c JOIN context_transfers t ON t.target=c.id WHERE c.id=?1 AND t.kind='fork'",
                            [id],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .optional()?
                        .flatten();
                }
            }
            prompt.session_id = session;
            id
        } else {
            let title: String = gpui_agent_domain::conversation_title(if prompt.text.trim().is_empty() {
                prompt.attachments.first().map(|a| a.name()).unwrap_or("Attachment")
            } else {
                &prompt.text
            });
            tx.execute(
                "INSERT INTO conversations(provider,workspace,title,model,session_id) VALUES (?1,?2,?3,?4,?5)",
                params![provider_name(provider), self.workspace, title, prompt.model, prompt.session_id],
            )?;
            tx.last_insert_rowid()
        };
        let mut attachments = prompt.attachments.clone();
        if !repair {
            attachments.extend(context_transfers::pending_context(&tx, id)?);
        }
        let mut combined = attachments.clone();
        combined.extend(context.iter().cloned());
        gpui_agent_domain::validate_attachments(&combined).map_err(anyhow::Error::msg)?;
        tx.execute(
            "UPDATE conversations SET archived=0, model=?1, updated_at=unixepoch() WHERE id=?2",
            params![prompt.model, id],
        )?;
        if !repair {
            tx.execute(
                "INSERT INTO messages(conversation_id,user,text,incomplete) VALUES (?1,1,?2,0)",
                params![id, prompt.text],
            )?;
            let user_message_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO message_guidance(message_id,skills,receipt) VALUES (?1,?2,?3)",
                params![
                    user_message_id,
                    serde_json::to_string(&prompt.skills)?,
                    prompt
                        .guidance
                        .as_ref()
                        .map(|guidance| serde_json::to_string(&guidance.receipt()))
                        .transpose()?
                ],
            )?;
            for attachment in &attachments {
                tx.execute(
                    "INSERT INTO attachments(message_id,content) VALUES (?1,?2)",
                    params![user_message_id, serde_json::to_string(attachment)?],
                )?;
            }
            context_transfers::attach(&tx, id, user_message_id)?;
        }
        tx.execute("INSERT INTO messages(conversation_id,user,text,incomplete) VALUES (?1,0,'',1)", [id])?;
        let message_id = tx.last_insert_rowid();
        if repair {
            tx.execute(
                "INSERT INTO message_guidance(message_id,skills,receipt) VALUES (?1,?2,?3)",
                params![
                    message_id,
                    serde_json::to_string(&prompt.skills)?,
                    prompt
                        .guidance
                        .as_ref()
                        .map(|guidance| serde_json::to_string(&guidance.receipt()))
                        .transpose()?
                ],
            )?;
        }
        tx.commit()?;
        db.active.insert(id);
        prompt.attachments = combined;
        Ok(Turn {
            conversation_id: id,
            message_id,
        })
    }
}

impl ConversationRepository for History {
    fn list(&self, provider: Provider, search: &str) -> Result<Vec<ConversationSummary>> {
        let search = search.trim();
        let db = self.reader()?;
        let mut query = db.prepare(
            "SELECT c.id, c.title, c.updated_at, m.id, m.user, m.text, c.pinned, c.archived FROM conversations c
             LEFT JOIN messages m ON m.id =
                 (SELECT id FROM messages WHERE conversation_id=c.id AND ?3<>'' AND instr(lower(text),lower(?3))>0 ORDER BY id LIMIT 1)
             WHERE c.provider=?1 AND c.workspace=?2 AND
                 (?3='' OR instr(lower(c.title),lower(?3))>0 OR m.id IS NOT NULL)
             ORDER BY c.pinned DESC, c.updated_at DESC, c.id DESC",
        )?;
        Ok(query
            .query_map(params![provider_name(provider), self.workspace, search], |row| {
                let message_id: Option<i64> = row.get(3)?;
                let matched_message = match message_id {
                    | Some(message_id) => Some(MatchedMessage {
                        message_id,
                        user: row.get(4)?,
                        preview: gpui_agent_domain::search::preview(&row.get::<_, String>(5)?, search),
                    }),
                    | None => None,
                };
                Ok(ConversationSummary {
                    pinned: row.get(6)?,
                    archived: row.get(7)?,
                    execution_state: None,
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    matched_message,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn load(&self, provider: Provider, id: i64) -> Result<Option<SavedConversation>> {
        self.database
            .lock()
            .map_err(|error| anyhow::anyhow!("Conversation database lock poisoned: {error}"))?
            .flush_text()?;
        let reader = self.reader()?;
        load_conversation(&reader, &self.workspace, provider, id)
    }

    fn transfer(&self, provider: Provider, command: &gpui_agent_domain::ContextTransferCommand) -> Result<gpui_agent_domain::ContextTransfer> {
        self.context_transfer(provider, command)
    }

    fn transfers(&self, provider: Provider, id: i64, after: i64) -> Result<Vec<gpui_agent_domain::ContextTransfer>> {
        self.context_transfers(provider, id, after)
    }

    fn begin(&self, provider: Provider, conversation_id: Option<i64>, prompt: &mut Prompt) -> Result<Turn> {
        self.begin_turn(provider, conversation_id, prompt, false)
    }

    fn begin_repair(&self, provider: Provider, conversation: i64, prompt: &mut Prompt) -> Result<Turn> {
        self.begin_turn(provider, Some(conversation), prompt, true)
    }

    fn fork(&self, provider: Provider, id: i64, message_id: i64, title: &str, context: &[gpui_agent_domain::Attachment]) -> Result<ForkOutcome> {
        let mut db = self
            .database
            .lock()
            .map_err(|error| anyhow::anyhow!("Conversation database lock poisoned: {error}"))?;
        let active = db.active.contains(&id);
        let tx = db.connection.transaction()?;
        let owned: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3)",
            params![id, provider_name(provider), self.workspace],
            |row| row.get(0),
        )?;
        if !owned {
            return Ok(ForkOutcome::NotFound);
        }
        let last: Option<i64> = tx
            .query_row(
                "SELECT id FROM messages WHERE conversation_id=?1 AND user=1 ORDER BY id DESC LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let incomplete: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND incomplete=1)", [id], |row| {
            row.get(0)
        })?;
        if active || incomplete || last != Some(message_id) {
            return Ok(ForkOutcome::Busy);
        }
        let prefix: Vec<i64> = tx
            .prepare("SELECT id FROM messages WHERE conversation_id=?1 AND id<?2 ORDER BY id")?
            .query_map(params![id, message_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let question_index = i64::try_from(prefix.len())?;
        let group: Option<i64> = tx
            .query_row(
                "SELECT group_id FROM conversation_versions WHERE conversation_id=?1 AND question_index=?2",
                params![id, question_index],
                |row| row.get(0),
            )
            .optional()?;
        let group = if let Some(group) = group {
            group
        } else {
            tx.execute(
                "INSERT INTO conversation_versions(conversation_id,question_index,group_id) VALUES (?1,?2,0)",
                params![id, question_index],
            )?;
            let group = tx.last_insert_rowid();
            tx.execute("UPDATE conversation_versions SET group_id=?1 WHERE id=?1", [group])?;
            group
        };
        tx.execute(
            "INSERT INTO conversations(provider,workspace,title,model) SELECT provider,workspace,?2,model FROM conversations WHERE id=?1",
            params![id, title],
        )?;
        let next = tx.last_insert_rowid();
        for old in &prefix {
            tx.execute(
                "INSERT INTO messages(conversation_id,user,text,incomplete) SELECT ?2,user,text,incomplete FROM messages WHERE id=?1",
                params![old, next],
            )?;
            let new = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO message_guidance(message_id,skills,receipt) SELECT ?2,skills,receipt FROM message_guidance WHERE message_id=?1",
                params![old, new],
            )?;
            tx.execute(
                "INSERT INTO attachments(message_id,content) SELECT ?2,content FROM attachments WHERE message_id=?1 ORDER BY id",
                params![old, new],
            )?;
            tx.execute(
                "INSERT INTO timeline(message_id,event) SELECT ?2,event FROM timeline WHERE message_id=?1 ORDER BY id",
                params![old, new],
            )?;
        }
        tx.execute(
            "INSERT INTO conversation_versions(conversation_id,question_index,group_id,context) VALUES (?1,?2,?3,?4)",
            params![next, question_index, group, serde_json::to_string(context)?],
        )?;
        tx.commit()?;
        Ok(ForkOutcome::Created(next))
    }

    fn versions(&self, provider: Provider, id: i64) -> Result<Vec<gpui_agent_domain::ConversationVersion>> {
        let db = self.reader()?;
        let mut query = db.prepare(
            "SELECT v.conversation_id,v.question_index FROM conversation_versions v JOIN conversations c ON c.id=v.conversation_id
             WHERE c.provider=?2 AND c.workspace=?3 AND v.group_id=(
                 SELECT group_id FROM conversation_versions WHERE conversation_id=?1 ORDER BY question_index DESC LIMIT 1)
             ORDER BY v.id",
        )?;
        Ok(query
            .query_map(params![id, provider_name(provider), self.workspace], |row| {
                Ok(gpui_agent_domain::ConversationVersion {
                    conversation_id: row.get(0)?,
                    question_index: row.get::<_, u32>(1)? as usize,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn rename(&self, provider: Provider, id: i64, title: &str) -> Result<EditOutcome> {
        let db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(anyhow::anyhow!("Conversation database lock poisoned: {error}")),
        };
        if db.active.contains(&id) {
            let owned: bool = db.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3)",
                params![id, provider_name(provider), self.workspace],
                |row| row.get(0),
            )?;
            return Ok(if owned { EditOutcome::Busy } else { EditOutcome::NotFound });
        }
        let changed = db.connection.execute(
            "UPDATE conversations SET title=?1, updated_at=unixepoch() WHERE id=?2 AND provider=?3 AND workspace=?4",
            params![title, id, provider_name(provider), self.workspace],
        )?;
        Ok(if changed == 0 { EditOutcome::NotFound } else { EditOutcome::Done })
    }

    fn set_state(&self, provider: Provider, id: i64, state: &gpui_agent_domain::ConversationState) -> Result<EditOutcome> {
        let db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(anyhow::anyhow!("Conversation database lock poisoned: {error}")),
        };
        let changed = db.connection.execute(
            "UPDATE conversations SET pinned=coalesce(?1,pinned), archived=coalesce(?2,archived) WHERE id=?3 AND provider=?4 AND workspace=?5",
            params![state.pinned, state.archived, id, provider_name(provider), self.workspace],
        )?;
        Ok(if changed == 0 { EditOutcome::NotFound } else { EditOutcome::Done })
    }

    fn delete(&self, provider: Provider, id: i64) -> Result<EditOutcome> {
        let mut db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(anyhow::anyhow!("Conversation database lock poisoned: {error}")),
        };
        if db.active.contains(&id) {
            let owned: bool = db.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3)",
                params![id, provider_name(provider), self.workspace],
                |row| row.get(0),
            )?;
            return Ok(if owned { EditOutcome::Busy } else { EditOutcome::NotFound });
        }
        let tx = db.connection.transaction()?;
        tx.execute(
            "DELETE FROM messages WHERE conversation_id IN
             (SELECT id FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3)",
            params![id, provider_name(provider), self.workspace],
        )?;
        let changed = tx.execute(
            "DELETE FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3",
            params![id, provider_name(provider), self.workspace],
        )?;
        tx.commit()?;
        Ok(if changed == 0 { EditOutcome::NotFound } else { EditOutcome::Done })
    }

    fn event(&self, turn: Turn, event: &Event) -> Result<()> {
        let mut db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(anyhow::anyhow!("Conversation database lock poisoned: {error}")),
        };
        match event {
            | Event::Session {
                id,
            } => {
                db.connection
                    .execute("UPDATE conversations SET session_id=?1 WHERE id=?2", params![id, turn.conversation_id])?;
            },
            | Event::Text {
                delta,
            } => {
                db.pending_text.entry(turn.message_id).or_default().push_str(delta);
                if db.last_flush.elapsed() >= TEXT_FLUSH_INTERVAL {
                    db.flush_text()?;
                }
            },
            | Event::Verification {
                ..
            }
            | Event::Tool {
                ..
            }
            | Event::Permission {
                ..
            }
            | Event::UserInput {
                ..
            }
            | Event::Elicitation {
                ..
            }
            | Event::InteractionResolved {
                ..
            }
            | Event::Usage {
                ..
            }
            | Event::RuntimeStatus {
                ..
            }
            | Event::InputDelivery {
                ..
            }
            | Event::TurnEnd {
                ..
            } => {
                db.connection.execute(
                    "INSERT INTO timeline(message_id,event) VALUES (?1,?2)",
                    params![turn.message_id, serde_json::to_string(event)?],
                )?;
            },
            | _ => {},
        }
        Ok(())
    }

    fn finish(&self, turn: Turn, completion: Option<&Completion>) -> Result<()> {
        let mut db = match self.database.lock() {
            | Ok(db) => db,
            | Err(error) => return Err(anyhow::anyhow!("Conversation database lock poisoned: {error}")),
        };
        db.active.remove(&turn.conversation_id);
        db.flush_text()?;
        let tx = db.connection.transaction()?;
        if let Some(Completion {
            session_id,
            text,
        }) = completion
        {
            tx.execute("UPDATE messages SET text=?1, incomplete=0 WHERE id=?2", params![text, turn.message_id])?;
            tx.execute("UPDATE conversations SET session_id=?1 WHERE id=?2", params![session_id, turn.conversation_id])?;
        }
        tx.execute("UPDATE conversations SET updated_at=unixepoch() WHERE id=?1", [turn.conversation_id])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_deltas_are_coalesced_but_visible_to_load_and_kept_when_cancelled() -> Result<()> {
        let root = tempfile::tempdir()?;
        let history = History::open(&root.path().join("history.sqlite3"), root.path())?;
        let turn = history.begin(Provider::Codex, None, &mut prompt("Question"))?;
        for delta in ["Hel", "lo ", "world"] {
            history.event(
                turn,
                &Event::Text {
                    delta: delta.into(),
                },
            )?;
        }
        assert_eq!(history.load(Provider::Codex, turn.conversation_id)?.unwrap().messages[1].text, "Hello world");
        history.event(
            turn,
            &Event::Text {
                delta: "!".into(),
            },
        )?;
        history.finish(turn, None)?;
        assert_eq!(history.load(Provider::Codex, turn.conversation_id)?.unwrap().messages[1].text, "Hello world!");
        Ok(())
    }

    #[tokio::test]
    async fn revisions_preserve_history_and_replay_only_the_prefix_in_a_fresh_session() -> Result<()> {
        use gpui_agent_application::{Application,
                                     ErrorKind};
        use gpui_agent_contracts::ReviseRequest;
        use std::sync::Arc;
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = Arc::new(History::open(&path, root.path())?);
        let app = Application::new(std::array::from_fn(|_| Err(anyhow::anyhow!("unused runtime"))), history.clone());
        let first = history.begin(Provider::Codex, None, &mut prompt("Earlier question"))?;
        history.event(
            first,
            &Event::Tool {
                call: serde_json::from_value(serde_json::json!({
                    "id":"earlier-tool", "title":"Earlier tool", "status":"completed", "output":"Earlier result"
                }))?,
            },
        )?;
        history.finish(
            first,
            Some(&Completion {
                session_id: "old-native-session".into(),
                text: "Earlier answer".into(),
            }),
        )?;
        let last = history.begin(Provider::Codex, Some(first.conversation_id), &mut prompt("Question to replace"))?;
        history.finish(
            last,
            Some(&Completion {
                session_id: "old-native-session".into(),
                text: "Answer to exclude".into(),
            }),
        )?;
        let original = history.load(Provider::Codex, first.conversation_id)?.unwrap();
        let revision = app
            .revise_conversation(
                Provider::Codex,
                original.id,
                ReviseRequest {
                    extensions: None,
                    skills: None,
                    message_id: original.messages[2].id.unwrap(),
                    text: "Edited question".into(),
                    model: None,
                    claude_permission_mode: None,
                    codex_execution_policy: None,
                },
            )
            .await?;
        let new_id = revision.conversation.id;
        assert_ne!(new_id, original.id);
        assert_eq!(revision.conversation.messages.len(), 2);
        assert_eq!(revision.conversation.messages[1].text, "Earlier answer");
        assert!(revision.conversation.session_id.is_none() && revision.request.session_id.is_none());
        assert_eq!(revision.versions.len(), 2);
        assert_eq!(revision.versions[0].question_index, 2);
        assert_eq!(revision.conversation.timeline[0].message_id, revision.conversation.messages[1].id.unwrap());
        let mut fresh = prompt(&revision.request.text);
        fresh.session_id = Some("must-not-resume".into());
        let turn = history.begin(Provider::Codex, Some(new_id), &mut fresh)?;
        assert!(fresh.session_id.is_none());
        let context = fresh.attachments[0].text_context().unwrap();
        assert!(context.contains("Earlier question") && context.contains("Earlier answer") && context.contains("Earlier result"));
        assert!(!context.contains("Question to replace") && !context.contains("Answer to exclude") && !context.contains("old-native-session"));
        history.finish(
            turn,
            Some(&Completion {
                session_id: "fresh-native-session".into(),
                text: "Edited answer".into(),
            }),
        )?;
        let saved = history.load(Provider::Codex, new_id)?.unwrap();
        assert_eq!(saved.messages.len(), 4);
        assert_eq!(saved.messages[2].text, "Edited question");
        assert!(saved.messages[2].attachments.is_empty(), "Internal context is not a user attachment");
        assert_eq!(history.load(Provider::Codex, original.id)?.unwrap().messages[3].text, "Answer to exclude");
        assert_eq!(
            app.revise_conversation(
                Provider::Codex,
                original.id,
                ReviseRequest {
                    extensions: None,
                    skills: None,
                    message_id: original.messages[0].id.unwrap(),
                    text: "Stale edit".into(),
                    model: None,
                    claude_permission_mode: None,
                    codex_execution_policy: None,
                }
            )
            .await
            .unwrap_err()
            .0,
            ErrorKind::Busy
        );
        assert!(matches!(
            history.fork(Provider::Claude, original.id, original.messages[2].id.unwrap(), "wrong", &[])?,
            ForkOutcome::NotFound
        ));
        let again = app
            .revise_conversation(
                Provider::Codex,
                new_id,
                ReviseRequest {
                    extensions: None,
                    skills: None,
                    message_id: saved.messages[2].id.unwrap(),
                    text: "Third version".into(),
                    model: None,
                    claude_permission_mode: None,
                    codex_execution_policy: None,
                },
            )
            .await?;
        assert_eq!(again.versions.len(), 3);
        drop(app);
        drop(history);
        let history = History::open(&path, root.path())?;
        assert_eq!(history.versions(Provider::Codex, new_id)?.len(), 3);
        assert_eq!(
            history.load(Provider::Codex, new_id)?.unwrap().session_id.as_deref(),
            Some("fresh-native-session")
        );
        assert_eq!(history.delete(Provider::Codex, original.id)?, EditOutcome::Done);
        assert_eq!(history.load(Provider::Codex, new_id)?.unwrap().messages[0].text, "Earlier question");
        assert_eq!(history.versions(Provider::Codex, new_id)?.len(), 2);
        let pending = history.begin(Provider::Codex, None, &mut prompt("Still running"))?;
        let incomplete = history.load(Provider::Codex, pending.conversation_id)?.unwrap();
        assert!(matches!(
            history.fork(Provider::Codex, pending.conversation_id, incomplete.messages[0].id.unwrap(), "blocked", &[])?,
            ForkOutcome::Busy
        ));
        history.finish(pending, None)?;
        Ok(())
    }

    fn prompt(text: &str) -> Prompt {
        Prompt {
            mcp_servers: None,
            skills: vec![],
            guidance: None,
            claude_permission_mode: None,
            codex_execution_policy: None,
            attachments: Vec::new(),
            text: text.into(),
            model: Some("test-model".into()),
            session_id: None,
        }
    }

    #[test]
    fn poisoned_database_returns_errors_without_writing() -> Result<()> {
        let root = tempfile::tempdir()?;
        let history = History::open(&root.path().join("history.sqlite3"), root.path())?;
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = history.database.lock().unwrap();
                panic!("poison conversation database");
            }))
            .is_err()
        );
        assert!(
            history
                .load(Provider::Codex, 1)
                .unwrap_err()
                .to_string()
                .contains("Conversation database lock poisoned")
        );
        assert!(history.begin(Provider::Codex, None, &mut prompt("must not be saved")).is_err());
        assert!(history.delete(Provider::Codex, 1).is_err());
        Ok(())
    }

    #[test]
    fn usage_is_persisted_on_interrupted_turns_and_restored_without_a_runtime() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = History::open(&path, root.path())?;
        let turn = history.begin(Provider::Codex, None, &mut prompt("usage"))?;
        let usage = gpui_agent_domain::Usage {
            scope: gpui_agent_domain::UsageScope::Session,
            input_tokens: Some(40),
            output_tokens: Some(2),
            total_tokens: Some(42),
            cached_input_tokens: Some(30),
            cache_write_tokens: None,
            context_window: Some(1000),
            cost_usd: None,
        };
        history.event(
            turn,
            &Event::Usage {
                usage: usage.clone(),
            },
        )?;
        history.finish(turn, None)?;
        drop(history);
        let history = History::open(&path, root.path())?;
        let saved = history.load(Provider::Codex, turn.conversation_id)?.unwrap();
        assert!(saved.messages[1].incomplete);
        assert_eq!(gpui_agent_domain::UsageReport::recorded(&saved.timeline, 1).usage, Some(usage));
        assert!(history.load(Provider::Claude, turn.conversation_id)?.is_none());
        Ok(())
    }

    #[test]
    fn pin_archive_survive_restart_preserve_sessions_and_respect_scope() -> Result<()> {
        use gpui_agent_domain::ConversationState;
        let root = tempfile::tempdir()?;
        let database = root.path().join("history.sqlite3");
        let history = History::open(&database, root.path())?;
        let mut ids = Vec::new();
        for provider in gpui_agent_domain::PROVIDERS {
            let first = history.begin(provider, None, &mut prompt("needle first"))?;
            history.finish(
                first,
                Some(&Completion {
                    session_id: "native-session".into(),
                    text: "needle response".into(),
                }),
            )?;
            let second = history.begin(provider, None, &mut prompt("second"))?;
            history.finish(second, None)?;
            assert_eq!(
                history.set_state(
                    provider,
                    first.conversation_id,
                    &ConversationState {
                        pinned: Some(true),
                        archived: Some(true)
                    }
                )?,
                EditOutcome::Done
            );
            assert_eq!(
                history.set_state(
                    provider,
                    second.conversation_id,
                    &ConversationState {
                        pinned: Some(false),
                        archived: None
                    }
                )?,
                EditOutcome::Done
            );
            ids.push((provider, first.conversation_id));
        }
        drop(history);
        let history = History::open(&database, root.path())?;
        let other_dir = root.path().join("other");
        std::fs::create_dir(&other_dir)?;
        let other = History::open(&database, &other_dir)?;
        for (provider, id) in ids {
            let all = history.list(provider, "")?;
            assert_eq!(all[0].id, id);
            assert!(all[0].pinned && all[0].archived);
            assert!(history.list(provider, "needle")?[0].archived);
            assert_eq!(
                other.set_state(
                    provider,
                    id,
                    &ConversationState {
                        archived: Some(false),
                        pinned: None
                    }
                )?,
                EditOutcome::NotFound
            );
            let wrong = gpui_agent_domain::PROVIDERS.into_iter().find(|p| *p != provider).unwrap();
            assert_eq!(history.set_state(wrong, id, &ConversationState::default())?, EditOutcome::NotFound);
            let mut input = prompt("continue");
            let turn = history.begin(provider, Some(id), &mut input)?;
            assert_eq!(input.session_id.as_deref(), Some("native-session"));
            history.finish(turn, None)?;
            let all = history.list(provider, "")?;
            assert!(all[0].pinned && !all[0].archived);
            assert_eq!(history.load(provider, id)?.unwrap().messages.len(), 4);
        }
        Ok(())
    }

    #[test]
    fn attachment_snapshots_survive_interruption_and_delete_with_the_message() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = History::open(&path, root.path())?;
        let attachment = gpui_agent_domain::Attachment::from_bytes("snapshot.rs".into(), b"immutable content").map_err(anyhow::Error::msg)?;
        let mut input = prompt("");
        input.attachments.push(attachment.clone());
        let turn = history.begin(Provider::Claude, None, &mut input)?;
        history.finish(turn, None)?;
        drop(history);
        let history = History::open(&path, root.path())?;
        let saved = history.load(Provider::Claude, turn.conversation_id)?.unwrap();
        assert_eq!(saved.messages[0].attachments, vec![attachment]);
        assert!(saved.messages[1].incomplete);
        assert_eq!(history.list(Provider::Claude, "")?[0].title, "snapshot.rs");
        history.rename(Provider::Claude, turn.conversation_id, "Renamed")?;
        assert_eq!(
            history.load(Provider::Claude, turn.conversation_id)?.unwrap().messages[0].attachments,
            input.attachments
        );
        history.delete(Provider::Claude, turn.conversation_id)?;
        let db = history.database.lock().unwrap();
        assert_eq!(db.connection.query_row("SELECT count(*) FROM attachments", [], |row| row.get::<_, i64>(0))?, 0);
        Ok(())
    }

    #[test]
    fn search_matches_titles_and_messages_without_duplicates_or_scope_leaks() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = History::open(&path, root.path())?;
        let mut ids = Vec::new();
        for provider in gpui_agent_domain::PROVIDERS {
            let first = history.begin(provider, None, &mut prompt("원래 질문 Needle 세계"))?;
            history.event(
                first,
                &Event::Session {
                    id: format!("{}-saved", provider_name(provider)),
                },
            )?;
            history.event(
                first,
                &Event::Text {
                    delta: "답변 NEEDLE 세계 100%_ + & '🦀'".into(),
                },
            )?;
            history.finish(first, None)?;
            history.rename(provider, first.conversation_id, "바뀐 제목 🦀")?;
            let second = history.begin(provider, None, &mut prompt("관계없는 대화"))?;
            history.finish(
                second,
                Some(&Completion {
                    session_id: "other".into(),
                    text: "다른 답변".into(),
                }),
            )?;
            ids.push((first.conversation_id, second.conversation_id));
        }
        drop(history);
        let history = History::open(&path, root.path())?;
        for (provider, (first, second)) in gpui_agent_domain::PROVIDERS.into_iter().zip(ids) {
            let saved = history.load(provider, first)?.unwrap();
            for query in ["제목 🦀", "원래 질문", "  nEeDlE 세계  ", "답변 NEEDLE", "%_", "+ & '🦀'"] {
                let results = history.list(provider, query)?;
                assert_eq!(results.len(), 1, "query: {query}");
                assert_eq!(results[0].id, first);
                if query == "제목 🦀" {
                    assert!(results[0].matched_message.is_none());
                } else {
                    let matched = results[0].matched_message.as_ref().unwrap();
                    let user = query == "원래 질문" || query.trim() == "nEeDlE 세계";
                    assert_eq!(matched.user, user);
                    assert_eq!(Some(matched.message_id), saved.messages[usize::from(!user)].id);
                    assert!(!gpui_agent_domain::match_ranges(&matched.preview, query).is_empty());
                }
            }
            assert!(history.list(provider, "missing")?.is_empty());
            assert!(history.list(provider, "' OR 1=1 --")?.is_empty());
            let all = history.list(provider, "  ")?;
            assert!(all.iter().all(|saved| saved.matched_message.is_none()));
            assert_eq!(all.iter().map(|saved| saved.id).collect::<Vec<_>>(), [second, first]);
            let saved = history.load(provider, first)?.unwrap();
            assert_eq!(saved.session_id, Some(format!("{}-saved", provider_name(provider))));
            assert!(saved.messages[1].incomplete);
            history.rename(provider, second, "새로운 Needle 제목")?;
            assert_eq!(history.list(provider, "needle")?.len(), 2);
            history.delete(provider, first)?;
            assert!(history.list(provider, "%_")?.is_empty());
            assert_eq!(history.list(provider, "needle")?[0].id, second);
        }
        let other = root.path().join("other");
        std::fs::create_dir(&other)?;
        let other = History::open(&path, &other)?;
        for provider in gpui_agent_domain::PROVIDERS {
            assert!(other.list(provider, "Needle")?.is_empty());
        }
        Ok(())
    }

    #[test]
    fn edits_migrate_old_database_preserve_history_and_delete_atomically() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let legacy = Connection::open(&path)?;
        legacy.execute_batch(
            "CREATE TABLE conversations (id INTEGER PRIMARY KEY, provider TEXT NOT NULL,
                 workspace TEXT NOT NULL, title TEXT NOT NULL, model TEXT, session_id TEXT,
                 updated_at INTEGER NOT NULL DEFAULT (unixepoch()));
             CREATE TABLE messages (id INTEGER PRIMARY KEY,
                 conversation_id INTEGER NOT NULL REFERENCES conversations(id),
                 user INTEGER NOT NULL, text TEXT NOT NULL, incomplete INTEGER NOT NULL);",
        )?;
        legacy.execute(
            "INSERT INTO conversations VALUES (5,'codex',?1,'old','model','native-session',1)",
            [std::fs::canonicalize(root.path())?.to_string_lossy().as_ref()],
        )?;
        legacy.execute_batch("INSERT INTO messages VALUES (1,5,1,'old question',0),(2,5,0,'old answer',0);")?;
        drop(legacy);

        let history = History::open(&path, root.path())?;
        let mut continuation = prompt("continue");
        let turn = history.begin(Provider::Codex, Some(5), &mut continuation)?;
        assert_eq!(history.rename(Provider::Claude, 5, "wrong provider")?, EditOutcome::NotFound);
        assert_eq!(history.delete(Provider::Claude, 5)?, EditOutcome::NotFound);
        assert_eq!(history.rename(Provider::Codex, 5, "busy")?, EditOutcome::Busy);
        assert_eq!(history.delete(Provider::Codex, 5)?, EditOutcome::Busy);
        history.finish(turn, None)?;
        assert_eq!(history.rename(Provider::Claude, 5, "wrong provider")?, EditOutcome::NotFound);
        assert_eq!(history.delete(Provider::Claude, 5)?, EditOutcome::NotFound);
        let other = root.path().join("other");
        std::fs::create_dir(&other)?;
        let other = History::open(&path, &other)?;
        assert_eq!(other.rename(Provider::Codex, 5, "wrong workspace")?, EditOutcome::NotFound);
        assert_eq!(other.delete(Provider::Codex, 5)?, EditOutcome::NotFound);
        drop(other);
        assert_eq!(history.rename(Provider::Codex, 5, "바뀐 제목 🦀")?, EditOutcome::Done);
        drop(history);
        let history = History::open(&path, root.path())?;
        assert_eq!(history.list(Provider::Codex, "")?[0].title, "바뀐 제목 🦀");
        let saved = history.load(Provider::Codex, 5)?.unwrap();
        assert_eq!(saved.session_id.as_deref(), Some("native-session"));
        assert_eq!(saved.messages.len(), 4);
        assert_eq!(saved.messages[1].text, "old answer");
        history
            .database
            .lock()
            .unwrap()
            .connection
            .execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON conversations BEGIN SELECT RAISE(ABORT,'fixture'); END;")?;
        assert!(history.delete(Provider::Codex, 5).is_err());
        assert_eq!(history.load(Provider::Codex, 5)?.unwrap().messages.len(), 4);
        history.database.lock().unwrap().connection.execute_batch("DROP TRIGGER fail_delete;")?;
        assert_eq!(history.delete(Provider::Codex, 5)?, EditOutcome::Done);
        assert_eq!(history.delete(Provider::Codex, 5)?, EditOutcome::NotFound);
        drop(history);
        let history = History::open(&path, root.path())?;
        assert!(history.list(Provider::Codex, "")?.is_empty());
        assert!(history.load(Provider::Codex, 5)?.is_none());
        let db = history.database.lock().unwrap();
        assert_eq!(db.connection.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))?, 0);
        assert_eq!(db.connection.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))?, 1);
        drop(db);
        let next = history.begin(Provider::Codex, None, &mut prompt("new conversation"))?;
        assert!(next.conversation_id > 5);
        Ok(())
    }

    #[test]
    fn reopen_restores_providers_models_sessions_and_interrupted_text() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("history.sqlite3");
        let history = History::open(&path, root.path())?;
        let mut ids = Vec::new();
        for provider in gpui_agent_domain::PROVIDERS {
            let turn = history.begin(provider, None, &mut prompt("첫 질문 🦀"))?;
            let session_id = format!("{}-session", provider_name(provider));
            history.event(
                turn,
                &Event::Session {
                    id: session_id.clone(),
                },
            )?;
            history.event(
                turn,
                &Event::Text {
                    delta: "부분 응답".into(),
                },
            )?;
            if provider != Provider::Antigravity {
                history.finish(
                    turn,
                    Some(&Completion {
                        session_id,
                        text: "최종 응답".into(),
                    }),
                )?;
            }
            ids.push(turn.conversation_id);
        }
        // Dropping without finish simulates a process interrupted during
        // streaming.
        drop(history);
        let history = History::open(&path, root.path())?;
        for (provider, id) in gpui_agent_domain::PROVIDERS.into_iter().zip(ids) {
            let list = history.list(provider, "")?;
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, id);
            assert_eq!(list[0].title, "첫 질문 🦀");
            let saved = history.load(provider, id)?.unwrap();
            assert_eq!(saved.model.as_deref(), Some("test-model"));
            assert_eq!(saved.session_id, Some(format!("{}-session", provider_name(provider))));
            assert_eq!(saved.messages.len(), 2);
            assert!(saved.messages[0].user);
            let partial = provider == Provider::Antigravity;
            assert_eq!(saved.messages[1].incomplete, partial);
            assert_eq!(saved.messages[1].text, if partial { "부분 응답" } else { "최종 응답" });
            let mut continuation = prompt("이어서");
            continuation.session_id = Some("untrusted-client-session".into());
            let turn = history.begin(provider, Some(id), &mut continuation)?;
            assert_eq!(continuation.session_id, saved.session_id);
            assert!(history.begin(provider, Some(id), &mut continuation).is_err());
            history.event(
                turn,
                &Event::Text {
                    delta: "중단된 답변".into(),
                },
            )?;
            history.finish(turn, None)?;
            let saved = history.load(provider, id)?.unwrap();
            assert_eq!(saved.messages.len(), 4);
            assert_eq!(saved.messages[3].text, "중단된 답변");
            assert!(saved.messages[3].incomplete);
            assert_eq!(history.list(provider, "")?.len(), 1);
            assert!(
                history
                    .load(
                        gpui_agent_domain::PROVIDERS[(gpui_agent_domain::provider_index(provider) + 1) % gpui_agent_domain::PROVIDERS.len()],
                        id
                    )?
                    .is_none()
            );
        }
        let other = root.path().join("other");
        std::fs::create_dir(&other)?;
        let other = History::open(&path, &other)?;
        assert!(other.list(Provider::Claude, "")?.is_empty());
        assert!(other.load(Provider::Claude, 1)?.is_none());
        let mut wrong_workspace = prompt("wrong");
        assert!(other.begin(Provider::Claude, Some(1), &mut wrong_workspace).is_err());
        Ok(())
    }
}
