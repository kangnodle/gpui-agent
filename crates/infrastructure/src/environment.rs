use super::*;
use gpui_agent_domain::{EnvironmentDescriptor,
                        EnvironmentPlatform,
                        EnvironmentPreferences,
                        EnvironmentPreferencesPatch};

pub(super) fn initialize(db: &Connection) -> ApplicationResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS environment(singleton INTEGER PRIMARY KEY CHECK(singleton=1),identity TEXT NOT NULL,preferences TEXT NOT NULL);",
    )
    .app()?;
    let exists: bool = db
        .query_row("SELECT EXISTS(SELECT 1 FROM environment WHERE singleton=1)", [], |row| row.get(0))
        .app()?;
    if !exists {
        let identity = new_token().map_err(|_| failure("Cannot create environment identity"))?;
        let preferences = serde_json::to_string(&EnvironmentPreferences::default()).map_err(|_| failure("Cannot encode environment preferences"))?;
        db.execute(
            "INSERT OR IGNORE INTO environment(singleton,identity,preferences) VALUES(1,?1,?2)",
            params![identity, preferences],
        )
        .app()?;
    }
    Ok(())
}

pub(super) fn read(db: &Connection) -> ApplicationResult<EnvironmentDescriptor> {
    let (environment_id, encoded): (String, String) = db
        .query_row("SELECT identity,preferences FROM environment WHERE singleton=1", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .app()?;
    let preferences: EnvironmentPreferences = serde_json::from_str(&encoded).map_err(|_| failure("Stored environment preferences are invalid"))?;
    preferences.validate().map_err(|_| failure("Stored environment preferences are invalid"))?;
    Ok(EnvironmentDescriptor {
        environment_id,
        server_version: env!("CARGO_PKG_VERSION").into(),
        platform: EnvironmentPlatform {
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        },
        preferences,
    })
}

impl Projects {
    pub(super) fn patch_environment(&self, patch: EnvironmentPreferencesPatch) -> ApplicationResult<EnvironmentPreferences> {
        let mut state = self.state.lock().map_err(|_| failure("Project database unavailable"))?;
        let tx = state.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).app()?;
        let preferences = read(&tx)?.preferences.patched(patch).map_err(invalid)?;
        let encoded = serde_json::to_string(&preferences).map_err(|_| failure("Cannot encode environment preferences"))?;
        tx.execute("UPDATE environment SET preferences=?1 WHERE singleton=1", [encoded]).app()?;
        tx.commit().app()?;
        Ok(preferences)
    }
}
