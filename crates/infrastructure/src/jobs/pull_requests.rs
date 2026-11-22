use super::*;
use gpui_agent_domain::{PullRequestData,
                        PullRequestGuide,
                        PullRequestLink,
                        PullRequestSnapshot,
                        PullRequestState,
                        PullRequestTarget,
                        PullRequestWatch};

pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS pull_request_links (
        id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL, conversation INTEGER NOT NULL,
        repository TEXT NOT NULL, number INTEGER NOT NULL, snapshot TEXT, watch TEXT, next_poll INTEGER,
        revision INTEGER NOT NULL DEFAULT 0, error TEXT, last_job INTEGER,
        UNIQUE(scope,conversation,repository,number));
        CREATE INDEX IF NOT EXISTS pr_link_conversation ON pull_request_links(conversation);
        CREATE INDEX IF NOT EXISTS pr_link_due ON pull_request_links(next_poll,id) WHERE watch IS NOT NULL;",
    )?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS pull_request_guides (
        id INTEGER PRIMARY KEY AUTOINCREMENT,scope TEXT NOT NULL,repository TEXT NOT NULL,number INTEGER NOT NULL,
        head TEXT NOT NULL,base TEXT NOT NULL,job INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE);
        CREATE INDEX IF NOT EXISTS pr_guide_target ON pull_request_guides(scope,repository,number,id);",
    )?;
    Ok(())
}

impl JobStore {
    pub(super) fn pr_guide(&self, scope: &JobScope, target: &PullRequestTarget) -> ApplicationResult<Option<PullRequestGuide>> {
        self.db(|db| {
            let row = db
                .query_row(
                    "SELECT head,base,job FROM pull_request_guides WHERE scope=?1 AND repository=?2 AND number=?3 ORDER BY id DESC LIMIT 1",
                    params![serde_json::to_string(scope)?, target.repository, target.number],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
                )
                .optional()?;
            row.map(|(head, base, id)| guide(db, head, base, id)).transpose()
        })
    }

