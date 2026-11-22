#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

mod files;
mod git;
mod guidance;
mod host_effects;
mod jobs;
mod mcp_sessions;
mod plugins;
mod project_verifier;
mod projects;
mod reloading;
mod runtimes;
mod settings;

use anyhow::Result;
use gpui_agent::Agent;
use gpui_agent_application::{Application,
                             ports::AgentRuntime};
pub use gpui_agent_history::History;
pub use guidance::GuidanceFiles;
pub use host_effects::{StderrDiagnostics,
                       SystemClock};
pub use jobs::JobStore;
pub use mcp_sessions::McpSessions;
pub use plugins::PluginFiles;
pub use projects::Projects;
pub use settings::{Settings,
                   load_or_create_token,
                   new_token};
use std::{path::Path,
          sync::Arc};

pub fn application_from_agents(agents: [Result<Agent>; gpui_agent_contracts::PROVIDERS.len()], database: &Path, cwd: &Path) -> Result<Application> {
    Ok(Application::new(
        agents.map(|agent| agent.map(|agent| Arc::new(agent) as Arc<dyn AgentRuntime>)),
        Arc::new(History::open(database, cwd)?),
    )
    .with_clock(Arc::new(SystemClock))
    .with_diagnostics(Arc::new(StderrDiagnostics))
    .with_file_context(Arc::new(files::Files::new(cwd)?))
    .with_guidance(gpui_agent_application::Guidance::new(Arc::new(GuidanceFiles::new(
        database.parent().unwrap_or(cwd),
        cwd,
    )?)))
    .with_plugins(gpui_agent_application::Plugins::new(Arc::new(PluginFiles::new(
        database.parent().unwrap_or(cwd),
        cwd,
    )?))))
}

impl Settings {
    pub fn plugins(&self) -> Result<gpui_agent_application::Plugins> {
        Ok(gpui_agent_application::Plugins::new(Arc::new(PluginFiles::new(
            &self.guidance_home,
            &self.cwd,
        )?)))
    }

    pub fn guidance(&self) -> Result<gpui_agent_application::Guidance> {
        Ok(gpui_agent_application::Guidance::new(Arc::new(GuidanceFiles::new(
            &self.guidance_home,
            &self.cwd,
        )?)))
    }

    pub fn jobs(&self, workspaces: gpui_agent_application::Workspaces) -> Result<gpui_agent_application::Jobs> {
        Ok(JobStore::open(self.clone(), workspaces.clone())?.service(workspaces))
    }

    pub fn workspaces(&self, application: Application) -> Result<gpui_agent_application::Workspaces> {
        Ok(Projects::open(self.clone(), application)?.service())
    }

    pub fn application(&self) -> Result<Application> {
        let cwd = projects::canonical(&std::path::absolute(&self.cwd)?)?;
        let settings = Self {
            cwd: std::path::PathBuf::from(cwd),
            ..self.clone()
        };
        Ok(Application::new(
            gpui_agent_domain::PROVIDERS.map(|provider| {
                Ok(Arc::new(reloading::Reloading {
                    settings: settings.clone(),
                    provider,
                }) as Arc<dyn AgentRuntime>)
            }),
            Arc::new(History::open(&settings.home.join("history.sqlite3"), &settings.cwd)?),
        )
        .with_clock(Arc::new(SystemClock))
        .with_diagnostics(Arc::new(StderrDiagnostics))
        .with_file_context(Arc::new(files::Files::new(&settings.cwd)?))
        .with_guidance(settings.guidance()?)
        .with_plugins(settings.plugins()?)
        .with_automatic_computer(cfg!(windows) && settings.mcp_sessions.is_some()))
    }
}
