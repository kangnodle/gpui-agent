use super::*;
use gpui::ParentElement;

pub(super) fn loaded(client: &ApiClient, provider: Provider, servers: Vec<McpServer>) -> anyhow::Result<Change> {
    let mut statuses = BTreeMap::new();
    for server in &servers {
        if matches!(
            server,
            McpServer::Http {
                oauth: Some(_),
                ..
            }
        ) {
            statuses.insert(
                server.name().to_owned(),
                client
                    .response(Method::GET, &format!("/v1/mcp/{}/{}/oauth", provider_name(provider), server.name()), None)?
                    .json()?,
            );
        }
    }
    Ok(Change::Mcp(servers, statuses))
}
impl McpUi {
    pub(super) fn oauth_command(&mut self, wake: &Wake, client: ApiClient, name: String, command: McpOAuthCommand) {
        let provider = self.provider;
        let account = self.account.clone();
        self.run(wake, move || {
            let path = format!("/v1/mcp/{}/{name}/oauth", provider_name(provider));
            let status = if command == McpOAuthCommand::Status {
                client.response(Method::GET, &path, None)?
            } else {
                client.response(Method::POST, &path, Some(serde_json::to_value(command)?))?
            }
            .json()?;
            Ok(Change::OAuth(provider, account, name, status, command == McpOAuthCommand::Login))
        });
    }
}
impl ChatApp {
    pub(super) fn mcp_oauth_panel(&mut self, mut page: gpui::Div, client: ApiClient, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let enabled = self.mcp.pending.is_none();
        let servers: Vec<_> = self.mcp.servers.iter().chain(&self.mcp.plugin_servers).cloned().collect();
        for server in servers {
            let McpServer::Http {
                name,
                url: resource,
                oauth: Some(_),
                ..
            } = server
            else {
                continue;
            };
            let state = self.mcp.oauth.get(&name).map(|s| s.state).unwrap_or(McpOAuthState::SignedOut);
            let label = match self.mcp.oauth.get(&name).map(|status| status.state) {
                | None => "인증 상태 확인 필요",
                | Some(McpOAuthState::SignedOut) => "로그인 필요",
                | Some(McpOAuthState::Pending) => "브라우저 승인 대기",
                | Some(McpOAuthState::Connected) => "연결됨",
                | Some(McpOAuthState::Expired) => "인증 만료 · 다시 로그인",
                | Some(McpOAuthState::Cancelled) => "로그인 취소됨",
                | Some(McpOAuthState::Failed) => "로그인 실패 · 다시 시도",
            };
            let mut body = column().gap_3().child(note(self.text(label), cx));
            if let Some(message) = self.mcp.oauth.get(&name).and_then(|s| s.message.clone()) {
                body = body.child(note(message, cx));
            }
            let mut actions = row();
            let commands = if state == McpOAuthState::Pending {
                vec![("cancel", "로그인 취소", McpOAuthCommand::Cancel)]
            } else {
                vec![
                    ("status", "인증 상태 확인", McpOAuthCommand::Status),
                    ("login", "OAuth 로그인", McpOAuthCommand::Login),
                    ("logout", "로그아웃", McpOAuthCommand::Logout),
                ]
            };
            for (id, label, command) in commands {
                let target = name.clone();
                let scoped = client.clone();
                actions = actions.child(self.button(format!("mcp-oauth-{id}-{name}"), label, enabled, cx, move |s, _, _| {
                    s.mcp.oauth_command(&s.wake, scoped.clone(), target.clone(), command)
                }));
            }
            if let Some(url) = self
                .mcp
                .oauth
                .get(&name)
                .and_then(|s| s.authorization_url.clone())
                .filter(|url| mcp_oauth_endpoint(url, &resource).is_ok())
            {
                actions = actions.child(self.button(
                    format!("mcp-oauth-browser-{name}"),
                    "로그인 페이지 열기",
                    enabled,
                    cx,
                    move |_, _, cx| cx.open_url(&url),
                ));
            }
            body = body.child(actions).child(note(
                self.text("저장한 서버에 로그인합니다. 원격 API 사용 시 브라우저의 콜백 주소가 API 호스트에 연결되어야 합니다."),
                cx,
            ));
            page = page.child(self.settings_section(format!("OAuth · {name}"), body, cx));
        }
        page
    }
}
