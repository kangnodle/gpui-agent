use crate::{GuidanceDiagnostic,
            MAX_SKILL_BYTES,
            McpServer,
            SkillMetadata};
use serde::{Deserialize,
            Serialize};
use std::collections::{BTreeMap,
                       BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub enabled: bool,
    pub skills: Vec<SkillMetadata>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginCatalog {
    pub plugins: Vec<Plugin>,
    pub diagnostics: Vec<GuidanceDiagnostic>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginImportRequest {
    pub scope: String,
    #[serde(deserialize_with = "deserialize_files")]
    pub files: BTreeMap<String, Vec<u8>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginClaudeSyncRequest {
    pub scope: String,
    /// Enable what is imported so its skills load immediately.
    #[serde(default)]
    pub enable: bool,
}
/// Outcome of importing plugins installed by the Claude provider profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginSyncResult {
    pub catalog: PluginCatalog,
    pub imported: Vec<String>,
    pub skipped: Vec<GuidanceDiagnostic>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginEnabledRequest {
    pub id: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRemoveRequest {
    pub id: String,
}

pub fn validate_plugin_id(id: &str) -> Result<(&str, &str), &'static str> {
    let (scope, name) = id.split_once(':').ok_or("Invalid plugin ID")?;
    if !matches!(scope, "user" | "repo")
        || name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || name.ends_with('-')
    {
        return Err("Plugin names require a lowercase ASCII slug (1–64)");
    }
    Ok((scope, name))
}

impl PluginImportRequest {
    pub fn validate(&self) -> Result<Plugin, String> {
        if !matches!(self.scope.as_str(), "user" | "repo") || self.files.is_empty() || self.files.len() > 128 {
            return Err("Select user or repo scope and at most 128 files".into());
        }
        let mut paths = BTreeSet::new();
        let mut directories = BTreeSet::new();
        let mut spellings = BTreeMap::new();
        let mut total = 0;
        for (path, bytes) in &self.files {
            crate::validate_skill_resource(path)?;
            if path.split('/').count() > 6
                || bytes.len() > MAX_SKILL_BYTES
                || path.split('/').any(|part| {
                    part.starts_with('.')
                        || part.ends_with(['.', ' '])
                        || part.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
                        || matches!(
                            part.split('.').next().unwrap_or_default().to_ascii_uppercase().as_str(),
                            "CON"
                                | "PRN"
                                | "AUX"
                                | "NUL"
                                | "COM1"
                                | "COM2"
                                | "COM3"
                                | "COM4"
                                | "COM5"
                                | "COM6"
                                | "COM7"
                                | "COM8"
                                | "COM9"
                                | "LPT1"
                                | "LPT2"
                                | "LPT3"
                                | "LPT4"
                                | "LPT5"
                                | "LPT6"
                                | "LPT7"
                                | "LPT8"
                                | "LPT9"
                        )
                })
            {
                return Err("Plugin resource exceeds portable path, size or depth limits".into());
            }
            let mut original = String::new();
            for part in path.split('/') {
                if !original.is_empty() {
                    original.push('/');
                }
                original.push_str(part);
                if spellings
                    .insert(original.to_lowercase(), original.clone())
                    .is_some_and(|previous| previous != original)
                {
                    return Err("Case-colliding plugin paths".into());
                }
            }
            let folded = path.to_lowercase();
            if !paths.insert(folded.clone()) {
                return Err("Case-colliding plugin paths".into());
            }
            let mut prefix = String::new();
            for part in folded.split('/').take(folded.split('/').count() - 1) {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                directories.insert(prefix.clone());
            }
            total += bytes.len();
        }
        if total > 1024 * 1024 || paths.iter().any(|path| directories.contains(path)) {
            return Err("Plugin folder exceeds capacity or has conflicting paths".into());
        }
        let manifest: PluginManifest =
            serde_json::from_slice(self.files.get("plugin.json").ok_or("Plugin requires plugin.json")?).map_err(|_| "Invalid plugin.json")?;
        let id = format!("{}:{}", self.scope, manifest.name);
        validate_plugin_id(&id)?;
        if manifest.description.trim().is_empty()
            || manifest.description.len() > 4096
            || manifest.version.len() > 256
            || manifest.display_name.as_ref().is_some_and(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err("Invalid plugin description, display name or version".into());
        }
        crate::validate_mcp(&manifest.mcp_servers)?;
        let mut skills = Vec::new();
        for (path, bytes) in &self.files {
            let parts: Vec<_> = path.split('/').collect();
            if parts.first() == Some(&"skills") && parts.last() == Some(&"SKILL.md") {
                if parts.len() != 3 {
                    return Err("Plugin skills must use skills/<name>/SKILL.md".into());
                }
                let text = std::str::from_utf8(bytes).map_err(|_| "Skill manifest must be UTF-8")?;
                let auxiliary = self
                    .files
                    .get(&format!("skills/{}/agents/openai.yaml", parts[1]))
                    .map(|b| std::str::from_utf8(b))
                    .transpose()
                    .map_err(|_| "Skill metadata must be UTF-8")?;
                let (skill, _) = SkillMetadata::parse(&format!("plugin:{id}/{path}"), parts[1], text, auxiliary)?;
                skills.push(skill);
            }
        }
        if skills.is_empty() && manifest.mcp_servers.is_empty() {
            return Err("Plugin requires at least one skill or MCP server".into());
        }
        Ok(Plugin {
            id,
            manifest,
            enabled: false,
            skills,
        })
    }
}

fn deserialize_files<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, Vec<u8>>, D::Error> {
    struct Files;
    impl<'de> serde::de::Visitor<'de> for Files {
        type Value = BTreeMap<String, Vec<u8>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str("unique plugin file paths") }

        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut files = BTreeMap::new();
            while let Some((path, bytes)) = access.next_entry::<String, Vec<u8>>()? {
                if files.len() >= 128 || files.insert(path, bytes).is_some() {
                    return Err(serde::de::Error::custom("Duplicate plugin path or too many files"));
                }
            }
            Ok(files)
        }
    }
    deserializer.deserialize_map(Files)
}
