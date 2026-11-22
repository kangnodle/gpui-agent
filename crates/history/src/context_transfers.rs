use super::*;
use gpui_agent_domain::{Attachment,
                        ContextTransfer,
                        ContextTransferCommand,
                        ContextTransferError,
                        ContextTransferKind,
                        ContextTransferState,
                        validate_attachments};

const COLUMNS: &str = "id,kind,source,target,boundary,attached_message,created";

pub(super) fn initialize(db: &Connection) -> Result<()> {
    // Accepted snapshots survive deletion of their source conversation.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS context_transfers (
        id INTEGER PRIMARY KEY AUTOINCREMENT,provider TEXT NOT NULL,workspace TEXT NOT NULL,command TEXT NOT NULL,request TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('fork','merge_back')),source INTEGER NOT NULL,
        target INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,boundary INTEGER NOT NULL,
        base_messages INTEGER NOT NULL,context TEXT NOT NULL,attached_message INTEGER,created INTEGER NOT NULL DEFAULT(unixepoch()),
        UNIQUE(provider,workspace,command));
        CREATE INDEX IF NOT EXISTS context_transfer_target ON context_transfers(target,kind,attached_message,id);
        CREATE INDEX IF NOT EXISTS context_transfer_source ON context_transfers(source,id);",
    )?;
    Ok(())
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextTransfer> {
    let attached_message_id: Option<i64> = row.get(5)?;
    let kind: String = row.get(1)?;
    let kind = serde_json::from_value(serde_json::json!(kind))
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(error)))?;
    Ok(ContextTransfer {
        id: row.get(0)?,
        kind,
        source_id: row.get(2)?,
        target_id: row.get(3)?,
        through_message_id: row.get(4)?,
        state: if attached_message_id.is_some() {
            ContextTransferState::Attached
        } else {
            ContextTransferState::Pending
        },
        attached_message_id,
        created_at: row.get(6)?,
    })
}

fn owned(db: &Connection, workspace: &str, provider: Provider, id: i64) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND provider=?2 AND workspace=?3)",
        params![id, provider_name(provider), workspace],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(ContextTransferError::NotFound.into());
    }
    Ok(())
}

pub(super) fn prepared_context(db: &Connection, id: i64) -> Result<Option<Vec<Attachment>>> {
    let raw: Option<String> = db
        .query_row("SELECT context FROM context_transfers WHERE target=?1 AND kind='fork'", [id], |row| row.get(0))
        .optional()?;
    let raw = match raw {
        Some(raw) => Some(raw),
        None => db.query_row("SELECT context FROM conversation_versions WHERE conversation_id=?1 AND question_index=(SELECT count(*) FROM messages WHERE conversation_id=?1) AND context IS NOT NULL", [id], |row| row.get(0)).optional()?,
    };
    Ok(raw.map(|raw| serde_json::from_str(&raw)).transpose()?)
}

pub(super) fn pending_context(db: &Connection, id: i64) -> Result<Vec<Attachment>> {
    let mut result = Vec::new();
    for raw in db
        .prepare("SELECT context FROM context_transfers WHERE target=?1 AND kind='merge_back' AND attached_message IS NULL ORDER BY id")?
        .query_map([id], |row| row.get::<_, String>(0))?
    {
        result.extend(serde_json::from_str::<Vec<Attachment>>(&raw?)?);
    }
    Ok(result)
}

pub(super) fn attach(db: &Connection, target: i64, message: i64) -> Result<()> {
    db.execute(
        "UPDATE context_transfers SET attached_message=?2 WHERE target=?1 AND attached_message IS NULL",
        params![target, message],
    )?;
    Ok(())
}

