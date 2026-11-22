use crate::{app::ChatApp,
            client::ApiClient,
            ui::*};
use gpui::{self,
           assets::IconName,
           base::TestSupportExt as _,
           *};
use gpui_agent_contracts::*;
use gpui_agent_presentation::{Action,
                              Conversation,
                              HistoryGroup,
                              history_entries};
use std::{collections::HashMap,
          sync::mpsc::{self,
                       Receiver,
                       SyncSender}};

#[derive(Default)]
pub(super) struct ProjectRecords {
    pub groups: Vec<HistoryGroup>,
    pub loading: bool,
    pub error: Option<String>,
}

enum Change {
    Jobs(u64, Vec<Job>),
    Project(u64, i64, Vec<HistoryGroup>, Option<String>),
}

#[derive(Clone)]
pub(super) struct OpenHistory {
    pub workspace: i64,
    pub provider: usize,
    pub account: String,
    pub title: String,
    pub source: HistorySource,
}

/// One owned row of the virtualized sidebar.
pub(super) enum SidebarRow {
    EmptyProjects,
    /// Single-folder fallback before the project catalog has loaded.
    Legacy,
    Workspace {
        project: usize,
        workspace: usize,
        position: usize,
        status: String,
    },
    Preamble {
        workspace: i64,
        loading: bool,
        error: Option<String>,
        empty: bool,
        records: bool,
    },
    Entry(Box<SidebarEntry>),
}

pub(super) struct SidebarEntry {
    pub workspace: i64,
    pub provider: Provider,
    pub account: String,
    pub conversation: Option<ConversationSummary>,
    pub job: Option<Job>,
}

impl SidebarRow {
    /// Hash of everything that changes the row's height.
    pub(super) fn signature(&self, drafts: usize) -> u64 {
        use std::hash::{Hash as _,
                        Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        match self {
            | Self::EmptyProjects | Self::Legacy => 0u8.hash(&mut hasher),
            | Self::Workspace {
                position,
                status,
                ..
            } => (*position == 0, status.is_empty()).hash(&mut hasher),
            | Self::Preamble {
                loading,
                error,
                empty,
                records,
                ..
            } => (*loading, error.as_ref().map(String::len), *empty, *records, drafts).hash(&mut hasher),
            | Self::Entry(entry) => (
                entry.job.is_some() || entry.conversation.as_ref().is_some_and(|c| c.execution_state.is_some()),
                entry.job.as_ref().map(|job| std::mem::discriminant(&job.state)),
            )
                .hash(&mut hasher),
        }
        hasher.finish()
    }
}

#[derive(Clone, Copy)]
pub(super) enum HistorySource {
    Conversation(i64, Option<i64>),
    Job(i64),
}

pub(super) struct HistoryUi {
    pub projects: HashMap<i64, ProjectRecords>,
    pub jobs: Vec<Job>,
    pub query: String,
    pub pending: Option<OpenHistory>,
    pub dirty: bool,
    pub accounts: HashMap<(i64, usize, String), Conversation>,
    generation: u64,
    tx: SyncSender<Change>,
    rx: Receiver<Change>,
}

impl Default for HistoryUi {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(512);
        Self {
            projects: HashMap::new(),
            jobs: Vec::new(),
            query: String::new(),
            pending: None,
            dirty: false,
            accounts: HashMap::new(),
            generation: 0,
            tx,
            rx,
        }
    }
}