    pub(super) fn pr_start_guide(
        &self,
        scope: &JobScope,
        target: &PullRequestTarget,
        head: &str,
        base: &str,
        request: &ChatRequest,
    ) -> ApplicationResult<(PullRequestGuide, bool)> {
        target.validate()?;
        request.validate()?;
        if [head, base].iter().any(|sha| sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()))
            || request.conversation_id.is_some()
            || request.session_id.is_some()
            || request.claude_permission_mode.is_some()
        {
            return Err("Guide jobs require an independent session and valid commit SHAs".into());
        }
        self.db(|db| {
            let tx = db.transaction()?;
            let scope_json = serde_json::to_string(scope)?;
            let existing: Option<i64> = tx.query_row("SELECT g.job FROM pull_request_guides g JOIN jobs j ON j.id=g.job WHERE g.scope=?1 AND g.repository=?2 AND g.number=?3 AND g.head=?4 AND g.base=?5 AND j.state IN ('queued','running','waiting','completed') ORDER BY g.id DESC LIMIT 1",params![scope_json,target.repository,target.number,head,base],|r|r.get(0)).optional()?;
            if let Some(id) = existing { return Ok((guide(&tx,head.into(),base.into(),id)?,false)); }
            let job = enqueue(&tx,scope,request)?;
            tx.execute("INSERT INTO pull_request_guides(scope,repository,number,head,base,job) VALUES(?1,?2,?3,?4,?5,?6)",params![scope_json,target.repository,target.number,head,base,job.id])?;
            tx.commit()?;
            Ok((PullRequestGuide { head_sha:head.into(),base_sha:base.into(),job,text:None },true))
        })
    }

    pub(super) fn pr_due(&self, now: i64) -> ApplicationResult<Vec<PullRequestLink>> {
        self.db(|db| db.prepare("SELECT id,scope,conversation,repository,number,snapshot,watch,next_poll,revision,error,last_job FROM pull_request_links WHERE watch IS NOT NULL AND next_poll<=?1 AND json_extract(scope,'$.provider') IN (SELECT name FROM supported_providers) ORDER BY next_poll,id LIMIT 16")?.query_map([now],link)?.collect::<rusqlite::Result<_>>().map_err(Into::into))
    }

    pub(super) fn pr_stop(&self, scope: &JobScope, conversation: i64) -> ApplicationResult<()> { self.db(|db| stop(db, scope, conversation)) }

    pub(super) fn pr_apply(&self, previous: &PullRequestLink, next: &PullRequestLink, notification: Option<&ChatRequest>) -> ApplicationResult<Option<Job>> {
        if let Some(request) = notification {
            request.validate()?;
        }
        self.db(|db| {
            let tx = db.transaction()?;
            let changed = tx.execute("UPDATE pull_request_links SET snapshot=?3,watch=?4,next_poll=?5,revision=revision+1,error=?6 WHERE id=?1 AND revision=?2 AND watch IS NOT NULL",params![previous.id,previous.revision,next.snapshot.as_ref().map(serde_json::to_string).transpose()?,next.watch.as_ref().map(serde_json::to_string).transpose()?,next.next_poll,next.error])?;
            if changed == 0 { return Ok(None); }
            let job = notification.map(|request|enqueue(&tx,&previous.scope,request)).transpose()?;
            if let Some(job) = &job { tx.execute("UPDATE pull_request_links SET last_job=?2 WHERE id=?1",params![previous.id,job.id])?; }
            tx.commit()?; Ok(job)
        })
    }

    pub(super) fn pr_links(&self, scope: &JobScope, conversation: i64) -> ApplicationResult<Vec<PullRequestLink>> {
        self.db(|db| {
            db.prepare("SELECT id,scope,conversation,repository,number,snapshot,watch,next_poll,revision,error,last_job FROM pull_request_links WHERE scope=?1 AND conversation=?2 ORDER BY id")?
                .query_map(params![serde_json::to_string(scope)?, conversation], link)?.collect::<rusqlite::Result<_>>().map_err(Into::into)
        })
    }

    pub(super) fn pr_set_link(&self, scope: &JobScope, conversation: i64, target: &PullRequestTarget, linked: bool) -> ApplicationResult<bool> {
        target.validate()?;
        self.db(|db| {
            let tx = db.transaction()?;
            let scope = serde_json::to_string(scope)?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pull_request_links WHERE scope=?1 AND conversation=?2 AND repository=?3 AND number=?4)",
                params![scope, conversation, target.repository, target.number],
                |r| r.get(0),
            )?;
            if linked && !exists {
                let count: i64 = tx.query_row(
                    "SELECT count(*) FROM pull_request_links WHERE scope=?1 AND conversation=?2",
                    params![scope, conversation],
                    |r| r.get(0),
                )?;
                if count >= 32 {
                    return Err(reject(ErrorKind::Busy, "A conversation can link at most 32 PRs"));
                }
                tx.execute(
                    "INSERT INTO pull_request_links(scope,conversation,repository,number) VALUES(?1,?2,?3,?4)",
                    params![scope, conversation, target.repository, target.number],
                )?;
            } else if !linked {
                tx.execute(
                    "DELETE FROM pull_request_links WHERE scope=?1 AND conversation=?2 AND repository=?3 AND number=?4",
                    params![scope, conversation, target.repository, target.number],
                )?;
            }
            tx.commit()?;
            Ok(exists)
        })
    }

    pub(super) fn pr_set_watch(
        &self,
        scope: &JobScope,
        conversation: i64,
        target: &PullRequestTarget,
        baseline: Option<&PullRequestData>,
        now: i64,
    ) -> ApplicationResult<bool> {
        target.validate()?;
        if baseline.is_some_and(|data| data.snapshot.state != PullRequestState::Open || data.viewer.is_none()) {
            return Err("Watching requires an open PR and authenticated viewer".into());
        }
        self.db(|db| {
            let tx = db.transaction()?;
            let scope = serde_json::to_string(scope)?;
            let previous: Option<Option<String>> = tx.query_row("SELECT watch FROM pull_request_links WHERE scope=?1 AND conversation=?2 AND repository=?3 AND number=?4",params![scope,conversation,target.repository,target.number],|row|row.get(0)).optional()?;
            let was_watching = previous.as_ref().is_some_and(Option::is_some);
            if baseline.is_some() && was_watching { return Ok(true); }
            if previous.is_none() && baseline.is_some() {
                let count: i64 = tx.query_row("SELECT count(*) FROM pull_request_links WHERE scope=?1 AND conversation=?2",params![scope,conversation],|r|r.get(0))?;
                if count >= 32 { return Err(reject(ErrorKind::Busy,"A conversation can link at most 32 PRs")); }
                tx.execute("INSERT INTO pull_request_links(scope,conversation,repository,number) VALUES(?1,?2,?3,?4)",params![scope,conversation,target.repository,target.number])?;
            }
            if let Some(data) = baseline {
                let active: i64 = tx.query_row("SELECT count(*) FROM pull_request_links WHERE watch IS NOT NULL",[],|r|r.get(0))?;
                if active >= 128 { return Err(reject(ErrorKind::Busy,"At most 128 PR watches can be active")); }
                let watch = PullRequestWatch { viewer: data.viewer.clone().unwrap_or_default(),failures:0,comment_wakes:0 };
                tx.execute("UPDATE pull_request_links SET snapshot=?5,watch=?6,next_poll=?7,revision=revision+1,error=NULL WHERE scope=?1 AND conversation=?2 AND repository=?3 AND number=?4",params![scope,conversation,target.repository,target.number,serde_json::to_string(&data.snapshot)?,serde_json::to_string(&watch)?,now.saturating_add(120)])?;
            } else {
                tx.execute("UPDATE pull_request_links SET watch=NULL,next_poll=NULL,revision=revision+1 WHERE scope=?1 AND conversation=?2 AND repository=?3 AND number=?4",params![scope,conversation,target.repository,target.number])?;
            }
            tx.commit()?;
            Ok(was_watching)
        })
    }
}

