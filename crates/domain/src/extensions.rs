use crate::{McpServer,
            PluginCatalog,
            SkillReference};
use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationExtensions {
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub mcp: Vec<String>,
}

impl ConversationExtensions {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.plugins.len() > 32 || self.mcp.len() > 16 {
            return Err("At most 32 plugins and 16 MCP servers may be selected");
        }
        for (index, id) in self.plugins.iter().enumerate() {
            crate::validate_plugin_id(id).map_err(|_| "Invalid selected plugin ID")?;
            if self.plugins[.. index].contains(id) {
                return Err("Selected plugins must be unique");
            }
        }
        validate_mcp_selection(&self.mcp)
    }

    pub fn mcp_names(&self, catalog: &PluginCatalog) -> Result<Vec<String>, &'static str> {
        self.validate()?;
        let mut names = self.mcp.clone();
        for id in &self.plugins {
            let plugin = catalog
                .plugins
                .iter()
                .find(|plugin| &plugin.id == id && plugin.enabled)
                .ok_or("Selected plugin is unavailable; refresh the catalog")?;
            for server in &plugin.manifest.mcp_servers {
                let name = server.name().to_owned();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        validate_mcp_selection(&names)?;
        Ok(names)
    }

    pub fn allows_skill(&self, id: &str, explicit: &[SkillReference]) -> bool {
        explicit.iter().any(|reference| reference.id == id)
            || id
                .strip_prefix("plugin:")
                .and_then(|id| id.split_once('/'))
                .is_some_and(|(plugin, _)| self.plugins.iter().any(|id| id == plugin))
    }
}

pub fn validate_mcp_selection(names: &[String]) -> Result<(), &'static str> {
    if names.len() > 16 {
        return Err("At most 16 MCP servers may be selected");
    }
    for (index, name) in names.iter().enumerate() {
        if name.is_empty()
            || name.len() > 64
            || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || names[.. index].contains(name)
        {
            return Err("Selected MCP names must be unique ASCII letters, digits, underscores or hyphens (1–64)");
        }
    }
    Ok(())
}

pub fn select_mcp_servers(servers: Vec<McpServer>, names: Option<&[String]>) -> Result<Vec<McpServer>, &'static str> {
    let Some(names) = names else {
        return Ok(servers);
    };
    validate_mcp_selection(names)?;
    if names.iter().any(|name| !servers.iter().any(|server| server.name() == name)) {
        return Err("Selected MCP server is unavailable; refresh the catalog");
    }
    Ok(servers.into_iter().filter(|server| names.iter().any(|name| name == server.name())).collect())
}
