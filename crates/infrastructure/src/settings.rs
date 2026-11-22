use anyhow::{Context,
             Result};
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
    pub(crate) private_tokens: Vec<(PathBuf, crate::PrivateFileIdentity)>,
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
            private_tokens: Vec::new(),
            home,
            browser_helper: std::env::current_exe()?,
            mcp_sessions: None,
            mcp_write: false,
            account_id: "default".into(),
        })
    }

    pub fn load_host_token(&mut self, path: &Path) -> Result<String> {
        let path = std::path::absolute(path)?;
        let (token, identity) = crate::tokens::load_host_token(&path)?;
        self.private_tokens.push((path, identity));
        Ok(token)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_or_create_token;

    #[cfg(unix)]
    #[test]
    fn existing_api_token_requires_private_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        let path = root.path().join("token");
        let token = load_or_create_token(&path)?;
        for mode in [0o640, 0o602] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            assert!(load_or_create_token(&path).is_err());
        }
        for mode in [0o400, 0o600] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            assert_eq!(load_or_create_token(&path)?, token);
        }
        Ok(())
    }

    #[test]
    fn existing_api_token_rejects_oversized_credentials() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("token");
        load_or_create_token(&path)?;
        std::fs::write(&path, vec![b'a'; 65536])?;
        assert!(load_or_create_token(&path).is_err());
        Ok(())
    }

    #[test]
    fn existing_api_token_is_reused_and_hard_links_are_rejected() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("token");
        assert!(crate::read_private_token(&path).is_err());
        assert!(!path.exists());
        let token = load_or_create_token(&path)?;
        assert_eq!(crate::read_private_token(&path)?, token);
        assert_eq!(load_or_create_token(&path)?, token);
        let alias = root.path().join("alias");
        std::fs::hard_link(&path, &alias)?;
        assert!(crate::read_private_token(&alias).is_err());
        assert!(load_or_create_token(&path).is_err());
        Ok(())
    }

    #[test]
    fn concurrent_api_token_creation_returns_one_complete_credential() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("home/token");
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|threads| {
            let calls: Vec<_> = (0 .. 4)
                .map(|_| {
                    threads.spawn(|| {
                        start.wait();
                        load_or_create_token(&path)
                    })
                })
                .collect();
            let tokens = calls.into_iter().map(|call| call.join().unwrap()).collect::<Result<Vec<_>>>()?;
            let stored = crate::read_private_token(&path)?;
            assert_eq!(stored.len(), 64);
            assert!(tokens.iter().all(|token| *token == stored));
            Ok(())
        })
    }

    #[cfg(unix)]
    #[test]
    fn existing_api_token_rejects_symbolic_links_and_pipes() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("token");
        load_or_create_token(&path)?;
        let link = root.path().join("link");
        std::os::unix::fs::symlink(path, &link)?;
        assert!(crate::read_private_token(&link).is_err());
        let pipe = root.path().join("pipe");
        rustix::fs::mkfifoat(rustix::fs::CWD, &pipe, rustix::fs::Mode::RUSR)?;
        assert!(crate::read_private_token(&pipe).is_err());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn existing_api_token_rejects_an_acl_grant_to_everyone() -> Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("token");
        load_or_create_token(&path)?;
        let result = std::process::Command::new("icacls").arg(&path).args(["/grant", "*S-1-1-0:(R)"]).output()?;
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(crate::read_private_token(&path).is_err());
        assert!(load_or_create_token(&path).is_err());
        Ok(())
    }

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
