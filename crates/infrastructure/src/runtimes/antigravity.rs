use super::*;

const REGISTRY: &str = "https://raw.githubusercontent.com/agentclientprotocol/registry/main/antigravity-acp/agent.json";

pub(super) fn latest(client: &reqwest::blocking::Client) -> Result<RuntimeRelease> {
    let (os, arch) = platform()?;
    RuntimeRelease::antigravity_from_registry(&json(client, REGISTRY)?, os, arch).map_err(anyhow::Error::msg)
}

pub(super) fn catalog(settings: &Settings, latest: Option<RuntimeRelease>) -> Result<Vec<RuntimeRelease>> {
    // shortcut: known releases only, fetch history for older versions.
    let (os, arch) = platform()?;
    if let Some(release) = &latest {
        release.validate_antigravity(os, arch).map_err(anyhow::Error::msg)?;
    }
    let root = root(settings, Provider::Antigravity);
    let _lock = if latest.is_some() {
        std::fs::create_dir_all(&root)?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("catalog.lock"))?;
        lock.try_lock().context("Another runtime catalog update is in progress")?;
        Some(lock)
    } else {
        None
    };
    let file = match std::fs::File::open(root.join("releases.json")) {
        | Ok(file) => Some(file),
        | Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        | Err(error) => return Err(error.into()),
    };
    let mut releases: Vec<RuntimeRelease> = if let Some(file) = file {
        ensure!(file.metadata()?.len() <= 4 * 1024 * 1024, "Runtime catalog is too large");
        serde_json::from_reader(file.take(4 * 1024 * 1024 + 1)).context("Invalid runtime catalog")?
    } else {
        vec![]
    };
    ensure!(releases.len() <= 4096, "Too many runtime releases");
    let mut versions = std::collections::HashSet::new();
    for release in &releases {
        release.validate_antigravity(os, arch).map_err(anyhow::Error::msg)?;
        ensure!(versions.insert(release.version.clone()), "Duplicate runtime catalog version");
    }
    let vetted: &[(&str, &str, Option<u64>)] = match (os, arch) {
        | ("windows", "x86_64") => &[
            ("1.4.1", "5f5b7d0328df17c128b2be338d4567b0d24e56ace8196e88794fec37d1372554", Some(129599308)),
            ("1.3.0", "65215e0688681fa3116e048a9eab27ef53af1bbd6f3da3f1c52bd4911d8b17f9", None),
        ],
        | ("darwin", "aarch64") => &[("1.4.3", "c57db0a9b8b712daa9c3c17cdb990a61842ea569f279e8bd79d7aa85a303b20b", Some(114147623))],
        | ("darwin", "x86_64") => &[("1.4.3", "9392dbbdad7f0663dbfb5a0bf641297b0cf80d2ee9cd2d49a616bbbc7378731d", Some(119801100))],
        | _ => return Ok(vec![]),
    };
    let directory = if os == "darwin" { "macos" } else { os };
    let archive_arch = if arch == "aarch64" { "arm64" } else { arch };
    for &(version, sha256, size) in vetted {
        if versions.insert(version.into()) {
            releases.push(RuntimeRelease {
                version: version.into(),
                sha256: sha256.into(),
                size,
                url: format!("https://dl.google.com/agy-extensions/releases/{directory}/agy-acp-server-{version}-{os}-{archive_arch}.zip"),
            });
        }
    }
    if let Some(latest) = latest {
        releases.retain(|release| release.version != latest.version);
        releases.insert(0, latest);
        let mut file = tempfile::NamedTempFile::new_in(&root)?;
        serde_json::to_writer(&mut file, &releases)?;
        file.as_file().sync_all()?;
        file.persist(root.join("releases.json")).map_err(|error| error.error)?;
    }
    Ok(releases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_successive_registry_versions_when_checking_then_both_survive_restart_and_invalid_updates() -> Result<()> {
        let (os, arch) = platform()?;
        if !matches!((os, arch), ("windows", "x86_64") | ("darwin", "x86_64" | "aarch64")) {
            return Ok(());
        }
        let temp = tempfile::tempdir()?;
        let settings = Settings::discover(temp.path().into(), Some(temp.path().join("home")))?;
        let directory = if os == "darwin" { "macos" } else { os };
        let archive_arch = if arch == "aarch64" { "arm64" } else { arch };
        let entry = |version: &str, checksum: &str| RuntimeRelease {
            version: version.into(),
            sha256: checksum.repeat(32),
            size: None,
            url: format!("https://dl.google.com/agy-extensions/releases/{directory}/agy-acp-server-{version}-{os}-{archive_arch}.zip"),
        };
        catalog(&settings, Some(entry("8.0.0", "ab")))?;
        catalog(&settings, Some(entry("9.0.0", "cd")))?;
        let restarted = Settings::discover(temp.path().into(), Some(temp.path().join("home")))?;
        let releases = catalog(&restarted, None)?;
        assert_eq!(releases[0].version, "9.0.0");
        assert_eq!(releases[1].version, "8.0.0");
        assert_eq!(releases[1].sha256, "ab".repeat(32));
        assert!(!root(&settings, Provider::Antigravity).join("current.json").exists());
        let updated = catalog(&settings, Some(entry("9.0.0", "ef")))?;
        assert_eq!(updated.iter().filter(|release| release.version == "9.0.0").count(), 1);
        assert_eq!(updated[0].sha256, "ef".repeat(32));
        let cache = root(&settings, Provider::Antigravity).join("releases.json");
        let previous = std::fs::read(&cache)?;
        let mut invalid = entry("10.0.0", "ab");
        invalid.url = "https://evil.test/runtime.zip".into();
        assert!(catalog(&settings, Some(invalid)).is_err());
        assert_eq!(std::fs::read(&cache)?, previous);
        std::fs::write(&cache, br#"[{"version":"../../escape"}]"#)?;
        assert!(catalog(&settings, None).is_err());
        Ok(())
    }
}
