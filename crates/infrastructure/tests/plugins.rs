use gpui_agent_application::ports::{GuidanceSource,
                                    PluginSource};
use gpui_agent_domain::*;
use gpui_agent_infrastructure::{GuidanceFiles,
                                PluginFiles};
use std::{collections::BTreeMap,
          fs,
          path::Path};

fn link_directory(directory: &Path, link: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(directory, link)?;
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .current_dir(link.parent().unwrap())
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link.file_name().unwrap())
            .arg(directory)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "Could not create junction fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn bundle(scope: &str, name: &str) -> PluginImportRequest {
    PluginImportRequest {
        scope: scope.into(),
        files: BTreeMap::from([
            (
                "plugin.json".into(),
                format!(r#"{{"name":"{name}","description":"Fixture tools","mcp_servers":[{{"name":"fixture","url":"https://example.invalid/mcp"}}]}}"#)
                    .into_bytes(),
            ),
            ("skills/check/SKILL.md".into(), b"---\ndescription: Check fixture\n---\nCheck it".to_vec()),
            ("skills/check/assets/raw.bin".into(), vec![0, 255, 1]),
        ]),
    }
}

#[tokio::test]
async fn given_installed_extensions_when_conversation_selection_changes_then_only_selected_guidance_is_prepared_without_writes() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo)?;
    fs::create_dir_all(home.join("skills/legacy"))?;
    fs::write(home.join("skills/legacy/SKILL.md"), "---\ndescription: Legacy\n---\nLegacy instructions")?;
    let plugins = PluginFiles::new(&home, &repo)?;
    plugins.import(&bundle("repo", "review"))?;
    plugins.set_enabled("repo:review", true)?;
    let before = serde_json::to_value(plugins.catalog()?)?;
    let source = std::sync::Arc::new(GuidanceFiles::new(&home, &repo)?);
    let guidance = gpui_agent_application::Guidance::new(source.clone());
    let empty = ConversationExtensions::default();
    let prepared = guidance.prepare_for_conversation("hello", &[], true, Some(&empty)).await?;
    assert!(prepared.snapshot.skills.is_empty());
    assert_eq!(prepared.receipt().extensions, Some(empty));
    let selected = ConversationExtensions {
        plugins: vec!["repo:review".into()],
        mcp: vec![],
    };
    let prepared = guidance.prepare_for_conversation("hello", &[], true, Some(&selected)).await?;
    assert_eq!(prepared.snapshot.skills.len(), 1);
    assert_eq!(prepared.snapshot.skills[0].id, "plugin:repo:review/skills/check/SKILL.md");
    let explicit = [SkillReference {
        id: "user:skills/legacy/SKILL.md".into(),
    }];
    let prepared = guidance
        .prepare_for_conversation("hello", &explicit, true, Some(&ConversationExtensions::default()))
        .await?;
    assert_eq!(prepared.selected[0].text, "Legacy instructions");
    assert_eq!(prepared.snapshot.skills.len(), 1);
    assert_eq!(guidance.prepare("hello", &[], true).await?.snapshot.skills.len(), 2);
    assert_eq!(serde_json::to_value(plugins.catalog()?)?, before);
    assert!(source.snapshot()?.skills.iter().all(|skill| skill.enabled));
    Ok(())
}

#[test]
fn given_an_enabled_package_when_reopened_and_removed_then_scope_resources_and_standalone_bytes_are_preserved() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let repo = root.path().join("repo");
    let other = root.path().join("other");
    fs::create_dir_all(&repo)?;
    fs::create_dir(root.path().join(".git"))?;
    fs::create_dir_all(&other)?;
    let plugins = PluginFiles::new(&home, &repo)?;
    fs::create_dir_all(home.join("skills/legacy"))?;
    fs::write(home.join("skills/legacy/SKILL.md"), "---\ndescription: Legacy\n---\nLegacy")?;
    let settings = b"{ \"user:skills/legacy/SKILL.md\" : false }\n";
    fs::write(home.join("skill-settings.json"), settings)?;
    plugins.import(&bundle("repo", "review"))?;
    assert!(repo.join(".agents/plugins/review/plugin.json").is_file());
    assert!(!root.path().join(".agents/plugins").exists());
    assert!(plugins.mcp_servers()?.is_empty());
    plugins.set_enabled("repo:review", true)?;
    let reopened = PluginFiles::new(&home, &repo)?;
    assert_eq!(reopened.mcp_servers()?.len(), 1);
    assert!(PluginFiles::new(&home, &other)?.catalog()?.plugins.is_empty());
    assert!(
        GuidanceFiles::new(&home, &other)?
            .snapshot()?
            .skills
            .iter()
            .all(|skill| !skill.id.starts_with("plugin:repo:"))
    );
    let guidance = GuidanceFiles::new(&home, &repo)?;
    let skill = guidance.snapshot()?.skills.into_iter().find(|s| s.id.starts_with("plugin:")).unwrap();
    assert_eq!(guidance.read_skill(&skill.id, "SKILL.md")?.text, "Check it");
    assert!(
        validate_skill_references(&[SkillReference {
            id: skill.id.clone()
        }])
        .is_ok()
    );
    assert!(guidance.read_skill(&skill.id, "../plugin.json").is_err());
    plugins.set_enabled("repo:review", false)?;
    assert!(guidance.read_skill(&skill.id, "SKILL.md").is_err());
    plugins.remove("repo:review")?;
    assert!(plugins.catalog()?.plugins.is_empty());
    assert_eq!(fs::read(home.join("skill-settings.json"))?, settings);
    assert!(home.join("skills/legacy/SKILL.md").exists());
    Ok(())
}

