use crate::ui::Wake;
use anyhow::{Context,
             Result,
             ensure};
use gpui_agent_contracts::{AuthStatus,
                           JobDetail,
                           JobEvent,
                           LoginRequest,
                           Provider,
                           RenameRequest,
                           StreamEvent,
                           provider_index,
                           provider_name};
use gpui_agent_presentation::{Action,
                              Update};
use reqwest::blocking::{Client,
                        RequestBuilder,
                        Response};
use std::{io::{BufRead,
               BufReader,
               Read},
          sync::mpsc::SyncSender,
          time::Duration};

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    url: String,
    token: String,
    workspace_id: Option<i64>,
    account: Option<(Provider, String)>,
}

pub(super) enum InteractionTarget {
    Operation(u64),
    Job(i64),
}

impl ApiClient {
    pub fn new(url: String, token: String) -> Result<Self> {
        validate_url(&url)?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(950))
            .build()?;
        Ok(Self {
            client,
            url,
            token,
            workspace_id: None,
            account: None,
        })
    }

    pub(super) fn for_account(&self, provider: Provider, account: String, workspace: Option<i64>) -> Self {
        Self {
            account: Some((provider, account)),
            workspace_id: workspace,
            ..self.clone()
        }
    }

    pub(super) fn origin(&self) -> &str { &self.url }

    pub(super) fn workspace_id(&self) -> Option<i64> { self.workspace_id }

    pub(super) fn guidance_editor_scope(&self) -> String { format!("{}:{:?}", self.url, self.workspace_id) }

    pub fn select_workspace(&mut self, id: Option<i64>) { self.workspace_id = id; }

    pub(super) fn workspace_files(&self, provider: Provider, path: &str) -> Result<gpui_agent_contracts::WorkspaceFiles> {
        let url = reqwest::Url::parse_with_params(&format!("{}/v1/providers/{}/files", self.url, provider_name(provider)), [("path", path)])?;
        let request = self
            .request(reqwest::Method::GET, &format!("{}?{}", url.path(), url.query().unwrap_or_default()))
            .timeout(Duration::from_secs(15));
        Ok(Self::checked(request)?.json()?)
    }

    pub(super) fn response(&self, method: reqwest::Method, route: &str, body: Option<serde_json::Value>) -> Result<Response> {
        let mut request = self.request(method, route);
        if let Some(body) = body {
            request = request.json(&body);
        }
        Self::checked(request)
    }

    pub(super) fn computer_status(&self) -> Result<gpui_agent_contracts::ComputerStatus> {
        Ok(Self::checked(self.request(reqwest::Method::GET, "/v1/control/computer").timeout(Duration::from_secs(15)))?.json()?)
    }

    pub(super) fn grant_computer(&self, grant: gpui_agent_contracts::ComputerGrantRequest) -> Result<()> {
        Self::checked(
            self.request(reqwest::Method::POST, "/v1/control/computer/grant")
                .timeout(Duration::from_secs(15))
                .json(&serde_json::json!({"job_id":grant.job_id,"enabled":grant.enabled})),
        )?;
        Ok(())
    }

    fn request(&self, method: reqwest::Method, route: &str) -> RequestBuilder {
        let mut request = self.client.request(method, format!("{}{route}", self.url)).bearer_auth(&self.token);
        if let Some(id) = self.workspace_id {
            request = request.header("X-KND-Workspace", id.to_string());
        }
        if let Some((provider, account)) = &self.account {
            request = request.header("X-KND-Account", account).header("X-KND-Provider", provider_name(*provider));
        }
        request
    }

    fn checked(request: RequestBuilder) -> Result<Response> {
        let response = request.send().context("로컬 API 서버에 연결할 수 없습니다")?;
        let status = response.status();
        if !status.is_success() {
            let value: serde_json::Value = response.json().unwrap_or_default();
            anyhow::bail!("{}", value["error"].as_str().unwrap_or("API 요청이 실패했습니다"));
        }
        Ok(response)
    }

    pub fn start(
        &self,
        provider: Provider,
        action: Action,
        queries: [(String, Option<String>); gpui_agent_contracts::PROVIDERS.len()],
        tx: SyncSender<(usize, Update)>,
        ctx: Wake,
    ) {
        let client = self.clone();
        std::thread::spawn(move || {
            let index = provider_index(provider);
            let sidebar_query = match &action {
                | Action::Search(query) => Some(Some(query.clone())),
                | Action::Refresh => Some(None),
                | _ => None,
            };
            let send = |update| {
                let result = tx.send((index, update));
                ctx.notify();
                result.is_ok()
            };
            if let Err(error) = client.run(provider, action, &queries[index].0, &send) {
                send(Update::Error(error.to_string()));
            }
            if let Some(query) = sidebar_query {
                for other in gpui_agent_contracts::PROVIDERS.into_iter().filter(|other| *other != provider) {
                    let other_index = provider_index(other);
                    let query = query.as_deref().unwrap_or(&queries[other_index].0);
                    let other_send = |update| {
                        let result = tx.send((other_index, update));
                        ctx.notify();
                        result.is_ok()
                    };
                    let scoped = client.for_account(other, queries[other_index].1.clone().unwrap_or_else(|| "default".into()), client.workspace_id());
                    if let Err(error) = scoped.run(other, Action::Search(query.into()), query, &other_send) {
                        other_send(Update::Error(error.to_string()));
                    }
                }
            }
            send(Update::Finished);
        });
    }

    pub fn respond(
        &self,
        provider: Provider,
        target: InteractionTarget,
        request_id: u64,
        response: gpui_agent_contracts::InteractionResponse,
        tx: SyncSender<(usize, Update)>,
        ctx: Wake,
    ) {
        let client = self.clone();
        std::thread::spawn(move || {
            let path = match target {
                | InteractionTarget::Operation(id) => format!("/v1/operations/{id}/interactions/{request_id}"),
                | InteractionTarget::Job(id) => format!("/v1/jobs/{id}/interactions/{request_id}"),
            };
            let result = (|| client.response(reqwest::Method::POST, &path, Some(serde_json::to_value(response)?)))()
                .map(|_| ())
                .map_err(|error: anyhow::Error| error.to_string());
            let _ = tx.send((
                provider_index(provider),
                Update::InteractionSent {
                    request_id,
                    result,
                },
            ));
            ctx.notify();
        });
    }

    pub fn cancel(&self, id: u64) {
        let client = self.clone();
        std::thread::spawn(move || {
            let _ = client.response(reqwest::Method::DELETE, &format!("/v1/operations/{id}"), None);
        });
    }

    pub(super) fn cancel_job(&self, provider: Provider, id: i64, request: Option<u64>, tx: SyncSender<(usize, Update)>, ctx: Wake) {
        let client = self.clone();
        std::thread::spawn(move || {
            let result = client
                .response(reqwest::Method::DELETE, &format!("/v1/jobs/{id}"), None)
                .map(|_| ())
                .map_err(|error| error.to_string());
            if let Some(request_id) = request {
                let _ = tx.send((
                    provider_index(provider),
                    Update::InteractionSent {
                        request_id,
                        result,
                    },
                ));
                ctx.notify();
            } else if let Err(error) = result {
                let _ = tx.send((provider_index(provider), Update::Error(error)));
                ctx.notify();
            }
        });
    }

    pub(super) fn run(&self, provider: Provider, action: Action, query: &str, send: &dyn Fn(Update) -> bool) -> Result<()> {
        let route = format!("/v1/providers/{}", provider_name(provider));
        match action {
            | Action::GuidanceFiles => {
                send(Update::GuidanceFiles {
                    scope: self.guidance_editor_scope(),
                    catalog: self.response(reqwest::Method::GET, "/v1/guidance/files", None)?.json()?,
                });
                self.run(provider, Action::Guidance, query, send)?;
                self.run(provider, Action::SkillProposals, query, send)?;
            },
            | Action::SkillProposals => {
                send(Update::McpProposals {
                    scope: self.guidance_editor_scope(),
                    proposals: self.response(reqwest::Method::GET, "/v1/mcp-proposals", None)?.json()?,
                });
                send(Update::SkillProposals {
                    scope: self.guidance_editor_scope(),
                    proposals: self.response(reqwest::Method::GET, "/v1/skill-proposals", None)?.json()?,
                });
            },
            | Action::DecideSkillProposal {
                id,
                allow,
            } => {
                self.response(
                    reqwest::Method::POST,
                    &format!("/v1/skill-proposals/{id}"),
                    Some(serde_json::json!({"allow":allow})),
                )?;
                self.run(provider, Action::GuidanceFiles, query, send)?;
            },
            | Action::DecideMcpProposal {
                id,
                allow,
            } => {
                self.response(
                    reqwest::Method::POST,
                    &format!("/v1/mcp-proposals/{id}"),
                    Some(serde_json::json!({"allow":allow})),
                )?;
                self.run(provider, Action::GuidanceFiles, query, send)?;
            },
            | Action::ReadGuidance(id) => {
                send(Update::GuidanceFile {
                    scope: self.guidance_editor_scope(),
                    file: self
                        .response(reqwest::Method::POST, "/v1/guidance/files/read", Some(serde_json::json!({"id":id})))?
                        .json()?,
                    saved: false,
                });
            },
            | Action::WriteGuidance(request) => {
                send(Update::GuidanceFile {
                    scope: self.guidance_editor_scope(),
                    file: self
                        .response(reqwest::Method::PUT, "/v1/guidance/files", Some(serde_json::to_value(request)?))?
                        .json()?,
                    saved: true,
                });
                self.run(provider, Action::GuidanceFiles, query, send)?;
            },
            | Action::ImportSkillFolder {
                path,
                id,
            } => {
                let request = gpui_agent_infrastructure::GuidanceFiles::collect_skill(&path, id)?;
                self.response(reqwest::Method::POST, "/v1/skills/import", Some(serde_json::to_value(request)?))?;
                self.run(provider, Action::GuidanceFiles, query, send)?;
            },
            | Action::ImportGuidance(path) => {
                let file = std::fs::File::open(path)?;
                ensure!(file.metadata()?.is_file(), "Select a Markdown file");
                let mut text = String::new();
                file.take((gpui_agent_contracts::MAX_SKILL_BYTES + 1) as u64).read_to_string(&mut text)?;
                ensure!(text.len() <= gpui_agent_contracts::MAX_SKILL_BYTES, "Guidance file exceeds 64 KiB");
                send(Update::GuidanceImported {
                    scope: self.guidance_editor_scope(),
                    text,
                });
            },
            | Action::Guidance => {
                send(Update::Guidance {
                    scope: gpui_agent_presentation::guidance_scope(self.workspace_id, self.account.as_ref().map(|(_, account)| account.as_str())),
                    snapshot: self.response(reqwest::Method::GET, "/v1/guidance", None)?.json()?,
                });
            },
            | Action::SetSkillEnabled {
                id,
                enabled,
            } => {
                self.response(reqwest::Method::PUT, "/v1/skills/enabled", Some(serde_json::json!({"id":id,"enabled":enabled})))?;
                send(Update::Guidance {
                    scope: gpui_agent_presentation::guidance_scope(self.workspace_id, self.account.as_ref().map(|(_, account)| account.as_str())),
                    snapshot: self.response(reqwest::Method::GET, "/v1/guidance", None)?.json()?,
                });
            },
            | Action::OpenJob(id) => {
                let detail: JobDetail = self.response(reqwest::Method::GET, &format!("/v1/jobs/{id}"), None)?.json()?;
                ensure!(
                    detail.job.scope.provider == provider
                        && detail.job.scope.workspace_id == self.workspace_id
                        && detail.job.scope.account_id == self.account.as_ref().map_or("default", |(_, account)| account),
                    "Job scope changed; refresh the project history"
                );
                let scoped = self.for_account(provider, detail.job.scope.account_id.clone(), detail.job.scope.workspace_id);
                let saved = detail.job.conversation_id.and_then(|id| {
                    scoped
                        .response(reqwest::Method::GET, &format!("{route}/conversations/{id}"), None)
                        .ok()?
                        .json()
                        .ok()
                });
                let conversation_id = detail.job.conversation_id;
                match (saved, detail.job.state.terminal()) {
                    | (Some(conversation), true) => {
                        send(Update::Loaded {
                            conversation,
                            message_id: None,
                            query: query.into(),
                        });
                    },
                    | (saved, _) => {
                        send(Update::JobLoaded(detail, saved));
                        if !send(Update::BackgroundJob(id)) {
                            return Ok(());
                        }
                        scoped.job_stream(id, send)?;
                    },
                }
                if let Some(id) = conversation_id {
                    send(Update::Versions(
                        scoped
                            .response(reqwest::Method::GET, &format!("{route}/conversations/{id}/versions"), None)?
                            .json()?,
                    ));
                }
                scoped.auth(&route, send)?;
            },
            | Action::Context(request) => {
                let attachment = self
                    .response(reqwest::Method::POST, &format!("{route}/context"), Some(serde_json::to_value(request)?))?
                    .json()?;
                send(Update::Attached(vec![attachment]));
            },
            | Action::AttachFiles(paths) => {
                let mut attachments = Vec::new();
                ensure!(
                    paths.len() <= gpui_agent_contracts::attachment::MAX_ATTACHMENTS,
                    "At most 8 attachments are allowed per message"
                );
                for path in paths {
                    ensure!(path.is_file(), "Attachment must be a regular file");
                    let file = std::fs::File::open(&path).context("Cannot open attachment")?;
                    ensure!(file.metadata()?.is_file(), "Attachment must be a regular file");
                    let mut bytes = Vec::new();
                    file.take((gpui_agent_contracts::attachment::MAX_IMAGE_BYTES + 1) as u64)
                        .read_to_end(&mut bytes)?;
                    ensure!(bytes.len() <= gpui_agent_contracts::attachment::MAX_IMAGE_BYTES, "Attachment exceeds 2 MiB");
                    let name = path.file_name().context("Missing filename")?.to_string_lossy().into_owned();
                    attachments.push(gpui_agent_contracts::Attachment::from_bytes(name, &bytes).map_err(anyhow::Error::msg)?);
                }
                gpui_agent_contracts::validate_attachments(&attachments).map_err(anyhow::Error::msg)?;
                send(Update::Attached(attachments));
            },

            | Action::Git {
                workspace,
                command,
            } => {
                let restore = match &command {
                    | gpui_agent_contracts::GitCommand::PreviewRestore {
                        checkpoint,
                    } => Some(checkpoint.clone()),
                    | _ => None,
                };
                send(Update::Git {
                    overview: self
                        .response(
                            reqwest::Method::POST,
                            &format!("/v1/workspaces/{workspace}/git"),
                            Some(serde_json::to_value(command)?),
                        )?
                        .json()?,
                    restore,
                });
            },
            | Action::State {
                id,
                state,
            } => {
                self.response(
                    reqwest::Method::PATCH,
                    &format!("{route}/conversations/{id}/state"),
                    Some(serde_json::to_value(state)?),
                )?;
                self.history(&route, query, send)?;
            },
            | Action::Runtime {
                provider,
                command,
            } => {
                send(Update::Runtime(
                    self.response(
                        reqwest::Method::POST,
                        &format!("/v1/runtimes/{}", provider_name(provider)),
                        Some(serde_json::to_value(command)?),
                    )?
                    .json()?,
                ));
                send(Update::Providers(self.response(reqwest::Method::GET, "/v1/providers", None)?.json()?));
            },
            | Action::Projects => {
                send(Update::Projects(self.response(reqwest::Method::GET, "/v1/projects", None)?.json()?));
            },
            | Action::SetProjectVerification {
                project,
                plan,
            } => {
                send(Update::Projects(
                    self.response(
                        reqwest::Method::PUT,
                        &format!("/v1/projects/{project}/verification"),
                        Some(serde_json::to_value(plan)?),
                    )?
                    .json()?,
                ));
            },
            | Action::OpenProject(path) => {
                let catalog: gpui_agent_contracts::ProjectCatalog = self
                    .response(reqwest::Method::POST, "/v1/projects", Some(serde_json::json!({"path":path})))?
                    .json()?;
                ensure!(
                    catalog.registered_workspace_id.is_some_and(|id| catalog
                        .projects
                        .iter()
                        .flat_map(|project| &project.workspaces)
                        .any(|workspace| workspace.id == id && !workspace.missing)),
                    "서버에서 열린 프로젝트의 작업 폴더를 확인할 수 없습니다."
                );
                send(Update::Projects(catalog));
            },
            | Action::CreateWorktree {
                project_id,
                branch,
                base,
            } => {
                send(Update::Projects(
                    self.response(
                        reqwest::Method::POST,
                        &format!("/v1/projects/{project_id}/worktrees"),
                        Some(serde_json::json!({"branch":branch,"base":base})),
                    )?
                    .json()?,
                ));
            },
            | Action::ForgetProject(id) => {
                send(Update::Projects(
                    self.response(reqwest::Method::DELETE, &format!("/v1/projects/{id}"), None)?.json()?,
                ));
            },
            | Action::RemoveWorktree(id) => {
                send(Update::Projects(
                    self.response(reqwest::Method::DELETE, &format!("/v1/workspaces/{id}"), None)?.json()?,
                ));
            },
            | Action::Changes(id) => {
                send(Update::Changes(
                    self.response(reqwest::Method::GET, &format!("/v1/workspaces/{id}/changes"), None)?.json()?,
                ));
            },
            | Action::Diff {
                workspace_id,
                path,
            } => {
                let mut url: reqwest::Url = format!("{}/v1/workspaces/{workspace_id}/diff", self.url).parse()?;
                url.query_pairs_mut().append_pair("path", &path);
                send(Update::Diff(
                    self.response(reqwest::Method::GET, &format!("{}?{}", url.path(), url.query().unwrap_or_default()), None)?
                        .json()?,
                ));
            },
            | Action::Refresh => {
                self.run(provider, Action::Guidance, query, send)?;
                send(Update::Projects(self.response(reqwest::Method::GET, "/v1/projects", None)?.json()?));
                self.history(&route, query, send)?;
                let providers = self.response(reqwest::Method::GET, "/v1/providers", None)?.json()?;
                if !send(Update::Providers(providers)) {
                    return Ok(());
                }
                self.auth(&route, send)?;
            },
            | Action::Logout => {
                send(Update::Status(self.response(reqwest::Method::POST, &format!("{route}/logout"), None)?.json()?));
            },
            | Action::Search(query) => self.history(&route, &query, send)?,
            | Action::Login => {
                let authenticated = self.stream(&format!("{route}/login"), serde_json::to_value(LoginRequest::default())?, send)?;
                if authenticated {
                    send(Update::Models(self.response(reqwest::Method::GET, &format!("{route}/models"), None)?.json()?));
                }
            },
            | Action::Chat(request) => {
                let result = self.stream(&format!("{route}/chat"), serde_json::to_value(request)?, send);
                self.history(&route, query, send)?;
                result?;
            },
            | Action::BackgroundChat(request) => {
                let job: gpui_agent_contracts::Job = self
                    .response(reqwest::Method::POST, &format!("{route}/jobs"), Some(serde_json::to_value(request)?))?
                    .json()?;
                if !send(Update::BackgroundJob(job.id)) {
                    return Ok(());
                }
                let result = self.job_stream(job.id, send);
                let history = self.history(&route, query, send);
                result?;
                history?;
            },
            | Action::Revise {
                extensions,
                skills,
                id,
                expected_messages,
                original,
                text,
                model,
                permission_mode,
                codex_execution_policy,
                background,
            } => {
                let source: gpui_agent_contracts::SavedConversation =
                    self.response(reqwest::Method::GET, &format!("{route}/conversations/{id}"), None)?.json()?;
                let question = source.messages.iter().rev().find(|message| message.user).context("No question to edit")?;
                ensure!(
                    source.messages.len() == expected_messages && question.text == original,
                    "대화가 변경되었습니다. 다시 불러온 뒤 수정하세요."
                );
                let revision = gpui_agent_contracts::ReviseRequest {
                    extensions,
                    skills: Some(skills),
                    message_id: question.id.context("Saved question has no ID")?,
                    text,
                    model,
                    claude_permission_mode: permission_mode,
                    codex_execution_policy,
                };
                let prepared: gpui_agent_contracts::PreparedRevision = self
                    .response(
                        reqwest::Method::POST,
                        &format!("{route}/conversations/{id}/revisions"),
                        Some(serde_json::to_value(revision)?),
                    )?
                    .json()?;
                let request = prepared.request.clone();
                if !send(Update::Revised(prepared)) {
                    return Ok(());
                }
                self.run(
                    provider,
                    if background { Action::BackgroundChat(request) } else { Action::Chat(request) },
                    query,
                    send,
                )?;
            },
            | Action::Load {
                id,
                message_id,
            } => {
                send(Update::Loaded {
                    conversation: self.response(reqwest::Method::GET, &format!("{route}/conversations/{id}"), None)?.json()?,
                    message_id,
                    query: query.to_owned(),
                });
                send(Update::Versions(
                    self.response(reqwest::Method::GET, &format!("{route}/conversations/{id}/versions"), None)?
                        .json()?,
                ));
                // History must load even if the runtime is unavailable.
                self.auth(&route, send)?;
            },
            | Action::Reference {
                id,
                title,
            } => {
                send(Update::Referenced {
                    conversation: self.response(reqwest::Method::GET, &format!("{route}/conversations/{id}"), None)?.json()?,
                    title,
                });
            },
            | Action::Rename {
                id,
                title,
            } => {
                self.response(
                    reqwest::Method::PATCH,
                    &format!("{route}/conversations/{id}"),
                    Some(serde_json::to_value(RenameRequest {
                        title: title.clone(),
                    })?),
                )?;
                send(Update::Renamed {
                    id,
                    title,
                });
                self.history(&route, query, send)?;
            },
            | Action::Delete(id) => {
                self.response(reqwest::Method::DELETE, &format!("{route}/conversations/{id}"), None)?;
                send(Update::Deleted(id));
                self.history(&route, query, send)?;
            },
        }
        Ok(())
    }

    fn history(&self, route: &str, query: &str, send: &dyn Fn(Update) -> bool) -> Result<()> {
        let mut url: reqwest::Url = format!("{}{route}/conversations", self.url).parse()?;
        url.query_pairs_mut().append_pair("q", query).append_pair("archived", "all");
        let path = format!("{route}/conversations?{}", url.query().unwrap_or_default());
        send(Update::History {
            query: query.to_owned(),
            conversations: self.response(reqwest::Method::GET, &path, None)?.json()?,
        });
        Ok(())
    }

    fn auth(&self, route: &str, send: &dyn Fn(Update) -> bool) -> Result<()> {
        let status: AuthStatus = self.response(reqwest::Method::GET, &format!("{route}/status"), None)?.json()?;
        let logged_in = status.authenticated;
        if send(Update::Status(status)) && logged_in {
            send(Update::Models(self.response(reqwest::Method::GET, &format!("{route}/models"), None)?.json()?));
        }
        Ok(())
    }

    fn stream(&self, route: &str, body: serde_json::Value, send: &dyn Fn(Update) -> bool) -> Result<bool> {
        let mut reader = BufReader::new(self.response(reqwest::Method::POST, route, Some(body))?);
        let mut terminal = false;
        loop {
            let mut line = String::new();
            let count = (&mut reader).take(16 * 1024 * 1024 + 1).read_line(&mut line)?;
            ensure!(count <= 16 * 1024 * 1024, "API event is too large");
            if count == 0 {
                break;
            }
            let event: StreamEvent = serde_json::from_str(&line).context("API 스트림을 읽을 수 없습니다")?;
            terminal |= event.is_terminal();
            let failed = matches!(event, StreamEvent::Error { .. } | StreamEvent::Cancelled);
            if !send(Update::Event(event)) {
                return Ok(false);
            }
            if failed {
                return Ok(false);
            }
        }
        ensure!(terminal, "응답이 완료되기 전에 연결이 끊겼습니다");
        Ok(true)
    }

    fn job_stream(&self, id: i64, send: &dyn Fn(Update) -> bool) -> Result<()> {
        let mut cursor = 0;
        loop {
            let response = self.response(reqwest::Method::GET, &format!("/v1/jobs/{id}/events?after={cursor}"), None)?;
            let mut reader = BufReader::new(response);
            loop {
                let mut line = String::new();
                let count = (&mut reader).take(16 * 1024 * 1024 + 1).read_line(&mut line)?;
                ensure!(count <= 16 * 1024 * 1024, "API event is too large");
                if count == 0 {
                    break;
                }
                let event: JobEvent = serde_json::from_str(&line).context("Cannot read job stream")?;
                if event.sequence <= cursor {
                    continue;
                }
                cursor = event.sequence;
                let terminal = event.event.is_terminal();
                if !send(Update::Event(event.event)) || terminal {
                    return Ok(());
                }
            }
            let detail: JobDetail = self.response(reqwest::Method::GET, &format!("/v1/jobs/{id}"), None)?.json()?;
            ensure!(
                !detail.job.state.terminal() || cursor < detail.job.last_sequence,
                "{}",
                detail.job.error.as_deref().unwrap_or("응답이 완료되기 전에 연결이 끊겼습니다")
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

pub(super) fn validate_url(url: &str) -> Result<()> {
    let parsed: reqwest::Url = url.parse()?;
    ensure!(
        parsed.username().is_empty() && parsed.password().is_none() && parsed.query().is_none() && parsed.fragment().is_none() && parsed.path() == "/",
        "API URL must be an origin without credentials, query or path"
    );
    ensure!(
        parsed.host_str().is_some()
            && (parsed.scheme() == "https" || (parsed.scheme() == "http" && matches!(parsed.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")))),
        "Remote APIs require HTTPS; HTTP is only allowed for loopback / SSH tunnels"
    );
    Ok(())
}
