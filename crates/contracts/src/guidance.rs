use serde::{Deserialize,
            Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillReadRequest {
    pub id: String,
    #[serde(default = "main_resource")]
    pub resource: String,
}
fn main_resource() -> String { "SKILL.md".into() }

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceFileRequest {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillEnabledRequest {
    pub id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillCatalog {
    pub skills: Vec<crate::SkillMetadata>,
    pub diagnostics: Vec<crate::GuidanceDiagnostic>,
    pub revision: String,
}
impl From<crate::GuidanceSnapshot> for SkillCatalog {
    fn from(snapshot: crate::GuidanceSnapshot) -> Self {
        Self {
            skills: snapshot.skills,
            diagnostics: snapshot.diagnostics,
            revision: snapshot.revision,
        }
    }
}

/// An agent's request to add one workspace skill. Nothing is installed until
/// an operator approves it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProposalInput {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub resources: std::collections::BTreeMap<String, String>,
}

pub type SkillProposalStatus = crate::ProposalStatus;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillProposal {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub content: String,
    pub resources: std::collections::BTreeMap<String, String>,
    /// The user's latest request in the proposing conversation, for the
    /// operator to compare against. Host-recorded, never agent-supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_during: Option<String>,
    pub status: SkillProposalStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub expires_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProposalAnswer {
    pub allow: bool,
}

/// An agent's request to add one MCP server to the current workspace. The
/// server is installed as a single-server plugin only after operator approval.
/// Authentication is limited to environment-variable references; OAuth setup
/// stays an operator action.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpProposalInput {
    pub server: crate::McpServer,
    pub rationale: String,
}

/// Whether the host currently has a usable value for a referenced environment
/// variable. The value itself is never reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpEnvironmentCheck {
    pub variable: String,
    pub ready: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpProposal {
    pub id: String,
    pub plugin_id: String,
    pub server: crate::McpServer,
    pub rationale: String,
    /// Filled in for operators only, when proposals are listed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment: Vec<McpEnvironmentCheck>,
    /// The user's latest request in the proposing conversation, for the
    /// operator to compare against. Host-recorded, never agent-supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_during: Option<String>,
    pub status: crate::ProposalStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub expires_at: u64,
}
