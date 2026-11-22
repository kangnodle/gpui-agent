use anyhow::{Context,
             Result,
             ensure};
use gpui_agent::{Agent,
                 Config,
                 Provider};
use std::{path::{Path,
                 PathBuf},
          time::Duration};

#[derive(Clone)]
pub struct Settings {
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub guidance_home: PathBuf,
    pub runtime_root: PathBuf,
    pub browser_helper: PathBuf,
    pub mcp_sessions: Option<std::sync::Arc<crate::McpSessions>>,
    pub mcp_write: bool,
    pub account_id: String,
}

impl Settings {
    pub fn discover(cwd: PathBuf, home: Option<PathBuf>) -> Result<Self> {
        let home = match home {
            | Some(home) => home,
            | None => dirs::home_dir().context("Cannot resolve home directory")?.join(".gpui-agent"),
        };
        let home = std::path::absolute(home)?;
        Ok(Self {
            cwd,
            runtime_root: home.join("runtimes"),
            guidance_home: home.clone(),
            home,
            browser_helper: std::env::current_exe()?,
            mcp_sessions: None,
            mcp_write: false,
            account_id: "default".into(),
        })
    }

    pub(crate) fn executable(&self, provider: Provider) -> Result<PathBuf> {
        let name = gpui_agent_domain::provider_name(provider);
        let variable = format!("gpui_agent_{}_EXECUTABLE", name.to_ascii_uppercase());
        let executable = if let Some(path) = std::env::var_os(&variable) {
            find_executable(Path::new(&path)).with_context(|| format!("{variable} does not point to a native executable"))?
        } else if let Some(path) = crate::runtimes::managed_path(self, provider)? {
            path
        } else if provider == Provider::Antigravity {
            dirs::data_local_dir()
                .and_then(|root| installed_antigravity(&root))
                .or_else(|| find_executable(Path::new(gpui_agent::default_executable(provider))))
                .context("Install Antigravity ACP from the runtime panel or set gpui_agent_ANTIGRAVITY_EXECUTABLE")?
        } else {
            find_executable(Path::new(gpui_agent::default_executable(provider))).with_context(|| format!("Install {name} or set {variable}"))?
        };
        Ok(executable)
    }

    pub fn agent(&self, provider: Provider) -> Result<Agent> {
        let name = gpui_agent_domain::provider_name(provider);
        let executable = self.executable(provider)?;
        let harness = (provider == Provider::Antigravity).then(|| {
            std::env::var_os("gpui_agent_ANTIGRAVITY_HARNESS").map(PathBuf::from).unwrap_or_else(|| {
                executable.with_file_name(if cfg!(windows) {
                    "localharness_external.exe"
                } else {
                    "localharness_external"
                })
            })
        });
        let agent = Agent::new(Config {
            provider,
            executable,
            home: self.home.join(name),
            cwd: self.cwd.clone(),
            harness,
            browser_helper: Some(self.browser_helper.clone()),
            timeout: Duration::from_secs(900),
            mcp_home: Some(self.guidance_home.clone()),
            mcp_selection: None,
            plugin_mcp: crate::PluginFiles::new(&self.guidance_home, &self.cwd)?.mcp_servers()?,
        })?;
        Ok(if let Some(sessions) = &self.mcp_sessions {
            agent.with_mcp_sessions(sessions.for_scope(
                gpui_agent_domain::McpScope {
                    provider,
                    account_id: self.account_id.clone(),
                    workspace_path: self.cwd.to_string_lossy().into_owned(),
                },
                self.mcp_write,
            ))
        } else {
            agent
        })
    }
}

fn installed_antigravity(data_local: &Path) -> Option<PathBuf> {
    // Keep runtime installations made before the application rename usable.
    ["gpui-agent"]
        .into_iter()
        .map(|app| {
            data_local
                .join(app)
                .join("runtimes/antigravity-1.3.0")
                .join(gpui_agent::default_executable(Provider::Antigravity))
        })
        .find(|path| path.is_file())
}

fn find_executable(path: &Path) -> Option<PathBuf> {
    let names = if cfg!(windows) && path.extension().is_none() {
        vec![path.with_extension("exe")]
    } else {
        vec![path.to_path_buf()]
    };
    if path.components().count() > 1 || path.is_absolute() {
        return names.into_iter().find(|path| path.is_file());
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        if let Some(path) = names.iter().map(|name| directory.join(name)).find(|path| path.is_file()) {
            return Some(path);
        }
    }
    None
}

pub fn new_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("Cannot generate API token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn load_or_create_token(path: &Path) -> Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        | Ok(mut file) => {
            use std::io::Write;
            let token = new_token()?;
            file.write_all(token.as_bytes())?;
            Ok(token)
        },
        | Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let token = std::fs::read_to_string(path)?.trim().to_owned();
            ensure!(token.len() >= 32 && token.bytes().all(|b| b.is_ascii_alphanumeric()), "Invalid API token file");
            Ok(token)
        },
        | Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_requires_a_file_in_the_current_installation() -> Result<()> {
        let root = tempfile::tempdir()?;
        assert_eq!(installed_antigravity(root.path()), None);
        let current = root.path().join("gpui-agent/runtimes/antigravity-1.3.0");
        std::fs::create_dir_all(&current)?;
        let current_executable = current.join(gpui_agent::default_executable(Provider::Antigravity));
        // A directory with the executable's name is not a runtime.
        std::fs::create_dir(&current_executable)?;
        assert_eq!(installed_antigravity(root.path()), None);
        std::fs::remove_dir(&current_executable)?;
        std::fs::write(&current_executable, b"current runtime")?;
        assert_eq!(installed_antigravity(root.path()), Some(current_executable));
        Ok(())
    }
}
