use crate::Config;
use anyhow::{Result,
             ensure};
use gpui_agent_domain::McpServer;
use gpui_agent_ports::McpConnection;

pub const NAME: &str = "gpui-agent-session";
pub const TOKEN: &str = "GPUI_AGENT_MCP_TOKEN";
pub const URL: &str = "GPUI_AGENT_MCP_URL";
pub fn oauth_variable(name: &str) -> String {
    use sha2::{Digest,
               Sha256};
    format!("GPUI_AGENT_MCP_OAUTH_{:x}", Sha256::digest(name.as_bytes()))
}

pub fn http_headers(bearer: Option<&str>, variables: &std::collections::BTreeMap<String, String>) -> Result<std::collections::BTreeMap<String, String>> {
    let mut headers = std::collections::BTreeMap::new();
    for (header, variable) in variables
        .iter()
        .map(|(header, variable)| (header.as_str(), variable.as_str()))
        .chain(bearer.map(|variable| ("Authorization", variable)))
    {
        let value = std::env::var(variable).map_err(|_| anyhow::anyhow!("MCP HTTP environment variable {variable} is missing or not Unicode"))?;
        ensure!(
            gpui_agent_domain::valid_mcp_header_value(&value),
            "MCP HTTP environment variable {variable} contains an empty or unsupported header value"
        );
        let value = if header == "Authorization" && bearer == Some(variable) {
            format!("Bearer {value}")
        } else {
            value
        };
        headers.insert(header.to_owned(), value);
    }
    Ok(headers)
}

pub fn servers(config: &Config, connection: Option<&McpConnection>) -> Result<Vec<McpServer>> {
    let mut servers = config.mcp()?;
    if connection.is_some() {
        ensure!(
            !servers.iter().any(|server| server.name() == NAME),
            "gpui-agent-session is reserved for scoped session tools"
        );
        let command = config
            .browser_helper
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("A host MCP bridge executable is required"))?;
        servers.push(McpServer::Stdio {
            name: NAME.into(),
            command: command.to_string_lossy().into_owned(),
            args: vec!["--mcp-stdio".into()],
        });
    }
    Ok(servers)
}

pub fn environment(command: &mut tokio::process::Command, connection: Option<&McpConnection>) {
    if let Some(connection) = connection {
        command.env(TOKEN, &connection.token).env(URL, &connection.url);
    }
}
