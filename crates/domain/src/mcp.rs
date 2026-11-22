use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpScope {
    pub provider: crate::Provider,
    pub account_id: String,
    pub workspace_path: String,
}
/// A host-bound MCP caller: its scope and, for a durable job, its conversation.
#[derive(Clone, Debug)]
pub struct TaskSession {
    pub scope: McpScope,
    pub conversation_id: Option<i64>,
}

impl McpScope {
    pub fn allows(&self, scope: &crate::JobScope) -> bool {
        self.provider == scope.provider && self.account_id == scope.account_id && self.workspace_path == scope.workspace_path
    }
}

#[derive(Clone, Debug)]
pub struct McpGrant {
    /// Host-issued execution identity; never a credential or a native session
    /// ID.
    pub execution_id: u64,
    pub job_id: Option<i64>,
    pub scope: McpScope,
    pub can_write: bool,
    pub conversation_id: Option<i64>,
}

/// Only host-generated correlation and bounded metadata belong in diagnostics.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename = "mcp_tool")]
pub struct McpDiagnostic {
    pub request_id: String,
    pub tool: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub job_id: Option<i64>,
    pub operation_id: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum McpServer {
    Stdio {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Http {
        name: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_env_var: Option<String>,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        env_http_headers: std::collections::BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        oauth: Option<crate::McpOAuthConfig>,
    },
}
impl McpServer {
    pub fn name(&self) -> &str {
        match self {
            | Self::Stdio {
                name, ..
            }
            | Self::Http {
                name, ..
            } => name,
        }
    }
}
/// Preserve legacy definitions without silently choosing a provider on
/// conflicts.
pub fn merge_mcp(sets: impl IntoIterator<Item = Vec<McpServer>>) -> Result<Vec<McpServer>, String> {
    let mut servers: Vec<McpServer> = vec![];
    for set in sets {
        validate_mcp(&set)?;
        for server in set {
            if let Some(existing) = servers.iter().find(|existing| existing.name() == server.name()) {
                if serde_json::to_value(existing).map_err(|e| e.to_string())? != serde_json::to_value(&server).map_err(|e| e.to_string())? {
                    return Err(format!(
                        "Conflicting legacy MCP server '{}'; rename it in the original profile files before loading common settings",
                        server.name()
                    ));
                }
            } else {
                servers.push(server);
            }
        }
    }
    validate_mcp(&servers)?;
    Ok(servers)
}
/// What an MCP HTTP header value read from the environment must look like. One
/// rule for the turn that uses it and the checks that predict it.
pub fn valid_mcp_header_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 8192 && value.bytes().all(|b| (0x20 ..= 0x7e).contains(&b)) && !value.contains("${")
}

pub fn validate_mcp(servers: &[McpServer]) -> Result<(), String> {
    if servers.len() > 16 {
        return Err("At most 16 MCP servers are allowed".into());
    }
    if serde_json::to_vec(servers).map_err(|e| e.to_string())?.len() > 256 * 1024 {
        return Err("MCP configuration exceeds 256 KiB".into());
    }
    let mut names = std::collections::HashSet::new();
    for server in servers {
        let name = server.name();
        if name.is_empty() || name.len() > 64 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') || !names.insert(name) {
            return Err("MCP names must be unique ASCII letters, digits, underscores or hyphens (1–64)".into());
        }
        match server {
            | McpServer::Stdio {
                command,
                args,
                ..
            } => {
                if !std::path::Path::new(command).is_absolute()
                    || command.len() > 4096
                    || command.contains(['\0', '\n', '\r'])
                    || args.len() > 64
                    || args.iter().any(|a| a.contains('\0') || a.len() > 4096)
                {
                    return Err("MCP requires an absolute executable path and at most 64 bounded arguments".into());
                }
            },
            | McpServer::Http {
                url,
                bearer_token_env_var,
                env_http_headers,
                oauth,
                ..
            } => {
                let parsed = url::Url::parse(url).map_err(|_| "Invalid MCP HTTP URL")?;
                let loopback = match parsed.host() {
                    | Some(url::Host::Domain("localhost")) => true,
                    | Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                    | Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                    | _ => false,
                };
                if url.len() > 4096
                    || url.chars().any(|c| c.is_whitespace() || c.is_control())
                    || url.contains(['$', '{', '}'])
                    || parsed.host().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || parsed.query().is_some()
                    || parsed.fragment().is_some()
                    || !(parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback))
                {
                    return Err("MCP requires HTTPS or loopback HTTP without URL credentials, query, fragment or variable expansion".into());
                }
                if bearer_token_env_var.as_ref().is_some_and(|name| !valid_mcp_env(name)) || env_http_headers.len() > 16 {
                    return Err("Invalid MCP authentication environment variable or header count".into());
                }
                let mut headers = std::collections::HashSet::new();
                if let Some(oauth) = oauth {
                    oauth.validate()?;
                    if bearer_token_env_var.is_some() {
                        return Err("OAuth and bearer environment authentication cannot be combined".into());
                    }
                }
                if bearer_token_env_var.is_some() || oauth.is_some() {
                    headers.insert("authorization".to_owned());
                }
                for (header, variable) in env_http_headers {
                    if header.is_empty()
                        || header.len() > 128
                        || !header.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                        || !headers.insert(header.to_ascii_lowercase())
                        || !valid_mcp_env(variable)
                    {
                        return Err("MCP headers require unique HTTP header names and valid environment variable references".into());
                    }
                }
            },
        }
    }
    Ok(())
}