impl History {
    pub(super) fn context_transfer(&self, provider: Provider, command: &ContextTransferCommand) -> Result<ContextTransfer> {
        command.validate()?;
        let mut db = self
            .database
            .lock()
            .map_err(|error| anyhow::anyhow!("Conversation database lock poisoned: {error}"))?;
        db.flush_text()?;
        let tx = db.connection.transaction()?;
        let request = serde_json::to_string(command)?;
        let previous: Option<(i64, String)> = tx
            .query_row(
                "SELECT id,request FROM context_transfers WHERE provider=?1 AND workspace=?2 AND command=?3",
                params![provider_name(provider), self.workspace, command.command_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((id, stored)) = previous {
            if stored != request {
                return Err(ContextTransferError::Conflict.into());
            }
            return Ok(tx.query_row(&format!("SELECT {COLUMNS} FROM context_transfers WHERE id=?1"), [id], row)?);
        }
        let source = load_conversation(&tx, &self.workspace, provider, command.source_id)?.ok_or(ContextTransferError::NotFound)?;
        let ancestor_prefix = if let Some(target) = command.target_id {
            owned(&tx, &self.workspace, provider, target)?;
            let count: Option<i64> = tx
                .query_row(
                    "WITH RECURSIVE ancestors(id,base_messages) AS (
                SELECT source,base_messages FROM context_transfers WHERE target=?1 AND provider=?2 AND workspace=?3 AND kind='fork'
                UNION SELECT t.source,t.base_messages FROM context_transfers t JOIN ancestors a ON t.target=a.id
                WHERE t.provider=?2 AND t.workspace=?3 AND t.kind='fork')
                SELECT base_messages FROM ancestors WHERE id=?4 LIMIT 1",
                    params![command.source_id, provider_name(provider), self.workspace, target],
                    |row| row.get(0),
                )
                .optional()?;
            count.map(usize::try_from).transpose()?
        } else {
            None
        };
        let plan = command.prepare(source, ancestor_prefix)?;
        let (target, kind) = if let Some(target) = command.target_id {
            let has_session: bool = tx.query_row("SELECT session_id IS NOT NULL FROM conversations WHERE id=?1", [target], |row| row.get(0))?;
            let mut combined = pending_context(&tx, target)?;
            if !has_session && let Some(context) = prepared_context(&tx, target)? {
                combined.extend(context);
            }
            combined.extend(plan.context.iter().cloned());
            validate_attachments(&combined).map_err(ContextTransferError::Invalid)?;
            (target, ContextTransferKind::MergeBack)
        } else {
            let title = command
                .title
                .as_deref()
                .map(str::trim)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Branch of conversation {}", command.source_id));
            tx.execute(
                "INSERT INTO conversations(provider,workspace,title,model) VALUES(?1,?2,?3,?4)",
                params![provider_name(provider), self.workspace, title, plan.source.model],
            )?;
            let target = tx.last_insert_rowid();
            for message in &plan.source.messages {
                tx.execute(
                    "INSERT INTO messages(conversation_id,user,text,incomplete) VALUES(?1,?2,?3,0)",
                    params![target, message.user, message.text],
                )?;
                let id = tx.last_insert_rowid();
                for attachment in &message.attachments {
                    tx.execute(
                        "INSERT INTO attachments(message_id,content) VALUES(?1,?2)",
                        params![id, serde_json::to_string(attachment)?],
                    )?;
                }
                for entry in plan.source.timeline.iter().filter(|entry| Some(entry.message_id) == message.id) {
                    tx.execute(
                        "INSERT INTO timeline(message_id,event) VALUES(?1,?2)",
                        params![id, serde_json::to_string(&entry.event)?],
                    )?;
                }
            }
            (target, ContextTransferKind::Fork)
        };
        tx.execute("INSERT INTO context_transfers(provider,workspace,command,request,kind,source,target,boundary,base_messages,context) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![provider_name(provider), self.workspace, command.command_id, request, serde_json::to_value(kind)?.as_str(), command.source_id, target, plan.through_message_id, i64::try_from(plan.prefix_count)?, serde_json::to_string(&plan.context)?])?;
        let result = tx.query_row(&format!("SELECT {COLUMNS} FROM context_transfers WHERE id=?1"), [tx.last_insert_rowid()], row)?;
        tx.commit()?;
        Ok(result)
    }

    pub(super) fn context_transfers(&self, provider: Provider, id: i64, after: i64) -> Result<Vec<ContextTransfer>> {
        let db = self
            .database
            .lock()
            .map_err(|error| anyhow::anyhow!("Conversation database lock poisoned: {error}"))?;
        owned(&db.connection, &self.workspace, provider, id)?;
        Ok(db
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM context_transfers WHERE provider=?1 AND workspace=?2 AND (source=?3 OR target=?3) AND id>?4 ORDER BY id LIMIT 50"
            ))?
            .query_map(params![provider_name(provider), self.workspace, id, after], row)?
            .collect::<rusqlite::Result<_>>()?)
    }
}
