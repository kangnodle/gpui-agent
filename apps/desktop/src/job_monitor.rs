//! Read-only view of the host's durable jobs. The sidebar, history, account
//! locks and computer-use approvals use it; it never submits or edits jobs.
use crate::{client::ApiClient,
            language::Language,
            ui::Wake};
use gpui_agent_contracts::*;
use gpui_agent_presentation::JobActivity;
use reqwest::Method;
use std::{collections::HashSet,
          sync::mpsc::{self,
                       Receiver,
                       SyncSender},
          time::{Duration,
                 Instant}};

enum Change {
    Catalog(Vec<Job>, HashSet<i64>),
    Error,
}

pub(super) struct JobMonitor {
    pub activity: JobActivity,
    pub(super) jobs: Vec<Job>,
    refreshing: bool,
    last_refresh: Option<Instant>,
    tx: SyncSender<Change>,
    rx: Receiver<Change>,
}
impl Default for JobMonitor {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(16);
        Self {
            activity: Default::default(),
            jobs: Vec::new(),
            refreshing: false,
            last_refresh: None,
            tx,
            rx,
        }
    }
}
impl JobMonitor {
    pub fn workspace_status(&self, workspace: i64, language: Language) -> String {
        let jobs: Vec<_> = self.jobs.iter().filter(|job| job.scope.workspace_id == Some(workspace)).collect();
        let mut labels = Vec::new();
        for (state, label) in [(JobState::Running, "실행 중"), (JobState::Queued, "대기 중"), (JobState::Waiting, "승인 요청")] {
            let count = jobs.iter().filter(|job| job.state == state).count();
            if count > 0 {
                labels.push(format!("{} {count}", language.text(label)));
            }
        }
        let unread = jobs.iter().filter(|job| self.activity.unread(job.id)).count();
        if unread > 0 {
            labels.push(format!("{} {unread}", language.text("새 알림")));
        }
        labels.join(" · ")
    }

    pub fn active_count(&self) -> usize { self.jobs.iter().filter(|job| !job.state.terminal()).count() }

    pub fn waiting_count(&self) -> usize { self.jobs.iter().filter(|job| job.state == JobState::Waiting).count() }

    pub fn profile_active(&self, provider: Provider, account: &str) -> bool {
        self.jobs
            .iter()
            .any(|job| job.scope.provider == provider && job.scope.account_id == account && matches!(job.state, JobState::Running | JobState::Waiting))
    }

    fn refresh(&mut self, ctx: &Wake, client: &ApiClient) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.last_refresh = Some(Instant::now());
        let client = client.clone();
        let known = self.jobs.iter().map(|job| job.id).collect();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let change = match client.response(Method::GET, "/v1/jobs", None).and_then(|response| Ok(response.json()?)) {
                | Ok(jobs) => Change::Catalog(jobs, known),
                | Err(_) => Change::Error,
            };
            let _ = tx.send(change);
            ctx.notify();
        });
    }

    fn updates(&mut self) -> bool {
        let mut changed = false;
        while let Ok(change) = self.rx.try_recv() {
            changed = true;
            self.refreshing = false;
            if let Change::Catalog(mut jobs, known) = change {
                // A job created after the request started stays until the next
                // refresh lists it.
                let present: HashSet<_> = jobs.iter().map(|job| job.id).collect();
                jobs.extend(self.jobs.iter().filter(|job| !known.contains(&job.id) && !present.contains(&job.id)).cloned());
                self.activity.catalog(&jobs);
                self.jobs = jobs;
            }
        }
        changed
    }

    /// Refreshes every few seconds while the host runs background jobs.
    pub fn poll(&mut self, ctx: &Wake, client: &ApiClient, enabled: bool) -> bool {
        let mut changed = self.updates();
        if enabled && self.last_refresh.is_none_or(|last| last.elapsed() > Duration::from_secs(5)) {
            self.refresh(ctx, client);
            changed = true;
        }
        changed
    }
}
