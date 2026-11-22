use anyhow::Result;
use gpui_agent_application::{Application,
                             ports::McpSessionAuthorizer};
use gpui_agent_contracts::{McpGrant,
                           McpScope,
                           Provider};
use gpui_agent_infrastructure::{History,
                                McpSessions};
use std::{sync::{Arc,
                 atomic::{AtomicU64,
                          Ordering}},
          time::{Duration,
                 Instant}};

struct Observed {
    sessions: Arc<McpSessions>,
    authenticated: tokio::sync::Notify,
}
impl McpSessionAuthorizer for Observed {
    fn resolve(&self, token: &str) -> Result<Option<McpGrant>> {
        let grant = self.sessions.resolve(token)?;
        self.authenticated.notify_one();
        Ok(grant)
    }

    fn revoke_all(&self) { self.sessions.revoke_all(); }
}

#[tokio::test]
async fn expiry_and_revocation_during_upload_are_checked_again_before_invocation() -> Result<()> {
    for expire in [false, true] {
        let root = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/mcp", listener.local_addr()?);
        let elapsed = Arc::new(AtomicU64::new(0));
        let clock = elapsed.clone();
        let start = Instant::now();
        let sessions = Arc::new(McpSessions::with_clock(
            url.clone(),
            Duration::from_secs(60),
            Arc::new(move || start + Duration::from_secs(clock.load(Ordering::SeqCst))),
        )?);
        let observed = Arc::new(Observed {
            sessions: sessions.clone(),
            authenticated: tokio::sync::Notify::new(),
        });
        let lease = sessions.issue(
            McpScope {
                provider: Provider::Codex,
                account_id: "default".into(),
                workspace_path: root.path().display().to_string(),
            },
            false,
        )?;
        let application = Application::new(
            std::array::from_fn(|_| Err(anyhow::anyhow!("no provider required"))),
            Arc::new(History::open(&root.path().join("history.sqlite3"), root.path())?),
        );
        let api = gpui_agent_api::Api::new(application, "operatorcredential12345678901234567890".into())?.with_mcp_sessions(observed.clone());
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let host = tokio::spawn(gpui_agent_api::serve(listener, api, async {
            let _ = stopped.await;
        }));
        let (body, chunks) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(1);
        let request = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?
            .post(&url)
            .bearer_auth(&lease.connection().token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(reqwest::Body::wrap_stream(tokio_stream::wrappers::ReceiverStream::new(chunks)));
        let response = tokio::spawn(request.send());
        tokio::time::timeout(Duration::from_secs(5), observed.authenticated.notified()).await?;
        if expire {
            elapsed.store(60, Ordering::SeqCst);
        } else {
            sessions.revoke_all();
        }
        body.send(Ok(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec())).await?;
        drop(body);
        assert_eq!(response.await??.status(), reqwest::StatusCode::UNAUTHORIZED);
        let _ = shutdown.send(());
        host.await??;
    }
    Ok(())
}
