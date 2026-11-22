use crate::{client::ApiClient,
            language::Language,
            ui::Wake};
use gpui_agent_contracts::{ChatRequest,
                           Event,
                           PROVIDERS,
                           ProviderInfo,
                           SavedConversation,
                           StreamEvent};
use gpui_agent_presentation::{Action,
                              Conversation,
                              ScrollTarget,
                              Update};
use std::{path::PathBuf,
          sync::mpsc::{Receiver,
                       SyncSender}};
pub(super) enum HistoryDialog {
    Rename { provider: usize, id: i64, title: String },
    Delete { provider: usize, id: i64, title: String },
}

pub(super) struct ReferenceConversation {
    pub(super) provider: usize,
    pub(super) title: String,
    pub(super) conversation: SavedConversation,
}

pub struct ChatApp {
    pub(super) plugins: crate::plugins::PluginsUi,
    pub(super) guidance_editor: gpui_agent_presentation::GuidanceEditor,
    pub(super) guidance_preview: bool,
    pub(super) history: crate::history::HistoryUi,
    pub(super) accounts: crate::accounts::AccountsUi,
    pub(super) computer: crate::computer::ComputerUi,
    pub(super) maintenance: crate::maintenance::MaintenanceUi,
    pub(super) mcp: crate::mcp::McpUi,
    pub(super) job_monitor: crate::job_monitor::JobMonitor,
    pub(super) attachment_ui: crate::content::AttachUi,
    pub(super) workspaces: crate::workspace::WorkspaceUi,
    pub(super) client: ApiClient,
    pub(super) tx: SyncSender<(usize, Update)>,
    pub(super) rx: Receiver<(usize, Update)>,
    pub(super) selected: usize,
    pub(super) conversations: [Conversation; gpui_agent_contracts::PROVIDERS.len()],
    pub(super) providers: Vec<ProviderInfo>,
    pub(super) busy: bool,
    pub(super) operation: Option<u64>,
    pub(super) background_job: Option<i64>,
    pub(super) last_computer_remaining: u64,
    pub(super) activity: &'static str,
    pub(super) cwd: String,
    pub(super) dialog: Option<HistoryDialog>,
    pub(super) modal_focus: Option<gpui_kit::FocusHandle>,
    pub(super) sidebar_open: bool,
    pub(super) sidebar_width: f32,
    pub(super) sidebar_resizing: bool,
    pub(super) search_changed_at: Option<std::time::Instant>,
    pub(super) search_task: Option<gpui_kit::Task<()>>,
    pub(super) settings_nav_open: bool,
    pub(super) settings_search: String,
    pub(super) settings_result: usize,
    pub(super) settings_scroll: gpui_kit::ScrollHandle,
    pub(super) right_panels: gpui_agent_presentation::RightPanels,
    pub(super) work_maximized: bool,
    pub(super) reference_resizing: bool,
    pub(super) tool_resizing: bool,
    pub(super) panel_split_resizing: bool,
    pub(super) reference: Option<ReferenceConversation>,
    pub(super) language: Language,
    pub(super) language_path: Option<PathBuf>,
    pub(super) titlebar_mode: crate::titlebar::TitlebarMode,
    pub(super) wake: Wake,
    pub(super) images: std::cell::RefCell<std::collections::HashMap<String, crate::content::CachedImage>>,
    pub(super) used_images: std::cell::RefCell<std::collections::HashSet<String>>,
    pub(super) used_fields: std::collections::HashSet<String>,
    pub(super) expanded_tools: std::collections::HashSet<String>,
    pub(super) inputs: std::collections::HashMap<String, crate::ui::Field>,
    pub(super) panel: Option<crate::ui::Panel>,
    pub(super) chat_tool: Option<crate::ui::Panel>,
    pub(super) chat_tool_scroll: gpui_kit::ScrollHandle,
    pub(super) work_scroll: gpui_kit::ScrollHandle,
    pub(super) work_list: std::cell::RefCell<crate::messages::MessageList>,
    pub(super) sidebar_rows: std::cell::RefCell<std::rc::Rc<Vec<crate::history::SidebarRow>>>,
    pub(super) sidebar_list: std::cell::RefCell<crate::messages::MessageList>,
    pub(super) timeline_facts: std::cell::RefCell<Option<(crate::panels::TimelineStamp, crate::panels::TimelineFacts)>>,
    pub(super) work_cache: std::cell::RefCell<Option<crate::views::WorkCache>>,
    pub(super) message_lists: std::cell::RefCell<[crate::messages::MessageList; 2]>,
    pub(super) panel_scroll: gpui_kit::ScrollHandle,
    pub(super) focus: gpui_kit::FocusHandle,
}

