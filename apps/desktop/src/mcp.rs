use crate::{client::ApiClient,
            ui::Wake};
use gpui_agent_contracts::*;
use reqwest::Method;
use std::sync::mpsc::{self,
                      Receiver};
mod oauth;
use std::collections::BTreeMap;
pub(super) enum Change {
    Accounts(Vec<AccountProfile>),
    Mcp(Vec<McpServer>, BTreeMap<String, McpOAuthStatus>),
    OAuth(Provider, String, String, McpOAuthStatus, bool),
}
/// MCP settings page state: shared server JSON plus per-account OAuth.
pub struct McpUi {
    pub open: bool,
    pub(super) provider: Provider,
    pub(super) account: String,
    pub(super) workspace: Option<i64>,
    pub(super) accounts: Vec<AccountProfile>,
    pub(super) json: String,
    pub(super) loaded_servers: bool,
    pub(super) catalog_changed: bool,
    pub(super) servers: Vec<McpServer>,
    pub(super) plugin_servers: Vec<McpServer>,
    pub(super) oauth: BTreeMap<String, McpOAuthStatus>,
    next_oauth_poll: std::time::Instant,
    pub(super) error: Option<String>,
    pub(super) pending: Option<Receiver<Result<Change, String>>>,
    pub(super) loaded: bool,
}
impl Default for McpUi {
    fn default() -> Self {
        Self {
            open: false,
            provider: Provider::Claude,
            account: "default".into(),
            workspace: None,
            accounts: vec![],
            json: "[]".into(),
            loaded_servers: false,
            catalog_changed: false,
            servers: vec![],
            plugin_servers: vec![],
            oauth: BTreeMap::new(),
            next_oauth_poll: std::time::Instant::now(),
            error: None,
            pending: None,
            loaded: false,
        }
    }
}
impl McpUi {
    pub(super) fn run(&mut self, ctx: &Wake, work: impl FnOnce() -> anyhow::Result<Change> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work().map_err(|e| e.to_string()));
            ctx.notify();
        });
    }

    pub(super) fn refresh(&mut self, ctx: &Wake, client: ApiClient) {
        let provider = self.provider;
        self.run(ctx, move || {
            Ok(Change::Accounts(
                client
                    .response(Method::GET, &format!("/v1/providers/{}/accounts", provider_name(provider)), None)?
                    .json()?,
            ))
        });
    }

    pub fn poll(&mut self, ctx: &Wake, client: &ApiClient) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.pending
            && let Ok(result) = rx.try_recv()
        {
            changed = true;
            self.pending = None;
            match result {
                | Ok(Change::Accounts(accounts)) => {
                    self.accounts = accounts;
                    self.loaded = true;
                },
                | Ok(Change::Mcp(servers, statuses)) => match serde_json::to_string_pretty(&servers) {
                    | Ok(json) => {
                        self.json = json;
                        self.loaded_servers = true;
                        self.servers = servers;
                        self.oauth = statuses;
                        self.catalog_changed = true;
                    },
                    | Err(error) => {
                        self.error = Some(format!("Cannot format MCP configuration: {error}"));
                        self.loaded_servers = false;
                    },
                },
                | Ok(Change::OAuth(provider, account, name, status, open)) =>
                    if provider == self.provider && account == self.account {
                        if open && let Some(url)=&status.authorization_url
                            && self.servers.iter().chain(&self.plugin_servers).any(|server|matches!(server,McpServer::Http{name:server_name,url:resource,..} if *server_name==name && mcp_oauth_endpoint(url,resource).is_ok())) {ctx.open_url(url);}
                        self.oauth.insert(name, status);
                        self.next_oauth_poll = std::time::Instant::now() + std::time::Duration::from_secs(1);
                    },
                | Err(error) => self.error = Some(error),
            }
        }
        if !self.open {
            return changed;
        }
        if self.pending.is_none()
            && std::time::Instant::now() >= self.next_oauth_poll
            && let Some(name) = self
                .oauth
                .iter()
                .find(|(_, status)| status.state == McpOAuthState::Pending)
                .map(|(name, _)| name.clone())
        {
            let client = client.for_account(self.provider, self.account.clone(), self.workspace);
            self.oauth_command(ctx, client, name, McpOAuthCommand::Status);
            changed = true;
        }
        if self.workspace != client.workspace_id() {
            changed = true;
            self.workspace = client.workspace_id();
            self.loaded = false;
        }
        if !self.loaded && self.pending.is_none() {
            self.refresh(ctx, client.clone());
            changed = true;
        }
        changed
    }
}

use crate::{app::ChatApp,
            ui::*};
use gpui::{AnyElement,
           InteractiveElement as _,
           IntoElement,
           ParentElement,
           Styled,
           Window,
           base::TestSupportExt as _};
impl ChatApp {
    fn mcp_scope_changed(&mut self) {
        self.mcp.loaded = false;
        self.mcp.oauth.clear();
        self.wake.notify();
    }

