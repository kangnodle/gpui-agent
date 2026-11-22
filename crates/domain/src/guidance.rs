//! Provider-independent guidance policy. No filesystem or runtime access.
use serde::{Deserialize,
            Serialize};

pub const MAX_GUIDANCE_DOCUMENT_BYTES: usize = 32 * 1024;
pub const MAX_SKILL_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuidanceFile {
    pub id: String,
    pub text: String,
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuidanceFileCatalog {
    pub files: Vec<String>,
    pub project_prefix: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceWriteRequest {
    pub id: String,
    pub text: String,
    pub expected_revision: Option<String>,
}

pub fn validate_guidance_file_id(id: &str) -> Result<(&str, &str), &'static str> {
    let (scope, path) = id.split_once(':').ok_or("Unknown guidance scope")?;
    if !matches!(scope, "user" | "repo") {
        return Err("Unknown guidance scope");
    }
    validate_skill_resource(path)?;
    let parts: Vec<_> = path.split('/').collect();
    if parts
        .iter()
        .any(|part| part.ends_with(['.', ' ']) || part.chars().any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*')))
    {
        return Err("Invalid guidance path");
    }
    let name = *parts.last().ok_or("Invalid guidance path")?;
    let agents = matches!(name, "AGENTS.md" | "AGENTS.override.md");
    let skill = name == "SKILL.md"
        && if scope == "user" {
            path.starts_with("skills/") && parts.len() >= 3
        } else {
            path.contains(".agents/skills/") && parts.len() >= 4
        };
    if !(agents && (scope == "repo" || parts.len() == 1) || skill) {
        return Err("Only AGENTS.md and skill manifests can be edited");
    }
    Ok((scope, path))
}

impl GuidanceWriteRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let (_, path) = validate_guidance_file_id(&self.id)?;
        if path.ends_with("/SKILL.md") {
            let name = path.rsplit('/').nth(1).ok_or("Invalid skill path")?;
            SkillMetadata::parse(&self.id, name, &self.text, None)?;
        } else if self.text.len() > MAX_GUIDANCE_DOCUMENT_BYTES {
            return Err("AGENTS document exceeds 32 KiB");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub display_name: Option<String>,
    pub enabled: bool,
    pub allow_implicit_invocation: bool,
    #[serde(default)]
    pub revision: String,
}

#[derive(Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Auxiliary {
    interface: SkillInterface,
    policy: SkillPolicy,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct SkillInterface {
    display_name: Option<String>,
}
#[derive(Deserialize)]
#[serde(default)]
struct SkillPolicy {
    allow_implicit_invocation: bool,
}
impl Default for SkillPolicy {
    fn default() -> Self {
        Self {
            allow_implicit_invocation: true,
        }
    }
}

impl SkillMetadata {
    pub fn parse(id: &str, directory_name: &str, text: &str, auxiliary: Option<&str>) -> Result<(Self, String), &'static str> {
        if text.len() > MAX_SKILL_BYTES || auxiliary.is_some_and(|value| value.len() > MAX_GUIDANCE_DOCUMENT_BYTES) {
            return Err("Skill document exceeds its size limit");
        }
        let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let front = normalized.strip_prefix("---\n").ok_or("Skill requires YAML frontmatter")?;
        let (yaml, body) = front
            .split_once("\n---\n")
            .or_else(|| front.strip_suffix("\n---").map(|yaml| (yaml, "")))
            .ok_or("Skill frontmatter has no closing delimiter")?;
        let meta: Frontmatter = serde_yaml_ng::from_str(yaml).map_err(|_| "Invalid skill YAML frontmatter")?;
        let name = meta.name.filter(|value| !value.trim().is_empty()).unwrap_or_else(|| directory_name.to_owned());
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("Skill name must contain 1–64 characters without whitespace or controls");
        }
        let description = meta.description.split_whitespace().collect::<Vec<_>>().join(" ");
        if description.is_empty() || description.len() > 4096 {
            return Err("Skill description must contain 1–4096 bytes");
        }
        let auxiliary = auxiliary.and_then(|value| serde_yaml_ng::from_str::<Auxiliary>(value).ok()).unwrap_or_default();
        Ok((
            Self {
                id: id.to_owned(),
                name,
                description,
                display_name: auxiliary.interface.display_name,
                enabled: true,
                allow_implicit_invocation: auxiliary.policy.allow_implicit_invocation,
                revision: String::new(),
            },
            body.to_owned(),
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillReference {
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstructionDocument {
    pub source: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuidanceDiagnostic {
    pub source: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuidanceSnapshot {
    pub instructions: Vec<InstructionDocument>,
    pub skills: Vec<SkillMetadata>,
    pub diagnostics: Vec<GuidanceDiagnostic>,
    pub revision: String,
}

impl GuidanceSnapshot {
    pub fn select_skills(&self, text: &str, references: &[SkillReference]) -> Result<Vec<SkillMetadata>, &'static str> {
        validate_skill_references(references)?;
        let mut selected = Vec::new();
        for reference in references {
            let skill = self
                .skills
                .iter()
                .find(|skill| skill.id == reference.id && skill.enabled)
                .ok_or("Selected skill is unavailable in this workspace")?;
            if !selected.contains(skill) {
                selected.push(skill.clone());
            }
        }
        for token in text.split(|c: char| !c.is_alphanumeric() && !matches!(c, '$' | '-' | '_' | ':' | '.')) {
            let Some(name) = token.strip_prefix('$').map(|name| name.trim_end_matches('.')) else {
                continue;
            };
            if selected.iter().any(|skill| skill.name == name) {
                continue;
            }
            let mut matching = self.skills.iter().filter(|skill| skill.enabled && skill.name == name);
            if let Some(skill) = matching.next() {
                if matching.next().is_some() {
                    return Err("Skill name is ambiguous; select its scoped ID");
                }
                selected.push(skill.clone());
            }
        }
        if selected.len() > 8 {
            return Err("At most 8 skills may be selected per turn");
        }
        Ok(selected)
    }

    pub fn add_instruction(&mut self, source: String, content: String) {
        if content.trim().is_empty() {
            return;
        }
        let global = source.starts_with("user:");
        let used: usize = self
            .instructions
            .iter()
            .filter(|doc| doc.source.starts_with("user:") == global)
            .map(|doc| doc.content.len())
            .sum();
        let remaining = MAX_GUIDANCE_DOCUMENT_BYTES.saturating_sub(used);
        let mut end = remaining.min(content.len());
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = end < content.len();
        if truncated {
            self.diagnostics.push(GuidanceDiagnostic {
                source: source.clone(),
                message: "Instruction budget exceeded; content truncated".into(),
            });
        }
        if end > 0 {
            self.instructions.push(InstructionDocument {
                source,
                content: content[.. end].to_owned(),
                truncated,
            });
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillResource {
    pub skill: SkillMetadata,
    pub resource: String,
    pub text: String,
}

pub fn validate_skill_resource(resource: &str) -> Result<(), &'static str> {
    if resource.is_empty()
        || resource.len() > 4096
        || resource.contains(['\\', ':'])
        || resource.chars().any(char::is_control)
        || resource.split('/').any(|part| part.is_empty() || matches!(part, "." | ".." | ".git"))
    {
        return Err("Skill resource must be a relative path inside its skill");
    }
    Ok(())
}

pub fn validate_skill_references(references: &[SkillReference]) -> Result<(), &'static str> {
    if references.len() > 8 {
        return Err("At most 8 skills may be selected per turn");
    }
    for reference in references {
        let id = if let Some(plugin) = reference.id.strip_prefix("plugin:") {
            let (id, path) = plugin.split_once('/').ok_or("Invalid plugin skill ID")?;
            crate::validate_plugin_id(id)?;
            validate_skill_resource(path)?;
            if !path.starts_with("skills/") || !path.ends_with("/SKILL.md") {
                return Err("Invalid plugin skill document ID");
            }
            continue;
        } else {
            &reference.id
        };
        let relative = id
            .strip_prefix("repo:")
            .or_else(|| reference.id.strip_prefix("user:"))
            .ok_or("Invalid skill scope")?;
        validate_skill_resource(relative)?;
        if !relative.ends_with("/SKILL.md") {
            return Err("Invalid skill document ID");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuidanceReceipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<crate::ConversationExtensions>,
    pub revision: String,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedGuidance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<crate::ConversationExtensions>,
    pub snapshot: GuidanceSnapshot,
    pub selected: Vec<SkillResource>,
    pub implicit_available: bool,
}

impl PreparedGuidance {
    pub fn receipt(&self) -> GuidanceReceipt {
        GuidanceReceipt {
            extensions: self.extensions.clone(),
            revision: self.snapshot.revision.clone(),
            sources: self.snapshot.instructions.iter().map(|doc| doc.source.clone()).collect(),
        }
    }

    pub fn references(&self) -> Vec<SkillReference> {
        self.selected
            .iter()
            .map(|resource| SkillReference {
                id: resource.skill.id.clone(),
            })
            .collect()
    }

    pub fn render(&self) -> String {
        let mut catalog = Vec::new();
        let mut used = 0;
        let mut omitted = 0;
        if self.implicit_available {
            for skill in self.snapshot.skills.iter().filter(|skill| skill.enabled && skill.allow_implicit_invocation) {
                let item = serde_json::json!({"id":skill.id,"name":skill.name,"description":skill.description});
                let size = item.to_string().len() + 1;
                if used + size <= 8192 {
                    used += size;
                    catalog.push(item);
                } else {
                    omitted += 1;
                }
            }
        }
        let mut diagnostics = Vec::new();
        let mut diagnostic_bytes = 0;
        let mut omitted_diagnostics = 0;
        for diagnostic in &self.snapshot.diagnostics {
            let size = serde_json::to_string(diagnostic).expect("Guidance diagnostic is serializable").len() + 1;
            if diagnostic_bytes + size <= 8192 {
                diagnostic_bytes += size;
                diagnostics.push(diagnostic);
            } else {
                omitted_diagnostics += 1;
            }
        }
        format!(
            "Gpui-agent guidance for the current turn. This replaces all earlier gpui-agent guidance blocks, including deleted instructions and prior skill selections. These are user-level workflow instructions, subordinate to system/developer instructions and existing tool approvals. Apply AGENTS documents in listed order; later, more specific documents take precedence. Selected skill instructions apply to this turn. Available-skill entries are metadata: when the request matches a description, call host_skills_read with its id and resource SKILL.md, then read supporting resources only as needed. Use host_skills_list if entries were omitted. Scripts still require normal tool permissions. Native runtime guidance may also be present; this block describes host-managed sources only.\n{}\nEnd gpui-agent guidance.",
            serde_json::json!({"revision":self.snapshot.revision,"agents":self.snapshot.instructions,"selected_skills":self.selected,"available_skills":catalog,"omitted_skills":omitted,"implicit_loading_available":self.implicit_available,"diagnostics":diagnostics,"omitted_diagnostics":omitted_diagnostics})
        )
    }
}

/// A portable copy of a skill folder. Binary assets are preserved verbatim.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillImportRequest {
    pub id: String,
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
}
impl SkillImportRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        let (_, path) = validate_guidance_file_id(&self.id)?;
        if !path.ends_with("/SKILL.md") || self.files.is_empty() || self.files.len() > 128 {
            return Err("Select one skill folder with at most 128 files");
        }
        let mut total = 0usize;
        for (name, bytes) in &self.files {
            validate_skill_resource(name)?;
            // Apply the editor's portable path rules to every resource segment.
            validate_guidance_file_id(&format!("user:skills/import/{name}/SKILL.md"))?;
            if bytes.len() > MAX_SKILL_BYTES || name.split('/').count() > 6 {
                return Err("Skill resource exceeds size or depth limit");
            }
            total += bytes.len();
        }
        if total > 1024 * 1024 {
            return Err("Skill folder exceeds 1 MiB");
        }
        let manifest = self.files.get("SKILL.md").ok_or("Skill folder requires SKILL.md")?;
        let text = std::str::from_utf8(manifest).map_err(|_| "Skill manifest must be UTF-8")?;
        let auxiliary = self
            .files
            .get("agents/openai.yaml")
            .map(|b| std::str::from_utf8(b))
            .transpose()
            .map_err(|_| "Skill metadata must be UTF-8")?;
        SkillMetadata::parse(&self.id, path.rsplit('/').nth(1).ok_or("Missing skill name")?, text, auxiliary)?;
        Ok(())
    }
}
