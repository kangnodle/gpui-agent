#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
use anyhow::Result;
use clap::Parser;
use gpui_agent_api::Api;
use gpui_agent_infrastructure::{Settings,
                                load_or_create_token};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Local API for Gpui Agent")]
struct Args {
    #[arg(long, default_value_t = 7431)]
    port: u16,
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    /// Root directory containing claude/codex/antigravity profiles.
    #[arg(long)]
    home: Option<PathBuf>,
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Allow injected sessions to manage scoped conversations/jobs in the
    /// same workspace.
    #[arg(long)]
    mcp_write: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    if gpui_agent_api::run_mcp_bridge().await? {
        return Ok(());
    }
    let args = Args::parse();
    let mut settings = Settings::discover(args.cwd, args.home)?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, args.port)).await?;
    let sessions = std::sync::Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("http://{}/mcp", listener.local_addr()?))?);
    settings.mcp_sessions = Some(sessions.clone());
    settings.mcp_write = args.mcp_write;
    let token_file = args.token_file.unwrap_or_else(|| settings.home.join("api.token"));
    let application = settings.application()?;
    let workspaces = settings.workspaces(application.clone())?;
    let jobs = settings.jobs(workspaces.clone())?;
    let api = Api::new(application, load_or_create_token(&token_file)?)?
        .with_mcp_sessions(sessions)
        .with_mcp_read_token(load_or_create_token(&settings.home.join("mcp-read.token"))?)?
        .with_workspaces(workspaces)
        .with_jobs(jobs)
        .with_controls(std::sync::Arc::new(gpui_agent_control::Controls::default().with_automatic_desktop()));
    println!("Gpui Agent API: http://{}", listener.local_addr()?);
    println!("Bearer token file: {}", token_file.display());
    println!("Read-only MCP token file: {}", settings.home.join("mcp-read.token").display());
    gpui_agent_api::serve(listener, api, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}
