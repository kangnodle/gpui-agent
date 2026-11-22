#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! Subscription authentication and streaming conversations through native agent
//! runtimes.
mod adapter;
mod antigravity;
mod browser;
mod claude;
mod codex;
mod config;
mod mcp;
mod mcp_oauth;
pub use mcp_oauth::McpOAuth;
pub mod process;
mod recovery;
mod rpc;
mod tools;
mod usage;

use anyhow::{Result,
             ensure};
pub fn run_browser_helper() -> bool { browser::run_browser_helper() }
pub use config::{Config,
                 default_executable,
                 load_mcp_config};
pub use gpui_agent_domain::{AuthStatus,
                            ClaudePermissionMode,
                            CodexApprovalPolicy,
                            CodexExecutionPolicy,
                            CodexSandbox,
                            Completion,
                            EmitFuture,
                            Event,
                            EventHandler,
                            GuidanceSnapshot,
                            Handler,
                            InteractionResponse,
                            McpOAuthCommand,
                            McpOAuthState,
                            McpServer,
                            Model,
                            PreparedGuidance,
                            Prompt,
                            Provider,
                            ResponseFuture,
                            SkillReference};
pub use gpui_agent_ports::{AgentRuntime,
                           McpConnection,
                           RuntimeFuture,
                           RuntimeInput,
                           TurnInput};
use serde_json::Value;

pub struct Agent {
    config: Config,
    mcp: Option<gpui_agent_ports::McpConnection>,
    mcp_issuer: Option<std::sync::Arc<dyn gpui_agent_ports::McpSessionIssuer>>,
}

