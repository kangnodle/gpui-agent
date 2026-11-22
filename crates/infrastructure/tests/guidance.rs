use gpui_agent_application::ports::GuidanceSource;
use gpui_agent_infrastructure::GuidanceFiles;
use std::{fs,
          path::Path};

/// Creates a file symlink fixture. Returns `Ok(false)` only when Windows denies
/// the symlink privilege (no Developer Mode or elevation), so callers can skip
/// the link-specific assertions; every other error still fails the test.
fn link_file(target: &Path, link: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link)?;
    #[cfg(windows)]
    {
        const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
        match std::os::windows::fs::symlink_file(target, link) {
            | Ok(()) => {},
            | Err(error) if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) => {
                eprintln!("skip: file symlinks need Developer Mode or elevation: {error}");
                return Ok(false);
            },
            | Err(error) => return Err(error),
        }
    }
    Ok(true)
}

#[test]
fn given_a_disabled_skill_and_linked_paths_when_an_operator_edits_then_validation_preserves_files_and_discovery_scope() -> anyhow::Result<()> {
    use gpui_agent_domain::GuidanceWriteRequest;
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    let cwd = repo.join("nested");
    fs::create_dir_all(repo.join(".git"))?;
    fs::create_dir_all(&cwd)?;
    let source = GuidanceFiles::new(&root.path().join("home"), &cwd)?;
    assert_eq!(source.files()?.project_prefix, "nested/");
    let id = "repo:nested/.agents/skills/verify/SKILL.md";
    let original = "---\ndescription: Verify behavior\n---\nOriginal";
    let saved = source.write_file(&GuidanceWriteRequest {
        id: id.into(),
        text: original.into(),
        expected_revision: None,
    })?;
    source.set_skill_enabled(id, false)?;
    assert_eq!(source.read_file(id)?.text, original, "Disabled skills remain editable");
    assert!(
        source
            .write_file(&GuidanceWriteRequest {
                id: id.into(),
                text: "invalid frontmatter".into(),
                expected_revision: saved.revision
            })
            .is_err()
    );
    assert_eq!(source.read_file(id)?.text, original);
    let agents = GuidanceWriteRequest {
        id: "user:AGENTS.md".into(),
        text: "x".repeat(32 * 1024 + 1),
        expected_revision: None,
    };
    assert!(source.write_file(&agents).is_err());
    let outside = root.path().join("outside.md");
    fs::write(&outside, "Keep outside")?;
    if !link_file(&outside, &cwd.join("AGENTS.md"))? {
        return Ok(());
    }
    assert!(
        source
            .write_file(&GuidanceWriteRequest {
                id: "repo:nested/AGENTS.md".into(),
                text: "escape".into(),
                expected_revision: None
            })
            .is_err()
    );
    assert_eq!(fs::read_to_string(outside)?, "Keep outside");
    Ok(())
}

#[test]
fn given_a_manifest_link_outside_its_skill_when_resources_are_read_then_the_boundary_cannot_expand() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    let skill = repo.join(".agents/skills/verify");
    fs::create_dir_all(&skill)?;
    fs::write(repo.join("SKILL.md"), "---\ndescription: Verify\n---\nTests")?;
    fs::write(repo.join("secret.txt"), "outside the skill")?;
    if !link_file(&repo.join("SKILL.md"), &skill.join("SKILL.md"))? {
        return Ok(());
    }
    let source = GuidanceFiles::new(&home, &repo)?;
    let id = "repo:.agents/skills/verify/SKILL.md";
    assert!(
        source.read_skill(id, "secret.txt").is_err(),
        "A linked manifest must not expose its target directory as skill resources"
    );
    Ok(())
}

#[test]
fn given_nearly_full_settings_when_an_update_exceeds_capacity_then_the_last_readable_state_is_preserved() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    fs::create_dir_all(home.join("skills/verify"))?;
    fs::create_dir_all(&repo)?;
    fs::write(home.join("skills/verify/SKILL.md"), "---\ndescription: Verify\n---\nTests")?;
    let source = GuidanceFiles::new(&home, &repo)?;
    let id = source.snapshot()?.skills[0].id.clone();
    let settings = std::collections::BTreeMap::from([("x".repeat(256 * 1024 - 18), false)]);
    fs::write(home.join("skill-settings.json"), serde_json::to_vec(&settings)?)?;
    assert!(source.set_skill_enabled(&id, false).is_err());
    assert!(source.snapshot()?.skills[0].enabled);
    Ok(())
}

#[test]
fn given_empty_global_and_nonfile_project_overrides_when_discovered_then_normal_guidance_is_used() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(repo.join("AGENTS.override.md"))?;
    fs::write(home.join("AGENTS.override.md"), "  \n")?;
    fs::write(home.join("AGENTS.md"), "global")?;
    fs::write(repo.join("AGENTS.md"), "project")?;
    let source = GuidanceFiles::new(&home, &repo)?;
    let snapshot = source.snapshot()?;
    assert_eq!(
        snapshot.instructions.iter().map(|doc| doc.content.as_str()).collect::<Vec<_>>(),
        ["global", "project"]
    );
    fs::remove_dir(repo.join("AGENTS.override.md"))?;
    fs::write(repo.join("AGENTS.override.md"), "\n")?;
    assert_eq!(source.snapshot()?.instructions.len(), 1, "An empty project override hides its fallback");
    Ok(())
}

