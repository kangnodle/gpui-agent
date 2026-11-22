#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
extern crate gpui_kit as gpui;
mod accounts;
mod app;
mod checkpoint;
mod client;
mod codex_policy;
mod computer;
mod content;
mod guidance;
mod history;
mod interaction;
mod job_monitor;
mod language;
mod maintenance;
mod mcp;
mod messages;
mod panels;
mod plugins;
mod pull_request;
mod settings;
#[cfg(test)]
mod tests;
mod titlebar;
mod ui;
mod usage;
mod views;
mod workspace;

use anyhow::Result;
use clap::Parser;
use gpui::AppContext;
use gpui_agent_api::Api;
use gpui_agent_infrastructure::Settings;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Native GPUI chat for Claude, Codex, and Antigravity")]
struct Args {
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    #[arg(long)]
    home: Option<PathBuf>,
    /// Connect to a loopback/SSH tunnel or HTTPS API.
    #[arg(long, requires = "token_file")]
    api_url: Option<String>,
    #[arg(long, requires = "api_url")]
    token_file: Option<PathBuf>,
    /// Allow injected sessions to manage scoped conversations/jobs on the
    /// embedded host.
    #[arg(long, conflicts_with = "api_url")]
    mcp_write: bool,
}

fn fonts(cx: &mut gpui::App) {
    let mut names = cx.text_system().all_font_names();
    let mut family = None;
    if let Some(path) = std::env::var_os("gpui_agent_FONT") {
        match std::fs::read(path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| cx.text_system().add_fonts(vec![bytes.into()]))
        {
            | Ok(()) => family = cx.text_system().all_font_names().into_iter().find(|name| !names.contains(name)),
            | Err(error) => eprintln!("Cannot load gpui_agent_FONT: {error}"),
        }
        names = cx.text_system().all_font_names();
    }
    let family = family.or_else(|| {
        ["Noto Sans KR", "Malgun Gothic", "Apple SD Gothic Neo", "Noto Sans CJK KR", "Noto Sans CJK JP"]
            .into_iter()
            .find(|name| names.iter().any(|n| n == name))
            .map(str::to_owned)
    });
    if let Some(family) = family {
        gpui::component::Theme::update(cx, |theme| theme.font_family = family.into());
    }
}

fn main() -> Result<()> {
    if gpui_agent::run_browser_helper() {
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    if runtime.block_on(gpui_agent_api::run_mcp_bridge())? {
        return Ok(());
    }
    let args = Args::parse();
    let mut settings = Settings::discover(args.cwd.clone(), args.home)?;
    let mut shutdown = None;
    let mut embedded = None;
    let (url, token) = if let Some(url) = args.api_url {
        client::validate_url(&url)?;
        let token_file = match args.token_file {
            | Some(path) => path,
            | None => anyhow::bail!("--token-file is required when connecting to an API"),
        };
        (
            url.trim_end_matches('/').to_owned(),
            gpui_agent_infrastructure::read_private_token(&token_file)?,
        )
    } else {
        let listener = runtime.block_on(tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)))?;
        let sessions = std::sync::Arc::new(gpui_agent_infrastructure::McpSessions::new(format!("http://{}/mcp", listener.local_addr()?))?);
        settings.mcp_sessions = Some(sessions.clone());
        settings.mcp_write = args.mcp_write;
        let token = gpui_agent_infrastructure::new_token()?;
        let mcp_token = settings.load_host_token(&settings.home.join("mcp-read.token"))?;
        let application = settings.application()?;
        let workspaces = gpui_agent_infrastructure::Projects::open_catalog(settings.clone(), application.clone())?.service();
        let jobs = settings.jobs(workspaces.clone())?;
        let api = Api::new(application, token.clone())?
            .with_mcp_sessions(sessions)
            .with_mcp_read_token(mcp_token)?
            .with_workspaces(workspaces)
            .with_jobs(jobs)
            .with_controls(std::sync::Arc::new(gpui_agent_control::Controls::default().with_automatic_desktop()));
        let url = format!("http://{}", listener.local_addr()?);
        let (tx, rx) = tokio::sync::oneshot::channel();
        shutdown = Some(tx);
        embedded = Some(runtime.spawn(gpui_agent_api::serve(listener, api, async {
            let _ = rx.await;
        })));
        (url, token)
    };
    let client = client::ApiClient::new(url, token)?;
    let (wake_tx, wake_rx) = async_channel::unbounded();
    let wake = ui::Wake(wake_tx);
    let failure = std::rc::Rc::new(std::cell::RefCell::new(None));
    let window_failure = failure.clone();
    gpui::application().with_assets(gpui::assets::AllAssets).run(move |cx| {
        gpui::init(cx);
        ui::apply_theme(cx);
        fonts(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                None,
                gpui::size(gpui::px(1100.), gpui::px(780.)),
                cx,
            ))),
            ..titlebar::TitlebarMode::load(&settings.home.join("ui-titlebar")).window_options()
        };
        let opened = gpui::open_window(options, cx, |window, cx| {
            cx.new(|cx: &mut gpui::Context<app::ChatApp>| {
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                let app = app::ChatApp::new(client, args.cwd.display().to_string(), Some(settings.home.join("ui-language")), &wake, focus);
                cx.spawn(async move |view, cx| {
                    while let Ok(event) = wake_rx.recv().await {
                        if view
                            .update(cx, |view, cx| {
                                view.handle_wakes(std::iter::once(event).chain(std::iter::from_fn(|| wake_rx.try_recv().ok())), cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
                cx.spawn(async move |view, cx| {
                    loop {
                        cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                        if view
                            .update(cx, |view, cx| {
                                view.tick(cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
                app
            })
        });
        if let Err(error) = opened {
            *window_failure.borrow_mut() = Some(error.to_string());
            cx.quit();
        }
        cx.activate(true);
    });
    if let Some(shutdown) = shutdown {
        let _ = shutdown.send(());
    }
    if let Some(task) = embedded {
        let _ = runtime.block_on(task);
    }
    if let Some(error) = failure.borrow_mut().take() {
        anyhow::bail!("Cannot open GPUI window: {error}");
    }
    Ok(())
}