    pub(super) fn mcp_panel(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> AnyElement {
        let enabled = self.mcp.pending.is_none();
        let mut target = column()
            .child(
                row().w_full().justify_end().child(
                    self.button("mcp-refresh", "계정 새로고침", enabled, cx, |s, _, _| {
                        s.mcp.loaded = false;
                        s.wake.notify();
                    })
                    .ghost(),
                ),
            )
            .child(note(
                self.text("OAuth 인증은 제공자·계정별로 관리합니다. MCP 서버 등록은 모든 제공자가 공유합니다."),
                cx,
            ));
        use gpui::component::tab::{Tab,
                                   TabBar};
        let selected = gpui_agent_contracts::provider_index(self.mcp.provider);
        let tabs = TabBar::new("mcp-provider-tabs")
            .underline()
            .w_full()
            .selected_index(selected)
            .children(PROVIDERS.into_iter().map(|p| Tab::new().label(display_name(p)).disabled(!enabled)))
            .on_click(cx.listener(|s, index, _, cx| {
                let provider = PROVIDERS[*index];
                if s.mcp.pending.is_none() && s.mcp.provider != provider {
                    s.mcp.provider = provider;
                    s.mcp.account = "default".into();
                    s.mcp_scope_changed();
                    cx.notify();
                }
            }));
        let providers = gpui::div().id("mcp-provider-group").test_support().w_full().child(tabs);
        target = target
            .child(providers)
            .child(note(self.text("인증 대상을 바꿔도 공통 MCP 설정은 유지됩니다."), cx));
        let mut accounts = row();
        if let [account] = self.mcp.accounts.as_slice() {
            let label = if account.id == "default" {
                self.text("기본 계정")
            } else {
                account.label.clone()
            };
            accounts = accounts.child(gpui::div().id("mcp-account-current").test_support().child(label));
        } else {
            for a in &self.mcp.accounts {
                let id = a.id.clone();
                let label = if id == "default" { self.text("기본 계정") } else { a.label.clone() };
                accounts = accounts.child(
                    self.choice(format!("mcp-account-{id}"), &label, self.mcp.account == id, enabled, cx, move |s, _, _| {
                        if s.mcp.account != id {
                            s.mcp.account = id.clone();
                            s.mcp_scope_changed();
                        }
                    }),
                );
            }
        }
        target = target.child(note(self.text("계정"), cx)).child(accounts);
        let scoped = self.client.for_account(self.mcp.provider, self.mcp.account.clone(), self.mcp.workspace);
        let mut page = column().gap_6();
        if let Some(message) = &self.mcp.error {
            page = page.child(error(message.clone(), cx));
        }
        let mut body = column();
        body = body
            .child(note(
                self.text("한 번 등록하면 Claude, ChatGPT/Codex, Gemini/Antigravity가 공통으로 사용합니다. 대화 헤더의 체크박스에서 사용할 MCP를 선택하세요. 인증은 서버 호스트의 환경변수 이름 또는 OAuth로 설정합니다."),
                cx,
            ))
            .child(self.field(
                "mcp-common",
                "MCP JSON",
                self.mcp.json.clone(),
                FieldKind::Code,
                enabled,
                window,
                cx,
                |s, v| s.mcp.json = v,
            ));
        let parsed = serde_json::from_str::<Vec<McpServer>>(&self.mcp.json);
        if let Err(message) = &parsed {
            body = body.child(error(message.to_string(), cx).id("mcp-json-error").test_support());
        }
        let c = scoped.clone();
        let p = self.mcp.provider;
        body = body.child(self.button("mcp-load", "MCP 불러오기", enabled, cx, move |s, _, _| {
            let c = c.clone();
            s.mcp
                .run(&s.wake, move || oauth::loaded(&c, p, c.response(Method::GET, "/v1/mcp", None)?.json()?));
        }));
        let c = scoped.clone();
        body = body.child(self.button(
            "mcp-save",
            "MCP 저장",
            enabled && self.mcp.loaded_servers && parsed.is_ok(),
            cx,
            move |s, _, _| match serde_json::from_str::<Vec<McpServer>>(&s.mcp.json) {
                | Ok(servers) => {
                    let c = c.clone();
                    s.mcp.run(&s.wake, move || {
                        oauth::loaded(&c, p, c.response(Method::PUT, "/v1/mcp", Some(serde_json::to_value(servers)?))?.json()?)
                    });
                },
                | Err(e) => s.mcp.error = Some(e.to_string()),
            },
        ));

        let mut page = page.child(self.settings_section(self.text("공통 MCP stdio · HTTP 서버 설정"), body, cx));
        if self.mcp.servers.iter().chain(&self.mcp.plugin_servers).any(|server| {
            matches!(
                server,
                McpServer::Http {
                    oauth: Some(_),
                    ..
                }
            )
        }) {
            page = page.child(self.settings_section(self.text("OAuth 인증 계정"), target, cx));
        }
        self.mcp_oauth_panel(page, scoped, cx).into_any_element()
    }
}