#[test]
fn given_a_resource_link_outside_the_skill_when_read_then_the_host_rejects_the_escape() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    let skill = repo.join(".agents/skills/verify");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&skill)?;
    fs::create_dir_all(&outside)?;
    fs::write(skill.join("SKILL.md"), "---\ndescription: Verify\n---\nTests")?;
    fs::write(outside.join("secret.txt"), "must not leave this directory")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, skill.join("escape"))?;
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .current_dir(&skill)
            .args(["/D", "/C", "mklink", "/J", "escape"])
            .arg(&outside)
            .output()?;
        assert!(
            output.status.success(),
            "Junction fixture: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let source = GuidanceFiles::new(&home, &repo)?;
    let id = source.snapshot()?.skills[0].id.clone();
    assert!(source.read_skill(&id, "escape/secret.txt").is_err());
    Ok(())
}

#[test]
fn given_a_multibyte_instruction_crossing_the_budget_when_loaded_then_truncation_is_visible_and_valid_utf8() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo)?;
    fs::write(repo.join("AGENTS.md"), format!("{}😀", "a".repeat(32 * 1024 - 2)))?;
    let snapshot = GuidanceFiles::new(&home, &repo)?.snapshot()?;
    assert_eq!(snapshot.instructions[0].content.len(), 32 * 1024 - 2);
    assert!(snapshot.instructions[0].truncated);
    assert_eq!(snapshot.diagnostics.len(), 1);
    Ok(())
}

#[tokio::test]
async fn given_a_shared_skill_when_a_turn_is_prepared_then_only_the_selected_body_is_injected_and_settings_survive_reopen() -> anyhow::Result<()> {
    use gpui_agent_application::Guidance;
    use gpui_agent_domain::SkillReference;
    use std::sync::Arc;
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    fs::create_dir_all(home.join("skills/verify"))?;
    fs::create_dir_all(&repo)?;
    fs::write(home.join("skills/verify/SKILL.md"), "---\ndescription: Run checks\n---\nPRIVATE_SKILL_BODY")?;
    let source = Arc::new(GuidanceFiles::new(&home, &repo)?);
    let guidance = Guidance::new(source.clone());
    let plain = guidance.prepare("Review", &[], true).await?;
    assert!(!plain.render().contains("PRIVATE_SKILL_BODY"));
    let selected = guidance.prepare("Review with $verify", &[], true).await?;
    assert!(selected.render().contains("PRIVATE_SKILL_BODY"));
    let id = selected.selected[0].skill.id.clone();
    guidance.set_skill_enabled(&id, false).await?;
    let reopened = Guidance::new(Arc::new(GuidanceFiles::new(&home, &repo)?));
    assert!(!reopened.snapshot().await?.skills[0].enabled);
    assert!(
        reopened
            .prepare(
                "Review",
                &[SkillReference {
                    id
                }],
                true
            )
            .await
            .is_err()
    );
    Ok(())
}

#[test]
fn given_global_and_nested_guidance_when_loaded_then_overrides_and_skills_are_scoped_without_a_provider() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    fs::create_dir_all(home.join("skills/global"))?;
    fs::create_dir_all(repo.join(".git"))?;
    fs::create_dir_all(repo.join("child/.agents/skills/local"))?;
    fs::write(home.join("AGENTS.md"), "global guidance")?;
    fs::write(repo.join("AGENTS.md"), "root guidance")?;
    fs::write(repo.join("child/AGENTS.md"), "ignored guidance")?;
    fs::write(repo.join("child/AGENTS.override.md"), "child guidance")?;
    fs::write(
        home.join("skills/global/SKILL.md"),
        "---\nname: verify\ndescription: Global workflow\n---\nGlobal body.",
    )?;
    fs::write(
        repo.join("child/.agents/skills/local/SKILL.md"),
        "---\nname: verify\ndescription: Local workflow\n---\nLocal body.",
    )?;
    let source = GuidanceFiles::new(&home, &repo.join("child"))?;
    let snapshot = source.snapshot()?;
    assert_eq!(
        snapshot.instructions.iter().map(|doc| doc.content.as_str()).collect::<Vec<_>>(),
        ["global guidance", "root guidance", "child guidance"]
    );
    assert_eq!(snapshot.skills.len(), 2, "Different paths with the same name remain separate");
    let local = snapshot.skills.iter().find(|skill| skill.id.starts_with("repo:")).unwrap();
    assert_eq!(source.read_skill(&local.id, "SKILL.md")?.text, "Local body.");
    assert!(source.read_skill(&local.id, "../../AGENTS.md").is_err());
    assert!(source.read_skill("user:../AGENTS.md", "SKILL.md").is_err());
    fs::remove_file(repo.join("child/AGENTS.override.md"))?;
    assert_eq!(source.snapshot()?.instructions.last().unwrap().content, "ignored guidance");
    Ok(())
}

#[test]
fn given_a_skill_folder_when_copied_then_resources_are_independent_and_unsafe_bundles_leave_no_install() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let original = root.path().join("original");
    fs::create_dir_all(original.join("references"))?;
    fs::write(original.join("SKILL.md"), "---\ndescription: Verify changes\n---\nRead references/check.md")?;
    fs::write(original.join("references/check.md"), "Original check")?;
    let source = GuidanceFiles::new(&root.path().join("home"), root.path())?;
    let id = "user:skills/copied/SKILL.md";
    let mut bundle = GuidanceFiles::collect_skill(&original, id.into())?;
    source.import_skill(&bundle)?;
    fs::write(original.join("references/check.md"), "Changed original")?;
    assert_eq!(source.read_skill(id, "references/check.md")?.text, "Original check");
    assert!(source.import_skill(&bundle).is_err());
    bundle.id = "user:skills/unsafe/SKILL.md".into();
    bundle.files.insert("../escape".into(), vec![1]);
    assert!(source.import_skill(&bundle).is_err());
    assert!(!root.path().join("home/skills/unsafe").exists());
    Ok(())
}