impl Agent {
    pub async fn validate_runtime(&self) -> Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            if self.config.provider == Provider::Antigravity {
                antigravity::validate(&self.config).await
            } else {
                use tokio::io::AsyncReadExt;
                let mut child = process::spawn(
                    process::command(&self.config)
                        .arg("--version")
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::null()),
                )?;
                let mut out = Vec::new();
                let stdout = match child.stdout.take() {
                    | Some(stdout) => stdout,
                    | None => anyhow::bail!("Runtime version stdout pipe is unavailable"),
                };
                stdout.take(65537).read_to_end(&mut out).await?;
                ensure!(out.len() <= 65536 && !out.is_empty(), "Invalid runtime version output");
                ensure!(child.wait().await?.success(), "Runtime version check failed");
                Ok(())
            }
        })
        .await?
    }

    fn operation_lock(&self) -> Result<std::fs::File> {
        use anyhow::Context;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.config.home.join(".operation.lock"))?;
        file.try_lock()
            .context("another operation is using this provider profile; finish or cancel it first")?;
        Ok(file)
    }

    pub fn new(mut config: Config) -> Result<Self> {
        ensure!(!config.timeout.is_zero(), "timeout must be positive");
        ensure!(!config.executable.as_os_str().is_empty(), "executable is empty");
        // Windows canonicalize adds a verbatim prefix that Antigravity cannot
        // turn into a file URI.
        config.cwd = std::path::absolute(&config.cwd)?;
        ensure!(config.cwd.is_dir(), "cwd must be a directory");
        if !config.home.is_absolute() {
            config.home = std::env::current_dir()?.join(config.home);
        }
        if config.executable.components().count() > 1 && !config.executable.is_absolute() {
            config.executable = std::env::current_dir()?.join(config.executable);
        }
        if let Some(harness) = &mut config.harness {
            *harness = harness.canonicalize()?;
            ensure!(harness.is_file(), "Antigravity harness must be a file");
        }
        std::fs::create_dir_all(&config.home)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config.home, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            config,
            mcp: None,
            mcp_issuer: None,
        })
    }

    pub fn with_mcp_connection(mut self, connection: gpui_agent_ports::McpConnection) -> Self {
        self.mcp = Some(connection);
        self.mcp_issuer = None;
        self
    }

    pub fn with_mcp_sessions(mut self, issuer: std::sync::Arc<dyn gpui_agent_ports::McpSessionIssuer>) -> Self {
        self.mcp = None;
        self.mcp_issuer = Some(issuer);
        self
    }

    /// Dropping this future cancels the operation and terminates its owned
    /// child process.
    pub async fn login(&self, device_code: bool, handler: &mut Handler<'_>) -> Result<AuthStatus> {
        let _lock = self.operation_lock()?;
        tokio::time::timeout(self.config.timeout, async {
            match self.config.provider {
                | Provider::Claude => {
                    ensure!(!device_code, "device-code login is only supported by Codex");
                    claude::login(&self.config, handler).await
                },
                | Provider::Codex => codex::login(&self.config, device_code, handler).await,
                | Provider::Antigravity => {
                    ensure!(!device_code, "device-code login is only supported by Codex");
                    antigravity::login(&self.config, handler).await
                },
            }
        })
        .await?
    }

    pub async fn status(&self) -> Result<AuthStatus> {
        let _lock = self.operation_lock()?;
        tokio::time::timeout(self.config.timeout, async {
            match self.config.provider {
                | Provider::Claude => claude::status(&self.config).await,
                | Provider::Codex => codex::status(&self.config).await,
                | Provider::Antigravity => antigravity::status(&self.config).await,
            }
        })
        .await?
    }

    pub async fn logout(&self) -> Result<()> {
        let _lock = self.operation_lock()?;
        tokio::time::timeout(self.config.timeout, async {
            match self.config.provider {
                | Provider::Claude => claude::logout(&self.config).await,
                | Provider::Codex => codex::logout(&self.config).await,
                | Provider::Antigravity => antigravity::logout(&self.config).await,
            }
        })
        .await?
    }

    pub async fn models(&self) -> Result<Vec<Model>> {
        let _lock = self.operation_lock()?;
        tokio::time::timeout(self.config.timeout, async {
            match self.config.provider {
                | Provider::Claude => Ok(claude::models()),
                | Provider::Codex => codex::models(&self.config).await,
                | Provider::Antigravity => antigravity::models(&self.config).await,
            }
        })
        .await?
    }

    pub async fn prompt(&self, prompt: &Prompt, handler: &mut Handler<'_>) -> Result<Completion> { self.prompt_with_input(prompt, handler, None).await }

    pub async fn prompt_with_input(
        &self,
        prompt: &Prompt,
        handler: &mut Handler<'_>,
        input: Option<&mut dyn gpui_agent_ports::TurnInput>,
    ) -> Result<Completion> {
        if let Some(policy) = prompt.codex_execution_policy {
            policy.validate_provider(self.config.provider).map_err(anyhow::Error::msg)?;
        }
        gpui_agent_domain::validate_message(&prompt.text, prompt.session_id.as_deref(), &prompt.attachments).map_err(anyhow::Error::msg)?;
        tokio::time::timeout(self.config.timeout, async {
            // Status reads share this lock and must not fail an admitted turn.
            let mut announced = false;
            let _lock = loop {
                match self.operation_lock() {
                    | Ok(lock) => break lock,
                    | Err(error) if matches!(error.downcast_ref::<std::fs::TryLockError>(), Some(std::fs::TryLockError::WouldBlock)) => {
                        if !announced {
                            handler
                                .emit_async(Event::RuntimeStatus {
                                    status: gpui_agent_domain::RuntimeStatus::Retrying,
                                    message: "Waiting for the provider profile to become available".into(),
                                    code: Some("profile_busy".into()),
                                })
                                .await;
                            announced = true;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    },
                    | Err(error) => return Err(error),
                }
            };
            let session = self
                .mcp_issuer
                .as_ref()
                .map(|issuer| issuer.issue_for_job(handler.conversation_id(), handler.job_id()))
                .transpose()?;
            let mcp = session.as_ref().map(|session| session.connection()).or(self.mcp.as_ref());
            let mut config = self.config.clone();
            config.mcp_selection = prompt.mcp_servers.clone();
            let oauth_config = config.clone();
            let tokens = tokio::task::spawn_blocking(move || McpOAuth::tokens(&oauth_config.home, &oauth_config.mcp()?)).await??;
            match config.provider {
                | Provider::Claude => claude::prompt(&config, prompt, handler, mcp, &tokens).await,
                | Provider::Codex => codex::prompt(&config, prompt, handler, mcp, &tokens, input).await,
                | Provider::Antigravity => antigravity::prompt(&config, prompt, handler, mcp, &tokens).await,
            }
        })
        .await?
    }
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("runtime response is missing {pointer}"))
}

async fn text(handler: &mut Handler<'_>, output: &mut String, delta: &str) {
    output.push_str(delta);
    handler
        .emit_async(Event::Text {
            delta: delta.to_owned(),
        })
        .await;
}
