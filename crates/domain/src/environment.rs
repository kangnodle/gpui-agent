use serde::{Deserialize,
            Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EnvironmentPreferences {
    pub label: String,
    pub default_worktree_base: String,
}

impl Default for EnvironmentPreferences {
    fn default() -> Self {
        Self {
            label: "gpui-agent".into(),
            default_worktree_base: "HEAD".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPreferencesPatch {
    pub label: Option<String>,
    pub default_worktree_base: Option<String>,
}

impl EnvironmentPreferences {
    pub fn validate(&self) -> Result<(), &'static str> {
        crate::normalize_title(&self.label)?;
        let base = &self.default_worktree_base;
        if base.is_empty() || base.len() > 512 || base.trim() != base || base.starts_with('-') || base.chars().any(char::is_control) {
            return Err("Default worktree base requires 1–512 bytes without surrounding whitespace, control characters or a leading hyphen");
        }
        Ok(())
    }

    pub fn patched(&self, patch: EnvironmentPreferencesPatch) -> Result<Self, &'static str> {
        if patch.label.is_none() && patch.default_worktree_base.is_none() {
            return Err("Supply at least one environment preference");
        }
        let updated = Self {
            label: patch.label.map(|label| label.trim().to_owned()).unwrap_or_else(|| self.label.clone()),
            default_worktree_base: patch.default_worktree_base.unwrap_or_else(|| self.default_worktree_base.clone()),
        };
        updated.validate()?;
        Ok(updated)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvironmentPlatform {
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnvironmentDescriptor {
    pub environment_id: String,
    pub server_version: String,
    pub platform: EnvironmentPlatform,
    pub preferences: EnvironmentPreferences,
}
