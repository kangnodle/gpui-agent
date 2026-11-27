//! Exercise the task entry point with a fake Cargo process, without compiling
//! the workspace recursively or launching the desktop during the test.
mod support;
use std::{fs,
          process::Command};

#[test]
fn given_mise_tasks_when_cargo_fails_then_arguments_working_directory_and_exit_are_preserved() {
    if Command::new("mise").arg("--version").output().is_err() {
        assert!(std::env::var_os("CI").is_none(), "CI must install mise to test the build entry point");
        eprintln!("skip: install mise to test the task entry point");
        return;
    }
    for (task, args, expected) in [
        ("check", vec!["--include-desktop"], vec!["harness", "check", "--include-desktop"]),
        (
            "test",
            vec!["--package", "gpui-agent-domain", "--filter", "name with spaces"],
            vec!["harness", "test", "--package", "gpui-agent-domain", "--filter", "name with spaces"],
        ),
        (
            "build",
            vec!["--release"],
            vec![
                "build",
                "--locked",
                "--workspace",
                "--exclude",
                "gpui-agent-desktop",
                "--all-targets",
                "--release",
            ],
        ),
        ("release", vec![], vec!["harness", "audit"]),
    ] {
        let sandbox = support::Sandbox::new();
        let root = sandbox.directory.path().join("checkout with spaces 한글");
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::copy(support::root().join("mise.toml"), root.join("mise.toml")).unwrap();
        for script in ["release.ps1", "release.sh", "gates.ps1", "gates.sh"] {
            fs::copy(support::root().join("scripts").join(script), root.join("scripts").join(script)).unwrap();
        }
        for fail in [false, true] {
            // Release must stop at the audit gate; never install packagers
            // here.
            if task == "release" && !fail {
                continue;
            }
            let log = sandbox.directory.path().join("calls.jsonl");
            if log.exists() {
                fs::remove_file(log).unwrap();
            }
            let mut command = Command::new("mise");
            sandbox.configure(&mut command);
            let output = command.current_dir(&root)
                .env("MISE_TRUSTED_CONFIG_PATHS", &root)
                .env("MISE_AUTO_INSTALL", "false")
                .env_remove("SKIP_GATES")
                // The release script probes audit tools before running audit.
                .env("HARNESS_TEST_FAIL", if fail { if task == "release" { "3" } else { "0" } } else { "" })
                .args(["run", task, "--"]).args(&args).output().unwrap();
            support::assert_exit(&output, if fail { 23 } else { 0 });
            let calls = sandbox.calls_in(&root);
            assert_eq!(calls.last().unwrap(), &expected, "task {task}");
            assert_eq!(calls.len(), if task == "release" { 4 } else { 1 }, "task {task} must stop at the failure");
        }
    }
}

#[test]
fn workspace_leaves_rust_toolchain_selection_to_rustup() {
    if Command::new("mise").arg("--version").output().is_err() {
        assert!(std::env::var_os("CI").is_none(), "CI must install mise to test toolchain selection");
        eprintln!("skip: install mise to test toolchain selection");
        return;
    }

    let sandbox = support::Sandbox::new();
    let checkout = sandbox.directory.path();
    fs::copy(support::root().join("mise.toml"), checkout.join("mise.toml")).unwrap();
    let user_config = checkout.join("user.toml");
    fs::write(&user_config, "[tools]\nrust = \"1.102.0-nightly\"\n").unwrap();

    let output = Command::new("mise")
        .current_dir(checkout)
        .env("MISE_TRUSTED_CONFIG_PATHS", checkout)
        .env("MISE_GLOBAL_CONFIG_FILE", user_config)
        .env_remove("MISE_DISABLE_TOOLS")
        .args(["ls", "--current", "rust", "--json"])
        .output()
        .unwrap();
    support::assert_exit(&output, 0);
    let managed_versions: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(managed_versions.is_empty(), "mise selected a Rust toolchain: {managed_versions:?}");
}
