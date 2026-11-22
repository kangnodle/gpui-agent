use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}
impl McpOAuthConfig {
    pub fn requested_scopes(&self, challenge: Option<&str>, fallback: &str) -> Result<String, String> {
        let mut scopes: std::collections::BTreeSet<String> = self.scopes.iter().cloned().collect();
        if let Some(challenge) = challenge {
            scopes.extend(challenge.split(' ').filter(|scope| !scope.is_empty()).map(str::to_owned));
        } else if scopes.is_empty() {
            scopes.extend(fallback.split(' ').filter(|scope| !scope.is_empty()).map(str::to_owned));
        }
        let scopes: Vec<_> = scopes.into_iter().collect();
        Self {
            client_id: None,
            scopes: scopes.clone(),
        }
        .validate()?;
        Ok(scopes.join(" "))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self
            .client_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 2048 || id.chars().any(char::is_control))
            || self.scopes.len() > 32
            || self
                .scopes
                .iter()
                .any(|scope| scope.is_empty() || scope.len() > 256 || !scope.bytes().all(|b| matches!(b, 0x21 | 0x23..=0x5b | 0x5d..=0x7e)))
        {
            return Err("OAuth requires a bounded public client ID and at most 32 scope tokens".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthCommand {
    Status,
    Login,
    Cancel,
    Logout,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthState {
    SignedOut,
    Pending,
    Connected,
    Expired,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpOAuthStatus {
    pub state: McpOAuthState,
    pub authorization_url: Option<String>,
    pub expires_at: Option<u64>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpOAuthMachine {
    SignedOut,
    Pending { authorization_url: String },
    Connected { expires_at: Option<u64> },
    Expired { message: String },
    Cancelled { message: Option<String> },
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpOAuthEvent {
    StartLogin { authorization_url: String },
    CallbackAuthorized { expires_at: Option<u64> },
    CallbackDenied,
    TokenExpired { message: String },
    Cancel,
    Logout,
    Fail { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpOAuthEffect {
    OpenAuthorizationUrl(String),
    ClearStoredTokens,
}

impl McpOAuthMachine {
    pub fn to_status(&self) -> McpOAuthStatus {
        match self {
            | Self::SignedOut => McpOAuthStatus {
                state: McpOAuthState::SignedOut,
                authorization_url: None,
                expires_at: None,
                message: None,
            },
            | Self::Pending {
                authorization_url,
            } => McpOAuthStatus {
                state: McpOAuthState::Pending,
                authorization_url: Some(authorization_url.clone()),
                expires_at: None,
                message: None,
            },
            | Self::Connected {
                expires_at,
            } => McpOAuthStatus {
                state: McpOAuthState::Connected,
                authorization_url: None,
                expires_at: *expires_at,
                message: None,
            },
            | Self::Expired {
                message,
            } => McpOAuthStatus {
                state: McpOAuthState::Expired,
                authorization_url: None,
                expires_at: None,
                message: Some(message.clone()),
            },
            | Self::Cancelled {
                message,
            } => McpOAuthStatus {
                state: McpOAuthState::Cancelled,
                authorization_url: None,
                expires_at: None,
                message: message.clone(),
            },
            | Self::Failed {
                message,
            } => McpOAuthStatus {
                state: McpOAuthState::Failed,
                authorization_url: None,
                expires_at: None,
                message: Some(message.clone()),
            },
        }
    }
}

pub fn transition_mcp_oauth(state: McpOAuthMachine, event: McpOAuthEvent) -> crate::Transition<McpOAuthMachine, McpOAuthEffect> {
    use crate::Transition;

    match (state, event) {
        | (
            _,
            McpOAuthEvent::StartLogin {
                authorization_url,
            },
        ) => Transition::with_effect(
            McpOAuthMachine::Pending {
                authorization_url: authorization_url.clone(),
            },
            McpOAuthEffect::OpenAuthorizationUrl(authorization_url),
        ),
        | (
            McpOAuthMachine::Pending {
                ..
            },
            McpOAuthEvent::CallbackAuthorized {
                expires_at,
            },
        ) => Transition::pure(McpOAuthMachine::Connected {
            expires_at,
        }),
        | (
            McpOAuthMachine::Pending {
                ..
            },
            McpOAuthEvent::CallbackDenied,
        ) => Transition::pure(McpOAuthMachine::Cancelled {
            message: Some("Authorization was declined".into()),
        }),
        | (
            _,
            McpOAuthEvent::TokenExpired {
                message,
            },
        ) => Transition::pure(McpOAuthMachine::Expired {
            message,
        }),
        | (_, McpOAuthEvent::Cancel) => Transition::pure(McpOAuthMachine::Cancelled {
            message: None,
        }),
        | (_, McpOAuthEvent::Logout) => Transition::with_effect(McpOAuthMachine::SignedOut, McpOAuthEffect::ClearStoredTokens),
        | (
            _,
            McpOAuthEvent::Fail {
                message,
            },
        ) => Transition::pure(McpOAuthMachine::Failed {
            message,
        }),
        | (other, _) => Transition::pure(other),
    }
}
pub fn mcp_oauth_refresh_due(expires_at: Option<u64>, now: u64, has_refresh: bool) -> Result<bool, String> {
    if expires_at.is_some_and(|at| at <= now) && !has_refresh {
        return Err("MCP OAuth session expired; log in again".into());
    }
    Ok(has_refresh && expires_at.is_some_and(|at| at <= now.saturating_add(30)))
}
pub fn mcp_oauth_loopback(url: &url::Url) -> bool {
    match url.host() {
        | Some(url::Host::Domain("localhost")) => true,
        | Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        | Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        | _ => false,
    }
}
pub fn mcp_oauth_endpoint(value: &str, resource: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(value).map_err(|_| "Invalid OAuth endpoint URL")?;
    let resource = url::Url::parse(resource).map_err(|_| "Invalid OAuth resource URL")?;
    let local = mcp_oauth_loopback(&resource);
    if parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.len() > 4096
        || !(parsed.scheme() == "https" || (parsed.scheme() == "http" && local && mcp_oauth_loopback(&parsed)))
        || (!local && mcp_oauth_loopback(&parsed))
    {
        return Err("Unsafe OAuth endpoint URL".into());
    }
    Ok(parsed)
}
pub fn mcp_oauth_callback(request: &str, redirect: &str, state: &str, issuer: &str, issuer_required: bool) -> Result<Option<String>, String> {
    let mut parts = request.lines().next().ok_or("Missing callback")?.split_whitespace();
    if parts.next() != Some("GET") {
        return Err("Invalid callback method".into());
    }
    let target = parts.next().ok_or("Missing callback target")?;
    let expected = url::Url::parse(redirect).map_err(|_| "Invalid redirect URI")?;
    let host = format!("127.0.0.1:{}", expected.port().ok_or("Missing callback port")?);
    let hosts: Vec<_> = request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts != [host.as_str()] || !target.starts_with('/') || target.starts_with("//") {
        return Err("Invalid callback host or target".into());
    }
    let received = expected.join(target).map_err(|_| "Invalid callback target")?;
    if received.path() != expected.path() || received.fragment().is_some() {
        return Err("Invalid callback path".into());
    }
    let mut query = std::collections::HashMap::new();
    for (key, value) in received.query_pairs() {
        if query.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err("Duplicate callback field".into());
        }
    }
    if query.get("state").is_none_or(|value| value != state)
        || query.get("iss").is_some_and(|value| value != issuer)
        || (issuer_required && !query.contains_key("iss"))
    {
        return Err("Invalid OAuth state or issuer".into());
    }
    if query.contains_key("error") {
        return Ok(None);
    }
    Ok(Some(
        query
            .remove("code")
            .filter(|value| !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control))
            .ok_or("Missing authorization code")?,
    ))
}

pub fn mcp_bearer_challenge(header: &str) -> std::collections::BTreeMap<String, String> {
    let mut result = std::collections::BTreeMap::new();
    let Some((scheme, mut rest)) = header.split_once(' ') else {
        return result;
    };
    if !scheme.eq_ignore_ascii_case("bearer") || header.len() > 8192 {
        return result;
    }
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let Some((key, value)) = rest.split_once('=') else {
            return Default::default();
        };
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Default::default();
        }
        let mut parsed = String::new();
        if let Some(quoted) = value.strip_prefix('"') {
            let mut escaped = false;
            let mut end = None;
            for (i, c) in quoted.char_indices() {
                if escaped {
                    parsed.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    end = Some(i + 1);
                    break;
                } else {
                    parsed.push(c);
                }
            }
            let Some(end) = end else {
                return Default::default();
            };
            rest = &quoted[end ..];
        } else {
            let end = value.find(',').unwrap_or(value.len());
            parsed = value[.. end].trim().into();
            rest = &value[end ..];
        }
        if result.insert(key, parsed).is_some() {
            return Default::default();
        }
        rest = rest.trim_start();
        if !rest.is_empty() {
            let Some(next) = rest.strip_prefix(',') else {
                return Default::default();
            };
            rest = next;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn given_token_expiry_then_only_expired_tokens_require_login_and_refresh_is_early_when_available() {
        assert_eq!(super::mcp_oauth_refresh_due(Some(120), 100, false), Ok(false));
        assert_eq!(super::mcp_oauth_refresh_due(Some(120), 100, true), Ok(true));
        assert!(super::mcp_oauth_refresh_due(Some(100), 100, false).is_err());
        assert_eq!(super::mcp_oauth_refresh_due(Some(200), 100, true), Ok(false));
        assert_eq!(super::mcp_oauth_refresh_due(None, 100, true), Ok(false));
    }
    #[test]
    fn given_oauth_metadata_and_callbacks_then_policy_requires_safe_endpoints_unique_state_and_the_advertised_issuer() {
        use super::*;
        assert!(mcp_oauth_endpoint("http://127.0.0.1/token", "https://mcp.example.invalid/mcp").is_err());
        assert!(mcp_oauth_endpoint("https://user:secret@auth.example.invalid/token", "https://mcp.example.invalid/mcp").is_err());
        let redirect = "http://127.0.0.1:34567/oauth/callback/random";
        let request = "GET /oauth/callback/random?state=expected&code=approved HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n";
        assert_eq!(
            mcp_oauth_callback(request, redirect, "expected", "https://auth.example.invalid", false)
                .unwrap()
                .as_deref(),
            Some("approved")
        );
        assert!(mcp_oauth_callback(request, redirect, "expected", "https://auth.example.invalid", true).is_err());
        assert!(mcp_oauth_callback(request, redirect, "wrong", "https://auth.example.invalid", false).is_err());
        assert!(
            mcp_oauth_callback(
                &request.replace("state=expected", "state=expected&state=other"),
                redirect,
                "expected",
                "https://auth.example.invalid",
                false
            )
            .is_err()
        );
        assert!(
            mcp_oauth_callback(
                &request.replace("Host: 127.0.0.1", "Host: evil.invalid"),
                redirect,
                "expected",
                "https://auth.example.invalid",
                false
            )
            .is_err()
        );
    }
    #[test]
    fn given_a_bearer_challenge_then_quoted_commas_and_scope_are_parsed_without_accepting_duplicate_parameters() {
        let params = super::mcp_bearer_challenge("Bearer resource_metadata=\"https://example.invalid/meta?a=b,c\", scope=\"files:read files:write\"");
        assert_eq!(params["resource_metadata"], "https://example.invalid/meta?a=b,c");
        assert_eq!(params["scope"], "files:read files:write");
        assert!(super::mcp_bearer_challenge("Basic realm=\"test\"").is_empty());
        assert!(super::mcp_bearer_challenge("Bearer scope=\"read\", scope=\"write\"").is_empty());
    }
}