impl HistoryUi {
    pub fn refresh(&mut self, client: &ApiClient, catalog: &ProjectCatalog, query: String, wake: &Wake) {
        let workspaces: Vec<_> = catalog
            .projects
            .iter()
            .flat_map(|p| &p.workspaces)
            .filter(|w| !w.missing)
            .map(|w| w.id)
            .collect();
        self.projects.retain(|id, _| workspaces.contains(id));
        if workspaces.is_empty() {
            return;
        }
        for id in &workspaces {
            let records = self.projects.entry(*id).or_default();
            if self.query != query {
                records.groups.clear();
            }
            records.loading = true;
            records.error = None;
        }
        self.query = query.clone();
        self.generation += 1;
        let generation = self.generation;
        let client = client.clone();
        let tx = self.tx.clone();
        let wake = wake.clone();
        std::thread::spawn(move || {
            let jobs = client
                .response(reqwest::Method::GET, "/v1/jobs", None)
                .and_then(|response| Ok(response.json::<Vec<Job>>()?));
            let mut errors = Vec::new();
            let mut accounts = Vec::new();
            for provider in PROVIDERS {
                let mut ids = vec!["default".to_owned()];
                if jobs.is_ok() {
                    match client
                        .response(reqwest::Method::GET, &format!("/v1/providers/{}/accounts", provider_name(provider)), None)
                        .and_then(|response| Ok(response.json::<Vec<AccountProfile>>()?))
                    {
                        | Ok(profiles) => ids.extend(profiles.into_iter().map(|p| p.id).filter(|id| id != "default")),
                        | Err(error) => errors.push(format!("{}: {error}", display_name(provider))),
                    }
                }
                accounts.extend(ids.into_iter().map(|id| (provider, id)));
            }
            match jobs {
                | Ok(jobs) => {
                    let _ = tx.send(Change::Jobs(generation, jobs));
                },
                | Err(error) if error.to_string().contains("Background jobs are unavailable") => {},
                | Err(error) => errors.push(error.to_string()),
            }
            for workspace in workspaces {
                let mut groups = Vec::new();
                let mut errors = errors.clone();
                for (provider, account) in &accounts {
                    let scoped = client.for_account(*provider, account.clone(), Some(workspace));
                    let result = (|| -> anyhow::Result<_> {
                        let mut url: reqwest::Url = format!("{}/v1/providers/{}/conversations", client.origin(), provider_name(*provider)).parse()?;
                        url.query_pairs_mut().append_pair("q", &query).append_pair("archived", "all");
                        Ok(scoped
                            .response(reqwest::Method::GET, &format!("{}?{}", url.path(), url.query().unwrap_or_default()), None)?
                            .json()?)
                    })();
                    match result {
                        | Ok(conversations) => groups.push(HistoryGroup {
                            provider: *provider,
                            account_id: account.clone(),
                            conversations,
                        }),
                        | Err(error) => errors.push(format!("{} / {account}: {error}", display_name(*provider))),
                    }
                }
                if tx
                    .send(Change::Project(generation, workspace, groups, (!errors.is_empty()).then(|| errors.join("\n"))))
                    .is_err()
                {
                    return;
                }
                wake.notify();
            }
        });
    }

    pub fn updates(&mut self) -> bool {
        let mut changed = false;
        while let Ok(change) = self.rx.try_recv() {
            changed = true;
            match change {
                | Change::Jobs(generation, jobs) if generation == self.generation => self.jobs = jobs,
                | Change::Project(generation, id, groups, error) if generation == self.generation => {
                    self.projects.insert(
                        id,
                        ProjectRecords {
                            groups,
                            loading: false,
                            error,
                        },
                    );
                },
                | _ => {},
            }
        }
        changed
    }
}

impl ChatApp {
    pub(super) fn refresh_project_history(&mut self) {
        self.history.refresh(
            &self.client,
            &self.workspaces.catalog,
            self.conversations[self.selected].search.clone(),
            &self.wake,
        );
    }

    pub(super) fn open_history(&mut self, entry: OpenHistory) {
        if self.busy {
            return;
        }
        if self.workspaces.selected != Some(entry.workspace) {
            let wake = self.wake.clone();
            self.select_workspace(Some(entry.workspace), &wake);
            self.history.pending = Some(entry);
            return;
        }
        let provider = entry.provider;
        let previous_account = self.conversations[provider].account_id.clone().unwrap_or_else(|| "default".into());
        if previous_account != entry.account {
            let key = (entry.workspace, provider, entry.account.clone());
            let mut next = self.history.accounts.remove(&key).unwrap_or_default();
            next.account_id = Some(entry.account.clone());
            let previous = std::mem::replace(&mut self.conversations[provider], next);
            self.history.accounts.insert((entry.workspace, provider, previous_account), previous);
        }
        self.selected = provider;
        self.client = self.client.for_account(PROVIDERS[provider], entry.account, Some(entry.workspace));
        self.close_panel();
        self.conversations[provider].title = Some(entry.title);
        self.command(match entry.source {
            | HistorySource::Conversation(id, message_id) => Action::Load {
                id,
                message_id,
            },
            | HistorySource::Job(id) => Action::OpenJob(id),
        });
    }

    /// Jobs known to the history plus the live job list; a live job replaces a
    /// stored one unless the stored copy is newer.
    pub(super) fn merged_history_jobs(&self) -> Vec<Job> {
        let mut by_id: HashMap<i64, Job> = self.history.jobs.iter().map(|job| (job.id, job.clone())).collect();
        for job in &self.job_monitor.jobs {
            if by_id.get(&job.id).is_some_and(|old| old.last_sequence > job.last_sequence) {
                continue;
            }
            by_id.insert(job.id, job.clone());
        }
        by_id.into_values().collect()
    }