fn valid_mcp_env(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.as_bytes()[0].is_ascii_digit()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !["GPUI_AGENT_MCP_TOKEN", "GPUI_AGENT_MCP_URL"]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
        && !name.to_ascii_uppercase().starts_with("GPUI_AGENT_MCP_OAUTH_")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn given_oauth_http_settings_then_public_client_configuration_round_trips_and_conflicting_credentials_are_rejected() {
        let config = serde_json::json!({"name":"remote","url":"https://example.invalid/mcp","oauth":{"client_id":"public-client","scopes":["read:tools"]}});
        let server: McpServer = serde_json::from_value(config.clone()).expect("OAuth HTTP settings must load");
        assert!(validate_mcp(std::slice::from_ref(&server)).is_ok());
        assert_eq!(serde_json::to_value(server).unwrap(), config);
        for extra in [
            serde_json::json!({"bearer_token_env_var":"TOKEN"}),
            serde_json::json!({"env_http_headers":{"authorization":"TOKEN"}}),
            serde_json::json!({"oauth":{"client_id":"","scopes":[]}}),
            serde_json::json!({"oauth":{"scopes":["bad scope"]}}),
        ] {
            let mut invalid = config.clone();
            invalid.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            assert!(
                serde_json::from_value::<McpServer>(invalid)
                    .map(|s| validate_mcp(&[s]).is_err())
                    .unwrap_or(true)
            );
        }
    }
    #[test]
    fn given_http_and_stdio_settings_when_loaded_then_the_wire_formats_round_trip() {
        for config in [
            serde_json::json!({"name":"remote","url":"https://mcp.example.invalid/tools","bearer_token_env_var":"MCP_ACCESS_TOKEN","env_http_headers":{"X-Tenant-Key":"MCP_TENANT_KEY"}}),
            serde_json::json!({"name":"local","url":"http://127.0.0.1:8080/mcp"}),
            serde_json::json!({"name":"local6","url":"http://[::1]:8080/mcp"}),
            serde_json::json!({"name":"stdio","command":std::env::current_exe().unwrap(),"args":["--stdio"]}),
        ] {
            let server: McpServer = serde_json::from_value(config.clone()).expect("Both MCP transports must load");
            assert!(validate_mcp(std::slice::from_ref(&server)).is_ok());
            assert_eq!(serde_json::to_value(server).unwrap(), config);
        }
    }
    #[test]
    fn given_remote_settings_when_validated_then_unsafe_urls_and_ambiguous_transports_are_rejected() {
        for url in [
            "http://example.invalid/mcp",
            "https://user:password@example.invalid/mcp",
            "https://example.invalid/mcp?token=secret",
            "https://example.invalid/mcp#fragment",
            "file:///tmp/server",
            "https://example.invalid/\npath",
            "https://${HOST}/mcp",
        ] {
            let server: McpServer = serde_json::from_value(serde_json::json!({"name":"remote","url":url})).unwrap();
            assert!(validate_mcp(&[server]).is_err(), "Unsafe MCP endpoint was accepted");
        }
        for extra in [
            serde_json::json!({"command":"/bin/server"}),
            serde_json::json!({"args":["--stdio"]}),
            serde_json::json!({"http_headers":{"Authorization":"raw-secret"}}),
            serde_json::json!({"bearer_token_env_var":""}),
            serde_json::json!({"bearer_token_env_var":"BAD-NAME"}),
            serde_json::json!({"bearer_token_env_var":"GPUI_AGENT_MCP_TOKEN"}),
            serde_json::json!({"env_http_headers":{"Bad Header":"MCP_KEY"}}),
            serde_json::json!({"env_http_headers":{"X-Key":"BAD\nNAME"}}),
            serde_json::json!({"env_http_headers":{"X-Key":"MCP_KEY","x-key":"MCP_OTHER"}}),
            serde_json::json!({"bearer_token_env_var":"MCP_TOKEN","env_http_headers":{"authorization":"OTHER_TOKEN"}}),
        ] {
            let mut config = serde_json::json!({"name":"remote","url":"https://example.invalid/mcp"});
            config.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            assert!(
                serde_json::from_value::<McpServer>(config)
                    .map(|server| validate_mcp(&[server]).is_err())
                    .unwrap_or(true)
            );
        }
    }
    #[test]
    fn names_and_paths_are_validated() {
        let mut s = McpServer::Stdio {
            name: "tools".into(),
            command: std::env::current_exe().unwrap().display().to_string(),
            args: vec![],
        };
        assert!(validate_mcp(&[s.clone()]).is_ok());
        assert!(validate_mcp(&[s.clone(), s.clone()]).is_err());
        if let McpServer::Stdio {
            name, ..
        } = &mut s
        {
            *name = "a.b".into();
        }
        assert!(validate_mcp(&[s]).is_err());
    }
}
