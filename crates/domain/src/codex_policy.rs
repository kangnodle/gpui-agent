use serde::{Deserialize,
            Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexApprovalPolicy {
    #[default]
    Untrusted,
    OnRequest,
    Never,
}
impl CodexApprovalPolicy {
    pub const ALL: [Self; 3] = [Self::Untrusted, Self::OnRequest, Self::Never];

    pub fn as_str(self) -> &'static str {
        match self {
            | Self::Untrusted => "untrusted",
            | Self::OnRequest => "on-request",
            | Self::Never => "never",
        }
    }
}
impl std::str::FromStr for CodexApprovalPolicy {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|policy| policy.as_str() == value)
            .ok_or("Expected untrusted, on-request or never")
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexSandbox {
    #[default]
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}
impl CodexSandbox {
    pub const ALL: [Self; 3] = [Self::ReadOnly, Self::WorkspaceWrite, Self::DangerFullAccess];

    pub fn as_str(self) -> &'static str {
        match self {
            | Self::ReadOnly => "read-only",
            | Self::WorkspaceWrite => "workspace-write",
            | Self::DangerFullAccess => "danger-full-access",
        }
    }
}
impl std::str::FromStr for CodexSandbox {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|policy| policy.as_str() == value)
            .ok_or("Expected read-only, workspace-write or danger-full-access")
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodexExecutionPolicy {
    pub approval_policy: CodexApprovalPolicy,
    pub sandbox: CodexSandbox,
}
impl CodexExecutionPolicy {
    pub fn validate_provider(self, provider: crate::Provider) -> Result<(), &'static str> {
        if provider != crate::Provider::Codex {
            return Err("Codex execution policy is only supported by Codex");
        }
        Ok(())
    }
}
