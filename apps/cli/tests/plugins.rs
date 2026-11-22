fn cli(root: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_gpui-agent"))
        .arg("--guidance-home")
        .arg(root.join("home"))
        .arg("--home")
        .arg(root.join("home/codex"))
        .arg("--cwd")
        .arg(root.join("repo"))
        .args(["--executable", "missing-provider-executable"])
        .args(arguments)
        .output()
        .unwrap()
}

fn catalog(root: &std::path::Path, arguments: &[&str]) -> serde_json::Value {
    let output = cli(root, arguments);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn given_legacy_settings_when_managing_a_plugin_without_a_provider_then_activation_and_bytes_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let standalone = home.join("skills/standalone/SKILL.md");
    let settings = home.join("skill-settings.json");
    let mcp = home.join("codex/mcp.json");
    let source = root.path().join("source");
    std::fs::create_dir_all(standalone.parent().unwrap()).unwrap();
    std::fs::create_dir_all(mcp.parent().unwrap()).unwrap();
    std::fs::create_dir_all(root.path().join("repo")).unwrap();
    std::fs::create_dir_all(source.join("skills/review")).unwrap();
    let standalone_bytes = b"---\ndescription: Existing skill\n---\nKeep these bytes\n";
    let settings_bytes = b"{ \"user:skills/standalone/SKILL.md\": true }\n";
    let mcp_bytes = b"[{\"name\":\"legacy\",\"url\":\"https://example.invalid/mcp\"}]\n";
    std::fs::write(&standalone, standalone_bytes).unwrap();
    std::fs::write(&settings, settings_bytes).unwrap();
    std::fs::write(&mcp, mcp_bytes).unwrap();
    std::fs::write(source.join("plugin.json"), r#"{"name":"review-tools","description":"Review workflows"}"#).unwrap();
    std::fs::write(
        source.join("skills/review/SKILL.md"),
        "---\ndescription: Review changes\n---\nReview the diff\n",
    )
    .unwrap();

    let imported = catalog(root.path(), &["plugins", "--import", source.to_str().unwrap()]);
    assert_eq!(imported["plugins"][0]["id"], "repo:review-tools");
    assert_eq!(imported["plugins"][0]["enabled"], false);
    assert_eq!(catalog(root.path(), &["plugins"]), imported);
    let skills = catalog(root.path(), &["skills"]);
    assert_eq!(skills["skills"].as_array().unwrap().len(), 1);

    let enabled = catalog(root.path(), &["plugins", "--enable", "repo:review-tools"]);
    assert_eq!(enabled["plugins"][0]["enabled"], true);
    let skills = catalog(root.path(), &["skills"]);
    assert!(skills["skills"].as_array().unwrap().iter().any(|skill| skill["name"] == "review"));

    let disabled = catalog(root.path(), &["plugins", "--disable", "repo:review-tools"]);
    assert_eq!(disabled["plugins"][0]["enabled"], false);
    assert_eq!(catalog(root.path(), &["skills"])["skills"].as_array().unwrap().len(), 1);
    let removed = catalog(root.path(), &["plugins", "--remove", "repo:review-tools"]);
    assert!(removed["plugins"].as_array().unwrap().is_empty());
    assert_eq!(std::fs::read(&standalone).unwrap(), standalone_bytes);
    assert_eq!(std::fs::read(&settings).unwrap(), settings_bytes);
    assert_eq!(std::fs::read(&mcp).unwrap(), mcp_bytes);

    let imported = catalog(root.path(), &["plugins", "--import", source.to_str().unwrap(), "--scope", "user"]);
    assert_eq!(imported["plugins"][0]["id"], "user:review-tools");
    assert!(
        !cli(root.path(), &["plugins", "--enable", "user:review-tools", "--remove", "user:review-tools"])
            .status
            .success()
    );
    assert!(
        !cli(root.path(), &["plugins", "--import", source.to_str().unwrap(), "--scope", "invalid"])
            .status
            .success()
    );
}
