use super::*;
use gpui_agent_application::ports::{HostControl,
                                    RuntimeFuture};

// The public fixture credential must never authorize real host controls.
pub(super) struct NoControls;
impl HostControl for NoControls {
    fn status(&self) -> RuntimeFuture<'_, ControlStatus> {
        Box::pin(async {
            Ok(ControlStatus {
                browser_available: false,
                browser_running: false,
                desktop_available: false,
                desktop_enabled_until: 0,
            })
        })
    }

    fn browser(&self, _: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> { Box::pin(async { anyhow::bail!("Fixture host controls are disabled") }) }

    fn desktop(&self, command: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> { self.browser(command) }

    fn enable_desktop(&self, _: bool) -> RuntimeFuture<'_, ()> { Box::pin(async { anyhow::bail!("Fixture host controls are disabled") }) }

    fn shutdown(&self) {}
}

pub fn supervise() -> Result<()> {
    // Windows can retain SQLite handles in tasks until the host runtime exits.
    let root = tempfile::tempdir()?;
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg(if std::env::args().any(|arg| arg == "--live-computer") {
            "--live-computer-host"
        } else if std::env::args().any(|arg| arg == "--computer-fixture") {
            "--computer-fixture-host"
        } else {
            "--client-fixture-host"
        })
        .env("TMPDIR", root.path())
        .env("TEMP", root.path())
        .env("TMP", root.path())
        .status()?;
    root.close()?;
    ensure!(status.success(), "Fixture host did not shut down successfully");
    Ok(())
}

pub async fn run() -> Result<()> {
    let server = Server::open_scenario(Provider::Codex, "workflow").await?;
    guidance::seed_guidance(&server)?;
    for provider in PROVIDERS {
        server.login(provider, "default").await?;
    }
    let token_file = server.root.path().join("home/api.token");
    gpui_agent_infrastructure::load_or_create_token(&token_file)?;
    fs::write(&token_file, TOKEN)?;
    println!("Fixture client: {}", server.url);
    println!("Fixture-only token file: {}", token_file.display());
    println!("Synthetic provider; isolated temporary home. Press Enter to stop and remove state.");
    tokio::task::spawn_blocking(|| std::io::stdin().read_line(&mut String::new())).await??;
    server.stop().await
}
