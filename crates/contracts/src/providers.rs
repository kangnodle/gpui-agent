use gpui_agent_domain::Provider;
use serde::{Deserialize,
            Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub provider: Provider,
    pub name: String,
    pub available: bool,
    pub issue: Option<String>,
    #[serde(default)]
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    #[serde(default)]
    pub mcp_stdio: bool,
    #[serde(default)]
    pub mcp_http: bool,
    #[serde(default)]
    pub mcp_capability_check: bool,
    #[serde(default)]
    pub usage: bool,
    #[serde(default)]
    pub background_jobs: bool,
    #[serde(default)]
    pub text_attachments: bool,
    #[serde(default)]
    pub images: bool,
    #[serde(default)]
    pub image_capability_check: bool,
    pub tool_timeline: bool,
    pub approvals: bool,
    pub user_input: bool,
}