#[test]
fn repo_mcp_requires_host_approval_for_the_workspace_and_current_capabilities() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let repo = root.path().join("first");
    let other = home.join("worktrees/group/second");
    fs::create_dir(&repo)?;
    fs::create_dir_all(&other)?;
    let plugins = PluginFiles::new(&home, &repo)?;
    let other_plugins = PluginFiles::new(&home, &other)?;
    let package = bundle("repo", "review");
    for (directory, source) in [(&repo, &plugins), (&other, &other_plugins)] {
        source.import(&package)?;
        fs::write(directory.join(".agents/plugin-settings.json"), r#"{"review":true}"#)?;
    }
    let selected = ConversationExtensions {
        plugins: vec!["repo:review".into()],
        mcp: vec![],
    };
    assert!(plugins.mcp_servers()?.is_empty());
    assert!(selected.mcp_names(&plugins.catalog()?).is_err());
    plugins.set_enabled("repo:review", true)?;
    assert_eq!(plugins.mcp_servers()?.len(), 1);
    assert!(other_plugins.mcp_servers()?.is_empty());
    let manifest = repo.join(".agents/plugins/review/plugin.json");
    let original = fs::read_to_string(&manifest)?;
    fs::write(&manifest, original.replace("example.invalid", "updated.invalid"))?;
    assert!(plugins.mcp_servers()?.is_empty());
    assert!(selected.mcp_names(&plugins.catalog()?).is_err());
    plugins.set_enabled("repo:review", true)?;
    assert_eq!(selected.mcp_names(&plugins.catalog()?).map_err(anyhow::Error::msg)?, ["fixture"]);
    fs::write(manifest, original)?;
    assert!(plugins.mcp_servers()?.is_empty());
    other_plugins.set_enabled("repo:review", true)?;
    assert_eq!(other_plugins.mcp_servers()?.len(), 1);
    other_plugins.set_enabled("repo:review", false)?;
    fs::write(other.join(".agents/plugin-settings.json"), r#"{"review":true}"#)?;
    assert!(other_plugins.mcp_servers()?.is_empty());
    Ok(())
}

#[test]
fn repo_mcp_approval_requires_a_guidance_home_outside_the_workspace() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    fs::create_dir(&repo)?;
    let external = root.path().join("external");
    fs::create_dir(&external)?;
    let linked_home = repo.join("linked-home");
    link_directory(&external, &linked_home)?;
    let inside = repo.join("home");
    let plugins = PluginFiles::new(&inside, &repo)?;
    plugins.import(&bundle("repo", "review"))?;
    for home in [inside, linked_home] {
        let plugins = PluginFiles::new(&home, &repo)?;
        assert!(plugins.set_enabled("repo:review", true).is_err());
        assert!(plugins.mcp_servers()?.is_empty());
    }
    Ok(())
}

#[test]
fn given_invalid_and_case_colliding_packages_when_imported_then_no_partial_install_is_visible() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let plugins = PluginFiles::new(&root.path().join("home"), root.path())?;
    for path in ["../escape", "/absolute", "a/../../escape", "CON", "a\\escape"] {
        let mut request = bundle("user", "review");
        request.files.insert(path.into(), vec![1]);
        assert!(plugins.import(&request).is_err(), "{path}");
    }
    let mut request = bundle("user", "review");
    request.files.insert("assets/Foo/a".into(), vec![1]);
    request.files.insert("assets/foo/b".into(), vec![1]);
    assert!(plugins.import(&request).is_err());
    assert!(plugins.catalog()?.plugins.is_empty());
    Ok(())
}

