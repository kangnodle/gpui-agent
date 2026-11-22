use serde::{Deserialize,
            Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeCommand {
    Inspect,
    Check,
    Install { version: String, sha256: String },
}

pub fn valid_runtime_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 80
        && version.as_bytes()[0].is_ascii_digit()
        && version.bytes().all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
}

impl RuntimeCommand {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Self::Install {
            version,
            sha256,
        } = self
            && (!valid_runtime_version(version) || sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("Invalid runtime version or checksum");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeRelease {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: Option<u64>,
}
impl RuntimeRelease {
    pub fn antigravity_from_registry(manifest: &serde_json::Value, os: &str, arch: &str) -> Result<Self, &'static str> {
        if manifest["id"] != "antigravity-acp" {
            return Err("Invalid Antigravity registry entry");
        }
        let entry = &manifest["distribution"]["binary"][format!("{os}-{arch}")];
        let release = Self {
            version: manifest["version"].as_str().ok_or("Missing Antigravity version")?.into(),
            url: entry["archive"].as_str().ok_or("No supported Antigravity archive")?.into(),
            sha256: entry["sha256"].as_str().ok_or("Missing Antigravity checksum")?.to_ascii_lowercase(),
            size: None,
        };
        release.validate_antigravity(os, arch)?;
        Ok(release)
    }

    pub fn validate_antigravity(&self, os: &str, arch: &str) -> Result<(), &'static str> {
        let (directory, archive_arch) = match (os, arch) {
            | ("windows", "x86_64") => ("windows", "x86_64"),
            | ("darwin", "x86_64") => ("macos", "x86_64"),
            | ("darwin", "aarch64") => ("macos", "arm64"),
            | _ => return Err("Automatic Antigravity installation supports Windows x64 and macOS x64/arm64; use an explicit runtime path on other platforms"),
        };
        let expected = format!(
            "https://dl.google.com/agy-extensions/releases/{directory}/agy-acp-server-{}-{os}-{archive_arch}.zip",
            self.version
        );
        if !valid_runtime_version(&self.version) || self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) || self.url != expected {
            return Err("Invalid Antigravity release metadata");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub provider: crate::Provider,
    pub active_path: Option<String>,
    pub managed_version: Option<String>,
    pub override_active: bool,
    pub release: Option<RuntimeRelease>,
    #[serde(default)]
    pub releases: Vec<RuntimeRelease>,
    pub issue: Option<String>,
}
impl RuntimeInfo {
    pub fn available_releases(&self) -> &[RuntimeRelease] { if self.releases.is_empty() { self.release.as_slice() } else { &self.releases } }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_requires_bounded_version_and_exact_hex_checksum() {
        for version in ["1.2.3", "1.0.0-beta.1"] {
            assert!(
                RuntimeCommand::Install {
                    version: version.into(),
                    sha256: "A0".repeat(32)
                }
                .validate()
                .is_ok()
            );
        }
        for version in ["", "../1", "-1", "v1.2", "1/2", "1\0", &"1".repeat(81)] {
            assert!(
                RuntimeCommand::Install {
                    version: version.into(),
                    sha256: "00".repeat(32)
                }
                .validate()
                .is_err()
            );
        }
        for sha256 in ["", &"0".repeat(63), &"0".repeat(65), &"g".repeat(64)] {
            assert!(
                RuntimeCommand::Install {
                    version: "1.2.3".into(),
                    sha256: sha256.into()
                }
                .validate()
                .is_err()
            );
        }
        assert!(RuntimeCommand::Inspect.validate().is_ok());
        assert!(RuntimeCommand::Check.validate().is_ok());
    }
}
