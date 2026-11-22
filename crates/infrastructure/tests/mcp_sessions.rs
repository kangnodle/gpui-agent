use anyhow::Result;
use gpui_agent_application::ports::McpSessionAuthorizer;
use gpui_agent_domain::{McpScope,
                        Provider};
use gpui_agent_infrastructure::McpSessions;
use std::{sync::{Arc,
                 atomic::{AtomicU64,
                          Ordering}},
          time::{Duration,
                 Instant}};

#[test]
fn session_credentials_are_unique_scoped_expiring_and_revocable() -> Result<()> {
    let elapsed = Arc::new(AtomicU64::new(0));
    let base = Instant::now();
    let clock = elapsed.clone();
    let sessions = McpSessions::with_clock(
        "http://127.0.0.1:12345/mcp".into(),
        Duration::from_secs(60),
        Arc::new(move || base + Duration::from_secs(clock.load(Ordering::SeqCst))),
    )?;
    let scope = McpScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_path: "workspace".into(),
    };
    let first = sessions.issue(scope.clone(), false)?;
    let second = sessions.issue(scope.clone(), true)?;
    let token = first.connection().token.clone();
    assert_ne!(token, second.connection().token);
    assert!(!format!("{:?}", first.connection()).contains(&token));
    let grant = sessions.resolve(&token)?.expect("active credential");
    assert_eq!(grant.scope, scope);
    assert!(!grant.can_write);
    assert!(sessions.resolve(&second.connection().token)?.expect("write credential").can_write);
    elapsed.store(60, Ordering::SeqCst);
    assert!(sessions.resolve(&token)?.is_none(), "expiry is inclusive");
    let resumed = sessions.issue(scope, false)?;
    let resumed_token = resumed.connection().token.clone();
    drop(resumed);
    assert!(sessions.resolve(&resumed_token)?.is_none(), "dropping a session revokes it");
    sessions.revoke_all();
    assert!(sessions.resolve(&second.connection().token)?.is_none());
    assert!(sessions.issue(grant.scope, false).is_err(), "shutdown closes credential admission");
    Ok(())
}
