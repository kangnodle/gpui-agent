use super::*;
use gpui_agent_contracts::{Attachment,
                           UploadedAttachment};
use gpui_agent_domain::attachment::{MAX_SCOPE_UPLOADS,
                                    MAX_UPLOADS};
use sha2::{Digest,
           Sha256};

fn metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<UploadedAttachment> {
    Ok(UploadedAttachment {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        bytes: row.get(3)?,
    })
}
pub(super) fn upload(store: &JobStore, scope: &JobScope, attachment: &Attachment) -> ApplicationResult<UploadedAttachment> {
    let bytes = attachment.validate()? as u32;
    let kind = match attachment {
        | Attachment::Text {
            ..
        } => "text",
        | Attachment::Image {
            ..
        } => "image",
    };
    store.db(|db| {
        let scope = serde_json::to_string(scope)?;
        let payload = serde_json::to_string(attachment)?;
        let fingerprint = Sha256::digest(payload.as_bytes()).to_vec();
        let tx = db.transaction()?;
        if let Some(existing) = tx
            .query_row(
                "SELECT id,name,kind,bytes FROM uploaded_attachments WHERE scope=?1 AND fingerprint=?2",
                params![scope, fingerprint],
                metadata,
            )
            .optional()?
        {
            return Ok(existing);
        }
        let (total, scoped): (i64, i64) = tx.query_row("SELECT count(*),coalesce(sum(scope=?1),0) FROM uploaded_attachments", [&scope], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        if total >= MAX_UPLOADS as i64 || scoped >= MAX_SCOPE_UPLOADS as i64 {
            return Err(reject(
                ErrorKind::Busy,
                "Uploaded attachment limit reached (16 per scope, 128 per host); delete unused uploads",
            ));
        }
        tx.execute(
            "INSERT INTO uploaded_attachments(scope,fingerprint,payload,name,kind,bytes) VALUES(?1,?2,?3,?4,?5,?6)",
            params![scope, fingerprint, payload, attachment.name(), kind, bytes],
        )?;
        let result = UploadedAttachment {
            id: tx.last_insert_rowid(),
            name: attachment.name().into(),
            kind: kind.into(),
            bytes,
        };
        tx.commit()?;
        Ok(result)
    })
}
pub(super) fn list(store: &JobStore, scope: &JobScope) -> ApplicationResult<Vec<UploadedAttachment>> {
    store.db(|db| {
        Ok(db
            .prepare("SELECT id,name,kind,bytes FROM uploaded_attachments WHERE scope=?1 ORDER BY id")?
            .query_map([serde_json::to_string(scope)?], metadata)?
            .collect::<rusqlite::Result<_>>()?)
    })
}
pub(super) fn load(db: &Connection, scope: &JobScope, id: i64) -> Result<Attachment> {
    let payload: String = db
        .query_row(
            "SELECT payload FROM uploaded_attachments WHERE scope=?1 AND id=?2",
            params![serde_json::to_string(scope)?, id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| reject(ErrorKind::NotFound, "Attachment not found in this scope"))?;
    Ok(serde_json::from_str(&payload)?)
}
pub(super) fn delete(store: &JobStore, scope: &JobScope, id: i64) -> ApplicationResult<()> {
    store.db(|db| {
        if db.execute(
            "DELETE FROM uploaded_attachments WHERE scope=?1 AND id=?2",
            params![serde_json::to_string(scope)?, id],
        )? == 0
        {
            return Err(reject(ErrorKind::NotFound, "Attachment not found in this scope"));
        }
        Ok(())
    })
}
