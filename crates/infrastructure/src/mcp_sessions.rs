//! Process-local credentials: monotonic expiry, hashed lookup, revocation on
//! drop.
use anyhow::{Result,
             ensure};
use gpui_agent_application::ports::{McpConnection,
                                    McpSessionAuthorizer,
                                    McpSessionIssuer,
                                    McpSessionLease};
use gpui_agent_domain::{McpGrant,
                        McpScope};
use sha2::{Digest,
           Sha256};
use std::{collections::HashMap,
          sync::{Arc,
                 Mutex,
                 Weak},
          time::{Duration,
                 Instant}};

struct Entry {
    grant: McpGrant,
    expires: Instant,
}
#[derive(Default)]
struct State {
    entries: HashMap<Vec<u8>, Entry>,
    closed: bool,
}

pub struct McpSessions {
    state: Arc<Mutex<State>>,
    url: String,
    ttl: Duration,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}

pub struct McpSession {
    connection: McpConnection,
    key: Vec<u8>,
    state: Weak<Mutex<State>>,
}
impl McpSession {
    pub fn connection(&self) -> &McpConnection { &self.connection }
}
impl McpSessionLease for McpSession {
    fn connection(&self) -> &McpConnection { &self.connection }
}

struct Issuer {
    sessions: Arc<McpSessions>,
    scope: McpScope,
    can_write: bool,
}
impl McpSessionIssuer for Issuer {
    fn issue_for_job(&self, conversation: Option<i64>, job: Option<i64>) -> Result<Box<dyn McpSessionLease>> {
        Ok(Box::new(self.sessions.issue_for_job(self.scope.clone(), self.can_write, conversation, job)?))
    }

    fn issue(&self) -> Result<Box<dyn McpSessionLease>> { Ok(Box::new(self.sessions.issue(self.scope.clone(), self.can_write)?)) }

    fn issue_in_conversation(&self, conversation: Option<i64>) -> Result<Box<dyn McpSessionLease>> {
        Ok(Box::new(self.sessions.issue_in_conversation(
            self.scope.clone(),
            self.can_write,
            conversation,
        )?))
    }
}
impl Drop for McpSession {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            match state.lock() {
                | Ok(mut state) => {
                    state.entries.remove(&self.key);
                },
                | Err(error) => {
                    let mut state = error.into_inner();
                    state.entries.clear();
                    state.closed = true;
                },
            }
        }
    }
}

impl McpSessions {
    pub fn for_scope(self: &Arc<Self>, scope: McpScope, can_write: bool) -> Arc<dyn McpSessionIssuer> {
        Arc::new(Issuer {
            sessions: self.clone(),
            scope,
            can_write,
        })
    }

    pub fn new(url: String) -> Result<Self> { Self::with_clock(url, Duration::from_secs(900), Arc::new(Instant::now)) }

    pub fn with_clock(url: String, ttl: Duration, clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> Result<Self> {
        let parsed = reqwest::Url::parse(&url)?;
        ensure!(
            parsed.scheme() == "http"
                && parsed
                    .host_str()
                    .is_some_and(|host| host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()))
                && parsed.port().is_some()
                && parsed.path() == "/mcp"
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none(),
            "MCP endpoint must be an explicit loopback /mcp URL"
        );
        ensure!(
            !ttl.is_zero() && ttl <= Duration::from_secs(900),
            "MCP lifetime must be between zero and 900 seconds"
        );
        Ok(Self {
            state: Arc::new(Mutex::new(State::default())),
            url,
            ttl,
            clock,
        })
    }

    pub fn issue(&self, scope: McpScope, can_write: bool) -> Result<McpSession> { self.issue_in_conversation(scope, can_write, None) }

    pub fn issue_in_conversation(&self, scope: McpScope, can_write: bool, conversation_id: Option<i64>) -> Result<McpSession> {
        self.issue_for_job(scope, can_write, conversation_id, None)
    }

    pub fn issue_for_job(&self, scope: McpScope, can_write: bool, conversation_id: Option<i64>, job_id: Option<i64>) -> Result<McpSession> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        ensure!(
            job_id.is_none_or(|id| id > 0) && (job_id.is_none() || conversation_id.is_some()),
            "Job credentials require a positive job and conversation ID"
        );
        ensure!(conversation_id.is_none_or(|id| id > 0), "Conversation ID must be positive");
        let now = (self.clock)();
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("MCP session registry is unavailable"))?;
        ensure!(!state.closed, "MCP session registry is stopped");
        state.entries.retain(|_, entry| entry.expires > now);
        let token = format!("session_{}", crate::new_token()?);
        let key = Sha256::digest(token.as_bytes()).to_vec();
        state.entries.insert(
            key.clone(),
            Entry {
                grant: McpGrant {
                    job_id,
                    execution_id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    scope,
                    can_write,
                    conversation_id,
                },
                expires: now + self.ttl,
            },
        );
        Ok(McpSession {
            connection: McpConnection {
                url: self.url.clone(),
                token,
            },
            key,
            state: Arc::downgrade(&self.state),
        })
    }

    pub fn active_sessions(&self) -> Result<usize> {
        let now = (self.clock)();
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("MCP session registry is unavailable"))?;
        state.entries.retain(|_, entry| entry.expires > now);
        Ok(state.entries.len())
    }
}

impl McpSessionAuthorizer for McpSessions {
    fn resolve(&self, token: &str) -> Result<Option<McpGrant>> {
        let now = (self.clock)();
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("MCP session registry is unavailable"))?;
        state.entries.retain(|_, entry| entry.expires > now);
        Ok(state.entries.get(Sha256::digest(token.as_bytes()).as_slice()).map(|entry| entry.grant.clone()))
    }

    fn revoke_all(&self) {
        let mut state = match self.state.lock() {
            | Ok(state) => state,
            | Err(error) => error.into_inner(),
        };
        state.closed = true;
        state.entries.clear();
    }
}