#[test]
fn given_legacy_and_package_server_name_collisions_when_enabling_then_existing_configuration_is_preserved() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let plugins = PluginFiles::new(&home, root.path())?;
    fs::create_dir_all(home.join("codex"))?;
    let legacy = br#"[{"name":"fixture","url":"https://example.invalid/mcp"}]"#;
    fs::write(home.join("codex/mcp.json"), legacy)?;
    plugins.import(&bundle("user", "review"))?;
    assert!(plugins.set_enabled("user:review", true).is_err());
    assert!(!plugins.catalog()?.plugins[0].enabled);
    assert_eq!(fs::read(home.join("codex/mcp.json"))?, legacy);
    Ok(())
}

#[test]
fn given_a_linked_installed_resource_when_collected_or_removed_then_outside_files_are_untouched() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let plugins = PluginFiles::new(&home, root.path())?;
    plugins.import(&bundle("user", "review"))?;
    let package = home.join("plugins/review");
    let outside = root.path().join("outside");
    fs::create_dir(&outside)?;
    fs::write(outside.join("keep.txt"), "untouched")?;
    link_directory(&outside, &package.join("escape"))?;
    assert!(PluginFiles::collect_plugin(&package, "user".into()).is_err());
    assert!(plugins.remove("user:review").is_err());
    assert!(plugins.mcp_servers()?.is_empty());
    assert_eq!(fs::read_to_string(outside.join("keep.txt"))?, "untouched");
    #[cfg(unix)]
    fs::remove_file(package.join("escape"))?;
    #[cfg(windows)]
    fs::remove_dir(package.join("escape"))?;
    plugins.remove("user:review")?;
    Ok(())
}

#[test]
fn given_an_enabled_package_corrupted_after_import_when_disabled_then_its_diagnostic_does_not_block_other_runtime_servers() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let plugins = PluginFiles::new(&home, root.path())?;
    plugins.import(&bundle("user", "review"))?;
    plugins.set_enabled("user:review", true)?;
    fs::write(home.join("plugins/review/plugin.json"), "invalid JSON")?;
    assert!(plugins.mcp_servers().is_err());
    let catalog = plugins.set_enabled("user:review", false)?;
    assert_eq!(catalog.diagnostics.len(), 1);
    assert!(plugins.mcp_servers()?.is_empty());
    plugins.remove("user:review")?;
    assert!(plugins.catalog()?.diagnostics.is_empty());
    Ok(())
}

#[test]
fn given_a_claude_plugin_folder_when_collected_then_only_manifest_and_skills_become_a_native_bundle() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("superpowers");
    fs::create_dir_all(source.join(".claude-plugin"))?;
    fs::create_dir_all(source.join("skills/check/assets"))?;
    fs::create_dir_all(source.join("hooks"))?;
    fs::write(
        source.join(".claude-plugin/plugin.json"),
        r#"{"name":"superpowers","description":"Core skills","version":"7.0.1","author":{"name":"x"},"keywords":["a"]}"#,
    )?;
    fs::write(source.join("skills/check/SKILL.md"), "---\ndescription: Check fixture\n---\nCheck it")?;
    fs::write(source.join("skills/check/assets/raw.bin"), [0u8, 255, 1])?;
    fs::write(source.join("hooks/run.sh"), "echo hi")?;
    fs::write(source.join("index.js"), "ignored")?;

    let request = PluginFiles::collect_plugin(&source, "user".into())?;
    let paths: Vec<_> = request.files.keys().map(String::as_str).collect();
    assert_eq!(paths, ["plugin.json", "skills/check/SKILL.md", "skills/check/assets/raw.bin"]);
    let plugin = request.validate().map_err(anyhow::Error::msg)?;
    assert_eq!(plugin.id, "user:superpowers");
    assert_eq!(plugin.manifest.version, "7.0.1");
    assert!(!plugin.enabled);

    let home = root.path().join("home");
    let plugins = PluginFiles::new(&home, root.path())?;
    let catalog = plugins.import(&request)?;
    assert_eq!(catalog.plugins.len(), 1);
    assert!(!catalog.plugins[0].enabled);
    Ok(())
}

#[test]
fn given_a_claude_plugin_without_skills_when_collected_then_it_is_rejected() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join(".claude-plugin"))?;
    fs::write(root.path().join(".claude-plugin/plugin.json"), r#"{"name":"hooks-only","description":"Hooks"}"#)?;
    assert!(PluginFiles::collect_plugin(root.path(), "user".into()).is_err());
    Ok(())
}

