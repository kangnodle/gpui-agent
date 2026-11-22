use crate::{Application,
            ApplicationError,
            ApplicationResult,
            ErrorKind};
use gpui_agent_domain::{PluginCatalog,
                        PluginImportRequest,
                        PluginSyncResult,
                        validate_plugin_id};
use gpui_agent_ports::PluginSource;
use std::sync::Arc;

#[derive(Clone)]
pub struct Plugins(Arc<dyn PluginSource>);
impl Plugins {
    pub fn new(source: Arc<dyn PluginSource>) -> Self { Self(source) }

    pub async fn catalog(&self) -> ApplicationResult<PluginCatalog> {
        let source = self.0.clone();
        tokio::task::spawn_blocking(move || source.catalog())
            .await
            .map_err(anyhow::Error::from)?
            .map_err(ApplicationError::from)
    }

    pub async fn import(&self, request: PluginImportRequest) -> ApplicationResult<PluginCatalog> {
        request.validate().map_err(|error| ApplicationError(ErrorKind::Invalid, error))?;
        let source = self.0.clone();
        tokio::task::spawn_blocking(move || source.import(&request))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn sync_claude(&self, scope: &str, enable: bool) -> ApplicationResult<PluginSyncResult> {
        if !matches!(scope, "user" | "repo") {
            return Err(ApplicationError(ErrorKind::Invalid, "Select user or repo scope".into()));
        }
        let source = self.0.clone();
        let scope = scope.to_owned();
        tokio::task::spawn_blocking(move || source.sync_claude(&scope, enable))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> ApplicationResult<PluginCatalog> {
        validate_plugin_id(id)?;
        let source = self.0.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || source.set_enabled(&id, enabled))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }

    pub async fn remove(&self, id: &str) -> ApplicationResult<PluginCatalog> {
        validate_plugin_id(id)?;
        let source = self.0.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || source.remove(&id))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(|error| ApplicationError(ErrorKind::Invalid, error.to_string()))
    }
}
impl Application {
    pub fn with_plugins(mut self, plugins: Plugins) -> Self {
        Arc::get_mut(&mut self.0).expect("configure before cloning").plugins = Some(plugins);
        self
    }

    pub fn plugins(&self) -> ApplicationResult<Plugins> {
        self.0
            .plugins
            .clone()
            .ok_or(ApplicationError(ErrorKind::Unavailable, "Host plugins are unavailable".into()))
    }
}