    /// Flat, owned rows for one workspace's history: one preamble row (drafts,
    /// loading, errors, empty notice) followed by one row per entry, so the
    /// sidebar can build only the visible ones.
    pub(super) fn history_rows(&self, workspace: i64, jobs: &[Job], rows: &mut Vec<SidebarRow>) {
        let Some(records) = self.history.projects.get(&workspace) else {
            rows.push(SidebarRow::Preamble {
                workspace,
                loading: true,
                error: None,
                empty: false,
                records: false,
            });
            return;
        };
        let entries = history_entries(
            &records.groups,
            jobs,
            workspace,
            self.conversations[self.selected].show_archived,
            &self.history.query,
        );
        rows.push(SidebarRow::Preamble {
            workspace,
            loading: records.loading,
            error: records.error.clone(),
            empty: entries.is_empty() && !records.loading && records.error.is_none(),
            records: true,
        });
        rows.extend(entries.into_iter().map(|entry| {
            SidebarRow::Entry(Box::new(SidebarEntry {
                workspace,
                provider: entry.provider,
                account: entry.account_id.to_owned(),
                conversation: entry.conversation.cloned(),
                job: entry.job.cloned(),
            }))
        }));
    }

    pub(super) fn history_preamble_row(
        &self,
        workspace: i64,
        loading: bool,
        failure: Option<&str>,
        empty: bool,
        records: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        if !records {
            return column()
                .child(
                    note(self.text("기록을 불러오는 중…"), cx)
                        .id(format!("history-loading-{workspace}"))
                        .test_support(),
                )
                .into_any_element();
        }
        let mut body = if self.workspaces.selected == Some(workspace) {
            self.history_drafts(cx)
        } else {
            column().gap_1().pl_2()
        };
        if loading {
            body = body.child(
                note(self.text("기록을 불러오는 중…"), cx)
                    .id(format!("history-loading-{workspace}"))
                    .test_support(),
            );
        }
        if let Some(message) = failure {
            body = body.child(
                column()
                    .id(format!("history-error-{workspace}"))
                    .test_support()
                    .child(error(format!("{}\n{message}", self.text("기록을 불러오지 못했습니다")), cx))
                    .child(self.button(format!("history-retry-{workspace}"), "다시 시도", !loading, cx, |s, _, _| {
                        s.refresh_project_history()
                    })),
            );
        }
        if empty {
            body = body.child(
                note(
                    self.text(if self.history.query.is_empty() {
                        "저장된 대화가 없습니다"
                    } else {
                        "검색 결과가 없습니다"
                    }),
                    cx,
                )
                .id(format!("history-empty-{workspace}"))
                .test_support()
                .px_2()
                .py_2(),
            );
        }
        body.into_any_element()
    }

    pub(super) fn history_entry_row(&self, entry: &SidebarEntry, cx: &Context<Self>) -> AnyElement {
        let workspace = entry.workspace;
        let provider = provider_index(entry.provider);
        let (conversation, job) = (entry.conversation.as_ref(), entry.job.as_ref());
        let account_id = entry.account.as_str();
        let title = conversation
            .map(|c| c.title.as_str())
            .or_else(|| job.map(|j| j.preview.as_str()))
            .unwrap_or_default();
        let source = job
            .filter(|job| !job.state.terminal())
            .map(|job| HistorySource::Job(job.id))
            .or_else(|| conversation.map(|c| HistorySource::Conversation(c.id, c.matched_message.as_ref().map(|m| m.message_id))))
            .unwrap_or_else(|| HistorySource::Job(job.expect("history entry source").id));
        let key = conversation
            .map(|c| format!("chat-{}", c.id))
            .unwrap_or_else(|| format!("job-{}", job.expect("history entry source").id));
        let open = OpenHistory {
            workspace,
            provider,
            account: account_id.into(),
            title: title.into(),
            source,
        };
        let active = self.workspaces.selected == Some(workspace)
            && self.selected == provider
            && self.conversations[provider].account_id.as_deref().unwrap_or("default") == account_id
            && (conversation.is_some_and(|c| Some(c.id) == self.conversations[provider].id)
                || job.is_some_and(|job| {
                    self.background_job == Some(job.id) || job.conversation_id.is_some() && job.conversation_id == self.conversations[provider].id
                }));
        let mut line = row().flex_nowrap().gap_0().child(
            self.sidebar_button(
                format!("project-history-{workspace}-{}-{}-{key}", provider_name(entry.provider), account_id),
                title,
                IconName::MessageSquare,
                !self.busy,
                cx,
                move |s, window, _| {
                    s.open_history(open.clone());
                    if window.viewport_size().width < px(720.) {
                        s.sidebar_open = false;
                    }
                },
            )
            .selected(active)
            .flex_1()
            .min_w_0(),
        );
        if self.workspaces.selected == Some(workspace)
            && self.conversations[provider].account_id.as_deref().unwrap_or("default") == account_id
            && let Some(conversation) = conversation
        {
            line = line.child(self.history_menu(provider, conversation, cx));
        }
        let mut item = column().gap_0().child(line);
        if let Some(state) = job.map(|job| job.state).or_else(|| conversation.and_then(|c| c.execution_state)) {
            item = item.child(note(self.text(gpui_agent_presentation::state_label(state)), cx).pl_2());
        }
        item.into_any_element()
    }
}