pub(super) fn stop(db: &Connection, scope: &JobScope, conversation: i64) -> Result<()> {
    db.execute(
        "UPDATE pull_request_links SET watch=NULL,next_poll=NULL,revision=revision+1 WHERE scope=?1 AND conversation=?2 AND watch IS NOT NULL",
        params![serde_json::to_string(scope)?, conversation],
    )?;
    Ok(())
}

fn guide(db: &Connection, head_sha: String, base_sha: String, id: i64) -> Result<PullRequestGuide> {
    let job = load_job(db, id)?;
    let text = if job.state == JobState::Completed {
        let text: Option<String> = db
            .query_row(
                "SELECT json_extract(event,'$.text') FROM job_events WHERE job=?1 AND json_extract(event,'$.type')='completed' ORDER BY sequence DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if text.as_ref().is_some_and(|text| text.len() > 256 * 1024) {
            return Err(reject(ErrorKind::Invalid, "Guide output exceeds 256 KiB; inspect its job events"));
        }
        text
    } else {
        None
    };
    Ok(PullRequestGuide {
        head_sha,
        base_sha,
        job,
        text,
    })
}

fn link(row: &rusqlite::Row<'_>) -> rusqlite::Result<PullRequestLink> {
    let snapshot: Option<String> = row.get(5)?;
    let watch: Option<String> = row.get(6)?;
    Ok(PullRequestLink {
        id: row.get(0)?,
        scope: serde_json::from_str(&row.get::<_, String>(1)?).map_err(json_error)?,
        conversation_id: row.get(2)?,
        target: PullRequestTarget {
            repository: row.get(3)?,
            number: row.get(4)?,
        },
        snapshot: snapshot
            .map(|v| serde_json::from_str::<PullRequestSnapshot>(&v))
            .transpose()
            .map_err(json_error)?,
        watch: watch.map(|v| serde_json::from_str::<PullRequestWatch>(&v)).transpose().map_err(json_error)?,
        next_poll: row.get(7)?,
        revision: row.get(8)?,
        error: row.get(9)?,
        last_notification_job: row.get(10)?,
    })
}
