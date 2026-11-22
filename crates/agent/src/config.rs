use gpui_agent_domain::Provider;
use std::{path::PathBuf,
          time::Duration};

#[derive(Clone, Debug)]
pub struct Config {
    pub provider: Provider,
    /// Executable or PATH name, never a shell command.
    pub executable: PathBuf,
    /// Provider-owned credential/config directory, separate from the user's
    /// existing CLI profile.
    pub home: PathBuf,
    /// Application-wide MCP catalog; None preserves standalone native profiles.
    pub mcp_home: Option<PathBuf>,
    pub cwd: PathBuf,
    /// Required for Antigravity; the native localharness_external executable.
    pub harness: Option<PathBuf>,
    /// Host executable implementing the browser interceptor and MCP stdio
    /// bridge.
    pub browser_helper: Option<PathBuf>,
    pub timeout: Duration,
    pub plugin_mcp: Vec<gpui_agent_domain::McpServer>,
    pub mcp_selection: Option<Vec<String>>,
}

/// Platform-specific default executable or PATH name; never a shell command.
pub fn default_executable(provider: Provider) -> &'static str {
    match provider {
        | Provider::Claude => "claude",
        | Provider::Codex => "codex",
        | Provider::Antigravity if cfg!(windows) => "agy_acp_server.exe",
        | Provider::Antigravity => "agy_acp_server.par",
    }
}

impl Config {
    pub(crate) fn mcp(&self) -> anyhow::Result<Vec<gpui_agent_domain::McpServer>> {
        if let Some(home) = &self.mcp_home {
            let mut servers = load_mcp_config(home)?;
            servers.extend(self.plugin_mcp.clone());
            gpui_agent_domain::validate_mcp(&servers).map_err(anyhow::Error::msg)?;
            return gpui_agent_domain::select_mcp_servers(servers, self.mcp_selection.as_deref()).map_err(anyhow::Error::msg);
        }
        let path = self.home.join("mcp.json");
        let file = match std::fs::File::open(path) {
            | Ok(file) => file,
            | Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                gpui_agent_domain::validate_mcp(&self.plugin_mcp).map_err(anyhow::Error::msg)?;
                return gpui_agent_domain::select_mcp_servers(self.plugin_mcp.clone(), self.mcp_selection.as_deref()).map_err(anyhow::Error::msg);
            },
            | Err(e) => return Err(e.into()),
        };
        anyhow::ensure!(file.metadata()?.len() <= 300000, "MCP configuration is too large");
        let mut servers: Vec<gpui_agent_domain::McpServer> = serde_json::from_reader(file)?;
        servers.extend(self.plugin_mcp.clone());
        gpui_agent_domain::validate_mcp(&servers).map_err(anyhow::Error::msg)?;
        gpui_agent_domain::select_mcp_servers(servers, self.mcp_selection.as_deref()).map_err(anyhow::Error::msg)
    }
}

pub fn load_mcp_config(home: &std::path::Path) -> anyhow::Result<Vec<gpui_agent_domain::McpServer>> {
    fn read(path: &std::path::Path) -> anyhow::Result<Option<Vec<gpui_agent_domain::McpServer>>> {
        let file = match std::fs::File::open(path) {
            | Ok(file) => file,
            | Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            | Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(file.metadata()?.len() <= 300000, "MCP configuration is too large");
        let servers: Vec<gpui_agent_domain::McpServer> = serde_json::from_reader(file)?;
        gpui_agent_domain::validate_mcp(&servers).map_err(anyhow::Error::msg)?;
        Ok(Some(servers))
    }
    if let Some(servers) = read(&home.join("mcp.json"))? {
        return Ok(servers);
    }
    let mut homes = vec![home.to_path_buf()];
    match std::fs::read_dir(home.join("accounts")) {
        | Ok(entries) => {
            let entries = entries.take(257).collect::<std::io::Result<Vec<_>>>()?;
            anyhow::ensure!(entries.len() <= 256, "Too many account profiles");
            for entry in entries {
                anyhow::ensure!(!entry.file_type()?.is_symlink(), "Linked account profiles are not supported");
                if entry.file_type()?.is_dir() {
                    homes.push(entry.path());
                }
            }
        },
        | Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        | Err(error) => return Err(error.into()),
    }
    homes.sort();
    let mut sets = vec![];
    for home in homes {
        for provider in gpui_agent_domain::PROVIDERS {
            if let Some(servers) = read(&home.join(gpui_agent_domain::provider_name(provider)).join("mcp.json"))? {
                sets.push(servers);
            }
        }
    }
    gpui_agent_domain::merge_mcp(sets).map_err(anyhow::Error::msg)
}
