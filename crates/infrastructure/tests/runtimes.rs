#![cfg(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", any(target_arch = "aarch64", target_arch = "x86_64"))
))]

use anyhow::Result;
use gpui_agent_contracts::{Provider,
                           RuntimeCommand};
use gpui_agent_infrastructure::Settings;

#[tokio::test]
async fn given_no_antigravity_installation_when_inspecting_then_vetted_versions_are_offered_without_network_or_installation() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let settings = Settings::discover(temp.path().into(), Some(temp.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let info = workspaces.runtime(Provider::Antigravity, RuntimeCommand::Inspect, || Ok(())).await?;
    let release = info
        .release
        .expect("A supported release must remain installable when upstream publishes a newer version");
    let (version, platform, sha256) = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            (
                "1.4.3",
                "macos/agy-acp-server-1.4.3-darwin-arm64.zip",
                "c57db0a9b8b712daa9c3c17cdb990a61842ea569f279e8bd79d7aa85a303b20b",
            )
        } else {
            (
                "1.4.3",
                "macos/agy-acp-server-1.4.3-darwin-x86_64.zip",
                "9392dbbdad7f0663dbfb5a0bf641297b0cf80d2ee9cd2d49a616bbbc7378731d",
            )
        }
    } else {
        (
            "1.4.1",
            "windows/agy-acp-server-1.4.1-windows-x86_64.zip",
            "5f5b7d0328df17c128b2be338d4567b0d24e56ace8196e88794fec37d1372554",
        )
    };
    assert_eq!(release.version, version);
    assert_eq!(release.sha256, sha256);
    assert_eq!(release.url, format!("https://dl.google.com/agy-extensions/releases/{platform}"));
    assert!(!settings.runtime_root.exists(), "Checking a release must not install it");
    Ok(())
}

#[tokio::test]
async fn given_a_cached_new_version_when_restarting_then_old_and_new_versions_remain_installable() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let settings = Settings::discover(temp.path().into(), Some(temp.path().join("home")))?;
    let root = settings.runtime_root.join("antigravity");
    std::fs::create_dir_all(&root)?;
    let (os, directory, arch) = if cfg!(target_os = "windows") {
        ("windows", "windows", "x86_64")
    } else {
        ("darwin", "macos", if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" })
    };
    std::fs::write(
        root.join("releases.json"),
        serde_json::to_vec(&serde_json::json!([
            {"version":"9.0.0", "sha256":"ab".repeat(32), "size":null,
             "url":format!("https://dl.google.com/agy-extensions/releases/{directory}/agy-acp-server-9.0.0-{os}-{arch}.zip")}
        ]))?,
    )?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let info = workspaces.runtime(Provider::Antigravity, RuntimeCommand::Inspect, || Ok(())).await?;
    let versions: Vec<_> = info.releases.iter().map(|release| release.version.as_str()).collect();
    assert_eq!(versions[0], "9.0.0");
    assert!(versions.contains(&if cfg!(target_os = "windows") { "1.4.1" } else { "1.4.3" }));
    // Given a reviewed release, a changed checksum cannot trigger a
    // download/activation.
    assert!(
        workspaces
            .runtime(
                Provider::Antigravity,
                RuntimeCommand::Install {
                    version: "9.0.0".into(),
                    sha256: "cd".repeat(32)
                },
                || Ok(())
            )
            .await
            .is_err()
    );
    assert!(!root.join("current.json").exists());
    Ok(())
}

#[tokio::test]
#[ignore = "Downloads and validates real Google runtimes in an isolated temporary home"]
async fn given_live_google_releases_when_installing_latest_then_a_vetted_previous_version_can_be_reinstalled() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let settings = Settings::discover(temp.path().into(), Some(temp.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let checked = workspaces.runtime(Provider::Antigravity, RuntimeCommand::Check, || Ok(())).await?;
    let latest = checked.releases[0].clone();
    let previous = checked.releases[1].clone();
    let mut paths = Vec::new();
    for release in [latest, previous] {
        let installed = workspaces
            .runtime(
                Provider::Antigravity,
                RuntimeCommand::Install {
                    version: release.version.clone(),
                    sha256: release.sha256,
                },
                || Ok(()),
            )
            .await?;
        assert_eq!(installed.managed_version.as_deref(), Some(release.version.as_str()));
        assert!(installed.issue.is_none(), "{:?}", installed.issue);
        paths.push(installed.active_path.unwrap());
        println!("Validated and activated Antigravity {} in an isolated home", release.version);
    }
    assert!(paths.iter().all(|path| std::path::Path::new(path).is_file()));
    Ok(())
}
