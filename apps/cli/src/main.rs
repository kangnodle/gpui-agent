#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
use anyhow::{Context,
             Result,
             bail};
use clap::{Parser,
           Subcommand,
           ValueEnum};
use gpui_agent::{Agent,
                 Config,
                 Event,
                 Prompt,
                 Provider};
use std::{io::{self,
               IsTerminal,
               Write},
          path::PathBuf,
          time::Duration};

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn given_codex_execution_flags_then_known_values_parse_and_unknown_values_fail() {
        assert!(
            Args::try_parse_from([
                "gpui-agent",
                "ask",
                "codex",
                "Review",
                "--approval-policy",
                "on-request",
                "--sandbox",
                "workspace-write"
            ])
            .is_ok()
        );
        assert!(Args::try_parse_from(["gpui-agent", "ask", "codex", "Review", "--sandbox", "all-files"]).is_err());
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ProviderArg {
    Claude,
    #[value(alias = "chatgpt")]
    Codex,
    #[value(alias = "gemini")]
    Antigravity,
}
impl From<ProviderArg> for Provider {
    fn from(value: ProviderArg) -> Self {
        match value {
            | ProviderArg::Claude => Self::Claude,
            | ProviderArg::Codex => Self::Codex,
            | ProviderArg::Antigravity => Self::Antigravity,
        }
    }
}

#[derive(Parser)]
#[command(version, about = "Use Claude, ChatGPT/Codex, and Antigravity through native runtimes")]
struct Args {
    #[command(subcommand)]
    command: Operation,
    /// Native executable path or PATH name (no shell commands).
    #[arg(long, global = true)]
    executable: Option<PathBuf>,
    /// Provider-specific credential directory; defaults to
    /// ~/.gpui-agent/<provider>.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Shared AGENTS.md and skills directory, independent of provider profiles.
    #[arg(long, global = true)]
    guidance_home: Option<PathBuf>,
    #[arg(long, global = true, default_value = ".")]
    cwd: PathBuf,
    /// Antigravity localharness_external executable (defaults to the ACP
    /// executable's sibling).
    #[arg(long, global = true)]
    harness: Option<PathBuf>,
    /// Total operation timeout in seconds.
    #[arg(long, global = true, default_value_t = 300)]
    timeout: u64,
}

#[derive(Subcommand)]
enum Operation {
    /// List or manage local plugins without starting a model provider.
    Plugins {
        #[arg(long, conflicts_with_all = ["enable", "disable", "remove"])]
        import: Option<PathBuf>,
        #[arg(long, value_parser = ["user", "repo"], default_value = "repo", requires = "import")]
        scope: String,
        #[arg(long, conflicts_with_all = ["import", "disable", "remove"])]
        enable: Option<String>,
        #[arg(long, conflicts_with_all = ["import", "enable", "remove"])]
        disable: Option<String>,
        #[arg(long, conflicts_with_all = ["import", "enable", "disable"])]
        remove: Option<String>,
        /// Import plugins installed by the Claude provider profile.
        #[arg(long, conflicts_with_all = ["import", "enable", "disable", "remove"])]
        sync_claude: bool,
    },
    /// List shared skills without starting a model provider.
    Skills {
        #[arg(long, conflicts_with = "disable")]
        enable: Option<String>,
        #[arg(long, conflicts_with = "enable")]
        disable: Option<String>,
    },
    Login {
        provider: ProviderArg,
        /// Codex login from a remote/headless environment.
        #[arg(long)]
        device_code: bool,
        /// Print the Codex/Antigravity URL without opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    Status {
        provider: ProviderArg,
    },
    Logout {
        provider: ProviderArg,
    },
    Models {
        provider: ProviderArg,
    },
    Ask {
        /// Scoped skill ID from the skills command; repeat for multiple skills.
        #[arg(long = "skill")]
        skills: Vec<String>,
        provider: ProviderArg,
        prompt: String,
        /// Codex: untrusted, on-request, never (default: untrusted).
        #[arg(long)]
        approval_policy: Option<gpui_agent::CodexApprovalPolicy>,
        /// Codex: read-only, workspace-write, danger-full-access.
        #[arg(long)]
        sandbox: Option<gpui_agent::CodexSandbox>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        session: Option<String>,
        /// Emit normalized JSON Lines instead of text.
        #[arg(long)]
        json: bool,
        /// Ask before each native tool approval (otherwise deny).
        #[arg(long)]
        approve_tools: bool,
    },
}

impl Operation {
    fn provider(&self) -> Option<ProviderArg> {
        Some(match self {
            | Self::Skills {
                ..
            }
            | Self::Plugins {
                ..
            } => return None,
            | Self::Login {
                provider, ..
            }
            | Self::Status {
                provider,
            }
            | Self::Logout {
                provider,
            }
            | Self::Models {
                provider,
            }
            | Self::Ask {
                provider, ..
            } => *provider,
        })
    }
}

fn permission(event: &Event, interactive: bool) -> bool {
    if !interactive || !io::stdin().is_terminal() {
        return false;
    }
    eprintln!("Tool approval: {}", serde_json::to_string(event).unwrap_or_default());
    eprint!(
        "{} [y/N] ",
        if matches!(event, Event::Permission { method, .. } if method == "item/permissions/requestApproval") {
            "Allow the requested permissions for this turn?"
        } else {
            "Allow this request once?"
        }
    );
    let _ = io::stderr().flush();
    let mut input = String::new();
    io::stdin().read_line(&mut input).is_ok() && input.trim().eq_ignore_ascii_case("y")
}

#[tokio::main]
async fn main() -> Result<()> {
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    let args = Args::parse();
    let guidance_home = args
        .guidance_home
        .clone()
        .or_else(|| dirs::home_dir().map(|home| home.join(".gpui-agent")))
        .context("Cannot resolve guidance home; pass --guidance-home")?;
    let settings = gpui_agent_infrastructure::Settings::discover(args.cwd.clone(), Some(guidance_home.clone()))?;
    if let Operation::Plugins {
        import,
        scope,
        enable,
        disable,
        remove,
        sync_claude,
    } = &args.command
    {
        let plugins = settings.plugins()?;
        if *sync_claude {
            let result = plugins.sync_claude(if scope == "user" { "user" } else { "repo" }, false).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }
        let catalog = if let Some(source) = import {
            plugins
                .import(gpui_agent_infrastructure::PluginFiles::collect_plugin(source, scope.clone())?)
                .await?
        } else if let Some(id) = enable {
            plugins.set_enabled(id, true).await?
        } else if let Some(id) = disable {
            plugins.set_enabled(id, false).await?
        } else if let Some(id) = remove {
            plugins.remove(id).await?
        } else {
            plugins.catalog().await?
        };
        println!("{}", serde_json::to_string_pretty(&catalog)?);
        return Ok(());
    }
    if let Operation::Skills {
        enable,
        disable,
    } = &args.command
    {
        let guidance = settings.guidance()?;
        if let Some(id) = enable {
            guidance.set_skill_enabled(id, true).await?;
        }
        if let Some(id) = disable {
            guidance.set_skill_enabled(id, false).await?;
        }
        let snapshot = guidance.snapshot().await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"skills":snapshot.skills,"diagnostics":snapshot.diagnostics,"revision":snapshot.revision}))?
        );
        return Ok(());
    }
    let provider: Provider = args.command.provider().context("Command requires a provider")?.into();
    let name = match provider {
        | Provider::Claude => "claude",
        | Provider::Codex => "codex",
        | Provider::Antigravity => "antigravity",
    };
    let executable = args.executable.unwrap_or_else(|| gpui_agent::default_executable(provider).into());
    let harness = args.harness.or_else(|| {
        if provider == Provider::Antigravity && executable.components().count() > 1 {
            Some(executable.with_file_name(if cfg!(windows) {
                "localharness_external.exe"
            } else {
                "localharness_external"
            }))
        } else {
            None
        }
    });
    let home = match args.home {
        | Some(home) => home,
        | None => dirs::home_dir()
            .context("cannot resolve home directory; pass --home")?
            .join(".gpui-agent")
            .join(name),
    };
    let agent = Agent::new(Config {
        provider,
        executable,
        home,
        mcp_home: Some(guidance_home.clone()),
        mcp_selection: None,
        plugin_mcp: gpui_agent_infrastructure::PluginFiles::new(&guidance_home, &args.cwd)?.mcp_servers()?,
        cwd: args.cwd,
        harness,
        browser_helper: Some(std::env::current_exe()?),
        timeout: Duration::from_secs(args.timeout),
    })?;
    let operation = async {
        match args.command {
            | Operation::Skills {
                ..
            }
            | Operation::Plugins {
                ..
            } => unreachable!("management returned before native initialization"),
            | Operation::Login {
                device_code,
                no_browser,
                ..
            } => {
                let status = agent
                    .login(device_code, &mut |event| {
                        if let Event::Authorization {
                            url,
                            code,
                        } = event
                        {
                            eprintln!("Sign in: {url}");
                            if let Some(code) = code {
                                eprintln!("Code: {code}");
                            }
                            if !no_browser && let Err(error) = webbrowser::open(&url) {
                                eprintln!("Open the URL manually ({error})");
                            }
                        }
                        false
                    })
                    .await?;
                println!("{}", serde_json::to_string_pretty(&status)?);
            },
            | Operation::Status {
                ..
            } => {
                println!("{}", serde_json::to_string_pretty(&agent.status().await?)?)
            },
            | Operation::Logout {
                ..
            } => {
                agent.logout().await?;
                println!("Signed out of {name}.");
            },
            | Operation::Models {
                ..
            } => {
                println!("{}", serde_json::to_string_pretty(&agent.models().await?)?)
            },
            | Operation::Ask {
                skills,
                prompt,
                approval_policy,
                sandbox,
                model,
                session,
                json,
                approve_tools,
                ..
            } => {
                let references = skills
                    .into_iter()
                    .map(|id| gpui_agent::SkillReference {
                        id,
                    })
                    .collect::<Vec<_>>();
                let guidance = settings.guidance()?.prepare(&prompt, &references, false).await?;
                let result = agent
                    .prompt(
                        &Prompt {
                            mcp_servers: None,
                            skills: guidance.references(),
                            guidance: Some(guidance),
                            claude_permission_mode: None,
                            codex_execution_policy: (approval_policy.is_some() || sandbox.is_some()).then(|| gpui_agent::CodexExecutionPolicy {
                                approval_policy: approval_policy.unwrap_or_default(),
                                sandbox: sandbox.unwrap_or_default(),
                            }),
                            attachments: Vec::new(),
                            text: prompt,
                            model,
                            session_id: session,
                        },
                        &mut |event| {
                            if json {
                                println!("{}", serde_json::to_string(&event).unwrap_or_default());
                            }
                            match &event {
                                | Event::RuntimeStatus {
                                    status,
                                    message,
                                    ..
                                } if !json => eprintln!("{status:?}: {message}"),
                                | Event::Text {
                                    delta,
                                } if !json => {
                                    print!("{delta}");
                                    let _ = io::stdout().flush();
                                },
                                | Event::Session {
                                    id,
                                } if !json => eprintln!("Session: {id}"),
                                | Event::Permission {
                                    ..
                                } => {
                                    return permission(&event, approve_tools);
                                },
                                | _ => {},
                            }
                            false
                        },
                    )
                    .await?;
                if json {
                    println!("{}", serde_json::json!({"type":"completed","session_id":result.session_id,"text":result.text}));
                } else {
                    println!();
                }
            },
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {
        result = operation => result,
        signal = tokio::signal::ctrl_c() => { signal?; bail!("operation cancelled"); }
    }
}