#[test]
fn given_claude_installed_plugins_when_synced_then_valid_ones_are_imported_once_and_the_rest_are_reported() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let cache = home.join("claude/plugins/cache/market");
    for (name, skill) in [("good", true), ("hooks-only", false)] {
        let dir = cache.join(name).join("1.0.0");
        fs::create_dir_all(dir.join(".claude-plugin"))?;
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(r#"{{"name":"{name}","description":"Fixture","version":"1.0.0"}}"#),
        )?;
        if skill {
            fs::create_dir_all(dir.join("skills/check"))?;
            fs::write(dir.join("skills/check/SKILL.md"), "---\ndescription: Check\n---\nCheck it")?;
        }
    }
    let outside = root.path().join("outside");
    fs::create_dir_all(outside.join(".claude-plugin"))?;
    fs::write(outside.join(".claude-plugin/plugin.json"), r#"{"name":"outside","description":"Elsewhere"}"#)?;
    fs::create_dir_all(outside.join("skills/x"))?;
    fs::write(outside.join("skills/x/SKILL.md"), "---\ndescription: X\n---\nX")?;
    let entry = |path: &std::path::Path| serde_json::json!([{"scope":"user","installPath":path,"version":"1.0.0"}]);
    fs::write(
        home.join("claude/plugins/installed_plugins.json"),
        serde_json::to_vec(&serde_json::json!({"version":2,"plugins":{
            "good@market": entry(&cache.join("good/1.0.0")),
            "hooks-only@market": entry(&cache.join("hooks-only/1.0.0")),
            "outside@market": entry(&outside),
        }}))?,
    )?;
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo)?;
    let plugins = PluginFiles::new(&home, &repo)?;

    let first = plugins.sync_claude("user", false)?;
    assert_eq!(first.imported, ["user:good"]);
    let mut skipped: Vec<_> = first.skipped.iter().map(|d| d.source.as_str()).collect();
    skipped.sort();
    assert_eq!(skipped, ["hooks-only@market", "outside@market"]);
    assert_eq!(first.catalog.plugins.len(), 1);
    assert!(!first.catalog.plugins[0].enabled);

    let second = plugins.sync_claude("user", false)?;
    assert!(second.imported.is_empty());
    assert!(second.skipped.iter().any(|d| d.source == "good@market"));
    assert_eq!(second.catalog.plugins.len(), 1);
    Ok(())
}

#[test]
fn given_no_claude_profile_when_synced_then_nothing_is_imported() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let plugins = PluginFiles::new(&root.path().join("home"), root.path())?;
    let result = plugins.sync_claude("user", false)?;
    assert!(result.imported.is_empty() && result.skipped.is_empty() && result.catalog.plugins.is_empty());
    Ok(())
}

#[test]
fn claude_sync_imports_mcp_packages_for_review_in_both_scopes() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let repo = root.path().join("repo");
    fs::create_dir(&repo)?;
    let install = home.join("claude/plugins/cache/fixture");
    fs::create_dir_all(&install)?;
    fs::write(install.join("plugin.json"), &bundle("user", "review").files["plugin.json"])?;
    fs::write(
        home.join("claude/plugins/installed_plugins.json"),
        serde_json::to_vec(&serde_json::json!({"plugins":{"review@fixture":[{"installPath": install}]}}))?,
    )?;
    let plugins = PluginFiles::new(&home, &repo)?;
    for scope in ["user", "repo"] {
        let result = plugins.sync_claude(scope, true)?;
        assert_eq!(result.imported, [format!("{scope}:review")]);
        assert!(result.catalog.plugins.iter().all(|plugin| !plugin.enabled));
        assert!(plugins.mcp_servers()?.is_empty());
    }
    Ok(())
}

#[test]
fn given_a_skill_plugin_installed_by_claude_when_synced_with_enable_then_it_is_enabled_and_its_skills_load() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let install = home.join("claude/plugins/cache/market/review/1.0.0");
    fs::create_dir_all(install.join(".claude-plugin"))?;
    fs::create_dir_all(install.join("skills/check"))?;
    fs::write(install.join(".claude-plugin/plugin.json"), r#"{"name":"review","description":"Review tools"}"#)?;
    fs::write(install.join("skills/check/SKILL.md"), "---\ndescription: Check\n---\nCheck it")?;
    fs::write(
        home.join("claude/plugins/installed_plugins.json"),
        serde_json::to_vec(&serde_json::json!({"plugins":{"review@market":[{"installPath":install}]}}))?,
    )?;
    let plugins = PluginFiles::new(&home, root.path())?;
    let result = plugins.sync_claude("user", true)?;
    assert_eq!(result.imported, ["user:review"]);
    assert!(result.catalog.plugins[0].enabled);
    assert_eq!(plugins.catalog()?.plugins[0].skills.len(), 1);
    Ok(())
}
