#[test]
fn given_no_native_executable_when_listing_shared_skills_then_the_cli_needs_no_provider() {
    let root = tempfile::tempdir().unwrap();
    let skill = root.path().join("home/skills/verify");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\ndescription: Verify changes\n---\nRun tests").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gpui-agent"))
        .arg("--guidance-home")
        .arg(root.path().join("home"))
        .arg("--cwd")
        .arg(root.path())
        .args(["--executable", "missing-provider-executable", "skills"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(catalog["skills"][0]["name"], "verify");
}