impl ChatApp {
    pub fn new(client: ApiClient, cwd: String, language_path: Option<PathBuf>, ctx: &Wake, focus: gpui_kit::FocusHandle) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(512);
        let sidebar_width = language_path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path.with_file_name("ui-sidebar-width")).ok())
            .and_then(|value| value.trim().parse::<f32>().ok())
            .filter(|width| width.is_finite() && *width >= 208.)
            .unwrap_or(256.);
        let mut app = Self {
            plugins: Default::default(),
            guidance_editor: Default::default(),
            guidance_preview: false,
            history: Default::default(),
            wake: ctx.clone(),
            images: Default::default(),
            used_images: Default::default(),
            used_fields: Default::default(),
            expanded_tools: Default::default(),
            inputs: Default::default(),
            panel: None,
            chat_tool: None,
            chat_tool_scroll: Default::default(),
            work_scroll: Default::default(),
            work_list: Default::default(),
            sidebar_rows: Default::default(),
            sidebar_list: Default::default(),
            timeline_facts: Default::default(),
            work_cache: Default::default(),
            message_lists: Default::default(),
            panel_scroll: Default::default(),
            focus,
            client,
            accounts: Default::default(),
            computer: Default::default(),
            maintenance: Default::default(),
            mcp: Default::default(),
            job_monitor: Default::default(),
            attachment_ui: Default::default(),
            workspaces: Default::default(),
            tx,
            rx,
            selected: 0,
            conversations: std::array::from_fn(|_| Conversation::default()),
            providers: Vec::new(),
            busy: false,
            operation: None,
            background_job: None,
            last_computer_remaining: 0,
            activity: "",
            cwd,
            dialog: None,
            modal_focus: None,
            sidebar_open: true,
            sidebar_width,
            sidebar_resizing: false,
            search_changed_at: None,
            search_task: None,
            settings_nav_open: false,
            settings_search: String::new(),
            settings_result: 0,
            settings_scroll: Default::default(),
            right_panels: crate::panels::load(language_path.as_deref()),
            work_maximized: false,
            reference_resizing: false,
            tool_resizing: false,
            panel_split_resizing: false,
            reference: None,
            language: language_path.as_deref().map(crate::language::load).unwrap_or_default(),
            titlebar_mode: language_path
                .as_ref()
                .map(|path| crate::titlebar::TitlebarMode::load(&path.with_file_name("ui-titlebar")))
                .unwrap_or_default(),
            language_path,
        };
        app.start(Action::Refresh, ctx);
        app
    }

    pub(super) fn start(&mut self, action: Action, ctx: &Wake) { self.start_for(self.selected, action, ctx); }

    pub(super) fn start_for(&mut self, provider: usize, action: Action, ctx: &Wake) {
        if matches!(
            action,
            Action::Refresh
                | Action::GuidanceFiles
                | Action::Guidance
                | Action::SetSkillEnabled { .. }
                | Action::WriteGuidance(_)
                | Action::DecideSkillProposal { .. }
                | Action::DecideMcpProposal { .. }
        ) {
            let key = gpui_agent_presentation::guidance_scope(self.client.workspace_id(), self.conversations[provider].account_id.as_deref());
            self.conversations[provider].guidance.begin(key);
        }
        if let Action::Runtime {
            provider, ..
        } = &action
        {
            self.maintenance.pending = Some(*provider);
            self.maintenance.error = None;
        }
        if matches!(
            action,
            Action::State { .. } | Action::Chat(_) | Action::BackgroundChat(_) | Action::Revise { .. } | Action::Rename { .. } | Action::Delete(_)
        ) {
            self.history.dirty = true;
        }
        self.client = self.client.for_account(
            PROVIDERS[provider],
            self.conversations[provider].account_id.clone().unwrap_or_else(|| "default".into()),
            self.client.workspace_id(),
        );
        if let Action::Search(query) = &action {
            self.conversations[provider].search = query.clone();
            self.refresh_project_history();
        }
        let action = if matches!(action, Action::Refresh)
            && (self
                .job_monitor
                .profile_active(PROVIDERS[provider], self.conversations[provider].account_id.as_deref().unwrap_or("default"))
                || self
                    .accounts
                    .busy(PROVIDERS[provider], self.conversations[provider].account_id.as_deref().unwrap_or("default")))
        {
            Action::Search(self.conversations[provider].query.clone())
        } else {
            action
        };
        self.busy = true;
        self.operation = None;
        self.background_job = None;
        self.workspaces.error = None;
        self.activity = match &action {
            | Action::GuidanceFiles
            | Action::ReadGuidance(_)
            | Action::WriteGuidance(_)
            | Action::ImportGuidance(_)
            | Action::ImportSkillFolder {
                ..
            } => "지침·스킬 확인 중…",
            | Action::Guidance
            | Action::SetSkillEnabled {
                ..
            } => "지침·스킬 확인 중…",
            | Action::SkillProposals
            | Action::DecideSkillProposal {
                ..
            }
            | Action::DecideMcpProposal {
                ..
            } => "스킬 제안 처리 중…",
            | Action::Runtime {
                ..
            } => "런타임 확인 / 설치 중…",
            | Action::Git {
                ..
            } => "Git 처리 중…",
            | Action::State {
                ..
            } => "대화 상태 저장 중…",
            | Action::Context(_) | Action::AttachFiles(_) => "파일 첨부 중…",
            | Action::Projects
            | Action::SetProjectVerification {
                ..
            }
            | Action::OpenProject(_)
            | Action::ForgetProject(_)
            | Action::CreateWorktree {
                ..
            }
            | Action::RemoveWorktree(_) => "프로젝트 / worktree 처리 중…",
            | Action::Changes(_)
            | Action::Diff {
                ..
            } => "변경 확인 중…",
            | Action::Refresh => "계정과 모델 확인 중…",
            | Action::Search(_) => "대화 검색 중…",
            | Action::Login => "브라우저 로그인 대기 중…",
            | Action::Logout => "로그아웃 중…",
            | Action::Chat(_)
            | Action::BackgroundChat(_)
            | Action::Revise {
                ..
            } => "응답 생성 중…",
            | Action::Load {
                ..
            }
            | Action::OpenJob(_)
            | Action::Reference {
                ..
            } => "대화 불러오는 중…",
            | Action::Rename {
                ..
            } => "제목 저장 중…",
            | Action::Delete(_) => "대화 삭제 중…",
        };
        self.conversations[provider].error = None;
        if let Action::Search(query) = &action {
            self.search_changed_at = None;
            self.search_task = None;
            for c in &mut self.conversations {
                c.search = query.clone();
            }
        }
        self.client.start(
            PROVIDERS[provider],
            action,
            std::array::from_fn(|i| (self.conversations[i].query.clone(), self.conversations[i].account_id.clone())),
            self.tx.clone(),
            ctx.clone(),
        );
    }

    pub(super) fn updates(&mut self, ctx: &Wake) -> bool {
        let mut changed = false;
        while let Ok((index, update)) = self.rx.try_recv() {
            changed = true;
            match update {
                | Update::GuidanceFiles {
                    scope,
                    catalog,
                } =>
                    if scope == self.guidance_editor.scope && scope == self.guidance_editor_scope() {
                        self.guidance_editor.catalog = catalog;
                    },
                | Update::McpProposals {
                    scope,
                    proposals,
                } =>
                    if scope == self.guidance_editor.scope && scope == self.guidance_editor_scope() {
                        self.guidance_editor.mcp_proposals = proposals;
                    },
                | Update::SkillProposals {
                    scope,
                    proposals,
                } =>
                    if scope == self.guidance_editor.scope && scope == self.guidance_editor_scope() {
                        self.guidance_editor.proposals = proposals;
                    },
                | Update::GuidanceFile {
                    scope,
                    file,
                    saved,
                } =>
                    if scope == self.guidance_editor.scope && scope == self.guidance_editor_scope() {
                        let new_skill = !saved && file.revision.is_none() && file.id.ends_with("/SKILL.md");
                        self.guidance_editor.load(file);
                        if new_skill {
                            self.guidance_editor.text = format!(
                                "---\nname: {}\ndescription: {}\n---\n",
                                serde_json::json!(self.guidance_editor.name.trim()),
                                serde_json::json!(self.guidance_editor.description.trim())
                            );
                            self.guidance_editor.creating = false;
                        }
                        if saved {
                            self.guidance_editor.message = self.text("저장되었습니다. 다음 턴부터 반영됩니다.");
                            for conversation in &mut self.conversations {
                                conversation.guidance.snapshot = None;
                            }
                        }
                    },
                | Update::GuidanceImported {
                    scope,
                    text,
                } =>
                    if scope == self.guidance_editor.scope && scope == self.guidance_editor_scope() {
                        self.guidance_editor.text = text;
                        self.guidance_editor.message = self.text("파일을 불러왔습니다. 저장을 눌러 적용하세요.");
                    },
                | Update::Guidance {
                    scope,
                    snapshot,
                } => {
                    let current = gpui_agent_presentation::guidance_scope(self.client.workspace_id(), self.conversations[index].account_id.as_deref());
                    if current == scope {
                        self.conversations[index].guidance.receive(&scope, snapshot);
                    }
                },
                | Update::JobLoaded(detail, saved) => self.conversations[index].resume_job(&detail, saved),
                | Update::Revised(prepared) => {
                    let conversation = &mut self.conversations[index];
                    if let Some(skills) = conversation.skills_before_edit.take() {
                        conversation.guidance.selected = skills;
                    }
                    conversation.save_draft();
                    conversation.new_chat();
                    conversation.id = Some(prepared.conversation.id);
                    conversation.messages = prepared.conversation.messages;
                    conversation.timeline = prepared.conversation.timeline;
                    conversation.model = prepared.request.model.clone().unwrap_or_default();
                    conversation.versions = prepared.versions;
                    conversation.begin_turn(&prepared.request);
                },
                | Update::Versions(versions) => self.conversations[index].versions = versions,
                | Update::BackgroundJob(id) => {
                    self.background_job = Some(id);
                    self.activity = "대기 중";
                },
                | Update::Runtime(info) => self.maintenance.apply(info),
                | Update::Git {
                    overview,
                    restore,
                } =>
                    if self.workspaces.selected == Some(overview.workspace_id) {
                        self.workspaces.git.apply(*overview, restore);
                        self.workspaces.changes = None;
                        self.workspaces.diff = None;
                    },
                | Update::Attached(attachments) => {
                    let conversation = &mut self.conversations[index];
                    let mut combined = conversation.attachments.clone();
                    combined.extend(attachments);
                    match gpui_agent_contracts::validate_attachments(&combined) {
                        | Ok(()) => {
                            conversation.attachments = combined;
                            self.attachment_ui.open = false;
                        },
                        | Err(error) => conversation.error = Some(error.into()),
                    }
                },

                | Update::Projects(catalog) => {
                    let opened = catalog.registered_workspace_id;
                    self.workspaces.apply(catalog);
                    if let Some(id) = opened {
                        self.workspaces.next_selection = Some(Some(id));
                    }
                    if let Some(workspace) = self.workspaces.selected_workspace() {
                        self.cwd = workspace.path.clone();
                        if self.client.workspace_id().is_none() {
                            self.client.select_workspace(Some(workspace.id));
                        }
                    }
                    self.refresh_project_history();
                },
                | Update::Changes(changes) =>
                    if self.workspaces.selected == Some(changes.workspace_id) {
                        self.workspaces.changes = Some(changes);
                        self.workspaces.diff = None;
                    },
                | Update::Diff(diff) =>
                    if self.workspaces.selected == Some(diff.workspace_id) {
                        self.workspaces.diff = Some(diff);
                    },
                | Update::Providers(providers) => self.providers = providers,
                | Update::Status(status) => {
                    if let Some(account) = self.accounts.providers[index]
                        .accounts
                        .iter_mut()
                        .find(|a| a.profile.id == self.conversations[index].account_id.as_deref().unwrap_or("default"))
                        && !account.busy
                    {
                        account.auth.status = Some(status.clone());
                    }
                    if !status.authenticated {
                        self.conversations[index].models.clear();
                    }
                    self.conversations[index].status = Some(status);
                },
                | Update::Models(models) => self.conversations[index].models = models,
                | Update::History {
                    query,
                    conversations,
                } => {
                    if self.search_changed_at.is_some() && self.conversations[index].search != query {
                        continue;
                    }
                    let conversation = &mut self.conversations[index];
                    if let Some(current) = conversations.iter().find(|c| Some(c.id) == conversation.id) {
                        conversation.title = Some(current.title.clone());
                    }
                    if query.is_empty() {
                        conversation.view_query.clear();
                    }
                    conversation.query = query;
                    conversation.history = conversations;
                },
                | Update::Renamed {
                    id,
                    title,
                } => {
                    if self.conversations[index].id == Some(id) {
                        self.conversations[index].title = Some(title.clone());
                    }
                    if let Some(reference) = &mut self.reference
                        && reference.provider == index
                        && reference.conversation.id == id
                    {
                        reference.title = title.clone();
                    }
                    if let Some(saved) = self.conversations[index].history.iter_mut().find(|saved| saved.id == id) {
                        saved.title = title;
                    }
                },
                | Update::Deleted(id) => {
                    if self
                        .reference
                        .as_ref()
                        .is_some_and(|reference| reference.provider == index && reference.conversation.id == id)
                    {
                        self.reference = None;
                    }
                    let conversation = &mut self.conversations[index];
                    conversation.history.retain(|saved| saved.id != id);
                    conversation.versions.retain(|version| version.conversation_id != id);
                    conversation.drafts.retain(|draft| draft.conversation_id != Some(id));
                    if conversation.id == Some(id) {
                        conversation.new_chat();
                    }
                },
                | Update::Loaded {
                    conversation: saved,
                    message_id,
                    query,
                } => {
                    let conversation = &mut self.conversations[index];
                    if conversation.id != Some(saved.id) {
                        self.work_scroll.set_offset(Default::default());
                    }
                    conversation.save_draft();
                    conversation.editing = None;
                    conversation.versions.clear();
                    conversation.title = conversation
                        .history
                        .iter()
                        .find(|c| c.id == saved.id)
                        .map(|c| c.title.clone())
                        .or_else(|| conversation.title.clone());
                    conversation.id = Some(saved.id);
                    conversation.model = saved.model.unwrap_or_default();
                    conversation.session_id = saved.session_id;
                    conversation.timeline = saved.timeline;
                    conversation.pending = None;
                    conversation.messages = saved.messages;
                    conversation.assistant = None;
                    conversation.input.clear();
                    conversation.attachments.clear();
                    conversation.guidance.selected.clear();
                    conversation.guidance.extensions = conversation
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.user)
                        .and_then(|message| message.guidance.as_ref())
                        .and_then(|receipt| receipt.extensions.clone())
                        .unwrap_or_default();
                    if conversation
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.user)
                        .and_then(|message| message.guidance.as_ref())
                        .is_some_and(|receipt| receipt.extensions.is_some())
                    {
                        conversation.guidance.selected = conversation
                            .messages
                            .iter()
                            .rev()
                            .find(|message| message.user)
                            .map(|message| message.skills.clone())
                            .unwrap_or_default();
                    }
                    if let Some(index) = conversation.drafts.iter().position(|draft| draft.conversation_id == Some(saved.id)) {
                        let draft = conversation.drafts.remove(index);
                        conversation.input = draft.input;
                        conversation.attachments = draft.attachments;
                        conversation.guidance.selected = draft.skills;
                        conversation.guidance.extensions = draft.extensions;
                    }
                    let target = message_id.or_else(|| {
                        (!query.is_empty())
                            .then(|| conversation.messages.first().and_then(|message| message.id))
                            .flatten()
                    });
                    conversation.view_query = query;
                    conversation.scroll_to = Some(target.map_or(ScrollTarget::Bottom, ScrollTarget::Message));
                },
                | Update::Referenced {
                    conversation,
                    title,
                } => {
                    self.reference = Some(ReferenceConversation {
                        provider: index,
                        title,
                        conversation,
                    });
                    self.right_panels.reference_open = true;
                    self.save_right_panels();
                },
                | Update::Event(event) => {
                    if matches!(&event, StreamEvent::Agent(Event::Verification { .. })) {
                        self.activity = "필수 검증 중";
                    }
                    if let StreamEvent::Started {
                        operation_id,
                    } = &event
                    {
                        self.operation = Some(*operation_id);
                        if self.background_job.is_some() {
                            self.activity = "응답 생성 중…";
                        }
                    }
                    if let StreamEvent::Agent(Event::Authorization {
                        url, ..
                    }) = &event
                        && PROVIDERS[index] != gpui_agent_contracts::Provider::Claude
                    {
                        ctx.open_url(url);
                    }
                    self.conversations[index].event(event);
                },
                | Update::Error(error) => {
                    if let Some(provider) = self.maintenance.pending {
                        self.maintenance.error = Some((provider, error.clone()));
                    }
                    self.workspaces.error = Some(error.clone());
                    self.conversations[index].error = Some(error);
                    self.conversations[index].authorization = None;
                },
                | Update::InteractionSent {
                    request_id,
                    result,
                } => {
                    if let Some(pending) = &mut self.conversations[index].pending
                        && pending.request_id == request_id
                    {
                        match result {
                            | Ok(()) => {},
                            | Err(error) => {
                                pending.sending = false;
                                self.conversations[index].error = Some(error);
                            },
                        }
                    }
                },
                | Update::Finished => {
                    let agent_turn = self.conversations[index].assistant.is_some();
                    self.workspaces.git.writing = false;
                    if self.background_job.is_some() || self.operation.is_some() {
                        self.workspaces.git.last = None;
                    }
                    self.maintenance.pending = None;
                    self.conversations[index].pending = None;
                    // A title is set optimistically while a history entry
                    // loads. If the load never produced a
                    // conversation (e.g. it was deleted), the title is stale.
                    if self.conversations[index].id.is_none() && self.conversations[index].messages.is_empty() {
                        self.conversations[index].title = None;
                    }
                    self.busy = false;
                    if agent_turn {
                        self.auto_sync_claude_plugins();
                    }
                    if let Some(provider) = self.maintenance.refresh_accounts.take() {
                        self.refresh_accounts(provider);
                    }
                    self.operation = None;
                    self.background_job = None;
                    self.conversations[index].assistant = None;
                    let opened = self.workspaces.catalog.registered_workspace_id.take();
                    if let Some(id) = self.workspaces.next_selection.take() {
                        self.select_workspace(id, ctx);
                    }
                    if opened.is_some() && self.workspaces.selected == opened {
                        self.conversations[self.selected].save_draft();
                        self.conversations[self.selected].new_chat();
                        self.close_panel();
                        self.workspaces.focus_composer = true;
                    }
                    if let Some(entry) = self.history.pending.take() {
                        self.open_history(entry);
                    }
                    if std::mem::take(&mut self.history.dirty) {
                        self.refresh_project_history();
                    }
                },
            }
        }
        changed |= self.history.updates();
        changed
    }

    pub(super) fn select_workspace(&mut self, id: Option<i64>, ctx: &Wake) {
        if self.busy {
            return;
        }
        if id.is_some() && self.workspaces.selected == id {
            self.workspaces.open = false;
            return;
        }
        let path = if let Some(id) = id {
            let Some(workspace) = self
                .workspaces
                .catalog
                .projects
                .iter()
                .flat_map(|project| &project.workspaces)
                .find(|workspace| workspace.id == id && !workspace.missing)
            else {
                return;
            };
            workspace.path.clone()
        } else {
            String::new()
        };
        let next = self
            .workspaces
            .cached
            .remove(&id)
            .unwrap_or_else(|| std::array::from_fn(|_| Conversation::default()));
        let previous = std::mem::replace(&mut self.conversations, next);
        let old = self.workspaces.selected;
        if old != id {
            self.workspaces.cached.insert(old, previous);
            self.workspaces.providers.insert(old, self.selected);
        }
        self.selected = self.workspaces.providers.get(&id).copied().unwrap_or(self.selected);
        self.search_changed_at = None;
        self.search_task = None;
        self.workspaces.selected = id;
        self.workspaces.open = false;
        self.workspaces.diff_open = false;
        self.workspaces.git = Default::default();
        self.client.select_workspace(id);
        self.client = self.client.for_account(
            PROVIDERS[self.selected],
            self.conversations[self.selected].account_id.clone().unwrap_or_else(|| "default".into()),
            id,
        );
        self.cwd = path;
        self.reference = None;
        self.attachment_ui = Default::default();
        self.dialog = None;
        self.workspaces.changes = None;
        self.workspaces.diff = None;
        if id.is_some() {
            self.start(Action::Refresh, ctx);
        }
    }

    pub(super) fn send(&mut self, ctx: &Wake) { self.send_task(ctx, false); }

    pub(super) fn send_task(&mut self, ctx: &Wake, computer_use: bool) {
        if self.busy {
            return;
        }
        if self.conversations[self.selected].editing.is_some() {
            self.revise_question(ctx);
            return;
        }
        let conversation = &mut self.conversations[self.selected];
        let text = conversation.input.clone();
        if text.trim().is_empty() && conversation.attachments.is_empty() {
            return;
        }
        if text.len() > 256 * 1024 {
            conversation.error = Some("Message exceeds 256 KiB".into());
            return;
        }
        if let Err(error) = gpui_agent_contracts::validate_attachments(&conversation.attachments) {
            conversation.error = Some(error.into());
            return;
        }
        let attachments = std::mem::take(&mut conversation.attachments);
        conversation.input.clear();
        let request = ChatRequest {
            extensions: Some(conversation.guidance.extensions.clone()),
            skills: conversation.guidance.selected.clone(),
            computer_use,
            claude_permission_mode: conversation.permission_mode,
            codex_execution_policy: conversation.codex_execution_policy,
            attachments: attachments.clone(),
            text: text.clone(),
            model: (!conversation.model.is_empty()).then(|| conversation.model.clone()),
            session_id: conversation.session_id.clone(),
            conversation_id: conversation.id,
        };
        let background = self
            .providers
            .iter()
            .any(|info| info.provider == PROVIDERS[self.selected] && info.capabilities.background_jobs);
        conversation.begin_turn(&request);
        self.start(if background { Action::BackgroundChat(request) } else { Action::Chat(request) }, ctx);
    }

    fn revise_question(&mut self, ctx: &Wake) {
        let conversation = &mut self.conversations[self.selected];
        let Some(question) = conversation.last_completed_question() else {
            return;
        };
        let Some(text) = conversation.editing.clone() else {
            return;
        };
        if let Err(error) = gpui_agent_contracts::validate_message(&text, None, &conversation.messages[question].attachments) {
            conversation.error = Some(error.into());
            return;
        }
        let action = Action::Revise {
            extensions: Some(conversation.guidance.extensions.clone()),
            skills: conversation.guidance.selected.clone(),
            id: conversation.id.expect("completed question has a conversation"),
            expected_messages: conversation.messages.len(),
            original: conversation.messages[question].text.clone(),
            text,
            model: (!conversation.model.is_empty()).then(|| conversation.model.clone()),
            permission_mode: conversation.permission_mode,
            codex_execution_policy: conversation.codex_execution_policy,
            background: self
                .providers
                .iter()
                .any(|info| info.provider == PROVIDERS[self.selected] && info.capabilities.background_jobs),
        };
        self.start(action, ctx);
    }

    pub(super) fn stop_chat(&self) {
        if let Some(id) = self.background_job {
            self.client.cancel_job(PROVIDERS[self.selected], id, None, self.tx.clone(), self.wake.clone());
        } else if let Some(id) = self.operation {
            self.client.cancel(id);
        }
    }
}
