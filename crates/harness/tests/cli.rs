mod support;

impl support::Sandbox {
    fn harness(&self, args: &[&str]) -> std::process::Command {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_gpui-agent-harness"));
        self.configure(&mut command);
        command.args(args);
        command
    }

    fn calls(&self) -> Vec<Vec<String>> { self.calls_in(&support::root()) }
}

#[test]
fn given_explicit_scope_when_linted_then_all_requested_packages_are_preserved() {
    for (args, expected) in [
        (
            vec!["lint", "--package", "gpui-agent-domain", "--package", "gpui-agent-contracts"],
            vec![
                "clippy",
                "--locked",
                "--package",
                "gpui-agent-domain",
                "--package",
                "gpui-agent-contracts",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        (
            vec!["lint", "--include-desktop"],
            vec!["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
        ),
        (vec!["fmt"], vec!["fmt", "--all", "--", "--check"]),
        (
            vec!["client-fixture"],
            vec!["test", "--locked", "--package", "gpui-agent-api", "--test", "jobs", "--", "--client-fixture"],
        ),
    ] {
        let sandbox = support::Sandbox::new();
        support::assert_exit(&sandbox.harness(&args).output().unwrap(), 0);
        assert_eq!(sandbox.calls(), [expected]);
    }
    let sandbox = support::Sandbox::new();
    support::assert_exit(
        &sandbox
            .harness(&["lint", "--package", "gpui-agent-domain", "--include-desktop"])
            .output()
            .unwrap(),
        2,
    );
    assert!(sandbox.calls().is_empty());
}

#[test]
fn given_all_gates_succeed_when_check_runs_then_web_tests_also_run() {
    let sandbox = support::Sandbox::new();
    let node = sandbox.directory.path().join(if cfg!(windows) { "node.exe" } else { "node" });
    std::fs::copy(env!("CARGO_BIN_EXE_harness-cargo-fixture"), node).unwrap();
    support::assert_exit(&sandbox.harness(&["check", "--package", "gpui-agent-domain"]).output().unwrap(), 0);
    assert_eq!(
        sandbox.calls(),
        [
            vec!["fmt", "--all", "--", "--check"],
            vec!["clippy", "--locked", "--package", "gpui-agent-domain", "--all-targets", "--", "-D", "warnings"],
            vec!["test", "--locked", "--package", "gpui-agent-domain"],
            vec!["--test", "apps/web/test.cjs"],
        ]
    );
}

#[cfg(windows)]
#[test]
fn given_windows_desktop_when_check_runs_then_native_accessibility_regressions_are_checked() {
    for args in [vec!["check", "--include-desktop"], vec!["check", "--package", "gpui-agent-desktop"]] {
        let sandbox = support::Sandbox::new();
        std::fs::copy(env!("CARGO_BIN_EXE_harness-cargo-fixture"), sandbox.directory.path().join("node.exe")).unwrap();
        support::assert_exit(&sandbox.harness(&args).output().unwrap(), 0);
        assert!(sandbox.calls().contains(&vec![
            "test".into(),
            "--locked".into(),
            "--package".into(),
            "accesskit_windows".into(),
            "--lib".into()
        ]));
    }
}

#[test]
fn given_a_cargo_symlink_when_setup_runs_then_dispatch_name_is_preserved() {
    let sandbox = support::Sandbox::new();
    let cargo = sandbox.directory.path().join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
    let real = cargo.with_file_name(if cfg!(windows) { "cargo-real.exe" } else { "cargo-real" });
    std::fs::rename(&cargo, &real).unwrap();
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_file(&real, &cargo);
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&real, &cargo);
    if let Err(error) = result {
        eprintln!("skip: command symlinks unavailable: {error}");
        return;
    }
    support::assert_exit(&sandbox.harness(&["setup"]).output().unwrap(), 0);
    assert_eq!(sandbox.calls(), [vec!["fetch", "--locked"]]);
}

#[test]
fn given_a_custom_runner_when_filtered_then_it_is_rejected_before_cargo_runs() {
    for args in [
        vec!["--package", "gpui-agent", "--test", "protocols"],
        vec!["--package", "gpui-agent-api", "--test", "jobs"],
        vec!["--package", "gpui-agent-api"],
        vec![],
    ] {
        let sandbox = support::Sandbox::new();
        let mut arguments = vec!["test"];
        arguments.extend(args);
        arguments.extend(["--filter", "missing"]);
        let output = sandbox.harness(&arguments).output().unwrap();
        support::assert_exit(&output, 2);
        assert!(String::from_utf8_lossy(&output.stderr).contains("custom runner"));
        assert!(sandbox.calls().is_empty());
    }
}

#[test]
fn given_check_when_a_gate_fails_then_later_commands_do_not_run() {
    let expected = [
        vec!["fmt", "--all", "--", "--check"],
        vec![
            "clippy",
            "--locked",
            "--workspace",
            "--exclude",
            "gpui-agent-desktop",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        vec!["test", "--locked", "--workspace", "--exclude", "gpui-agent-desktop"],
    ];
    for failure in 0 .. 3 {
        let sandbox = support::Sandbox::new();
        let output = sandbox.harness(&["check"]).env("HARNESS_TEST_FAIL", failure.to_string()).output().unwrap();
        support::assert_exit(&output, 23);
        assert_eq!(sandbox.calls(), expected[..= failure]);
    }
}

#[test]
fn given_a_test_selection_when_run_then_package_target_and_filter_are_preserved() {
    let sandbox = support::Sandbox::new();
    support::assert_exit(
        &sandbox
            .harness(&[
                "test",
                "--package",
                "gpui-agent-application",
                "--test",
                "architecture",
                "--filter",
                "production_dependencies",
            ])
            .output()
            .unwrap(),
        0,
    );
    assert_eq!(
        sandbox.calls(),
        [vec![
            "test",
            "--locked",
            "--package",
            "gpui-agent-application",
            "--test",
            "architecture",
            "production_dependencies"
        ]]
    );
}

#[test]
fn given_a_locked_checkout_when_setup_runs_then_fetch_is_locked() {
    let sandbox = support::Sandbox::new();
    let output = sandbox.harness(&["setup"]).output().unwrap();
    support::assert_exit(&output, 0);
    assert_eq!(sandbox.calls(), [vec!["fetch", "--locked"]]);
}

#[test]
fn given_a_foreign_directory_when_dev_runs_then_home_is_private_and_stable_across_surfaces() {
    let mut shared_home = None;
    for surface in ["server", "desktop"] {
        for write in [false, true] {
            let sandbox = support::Sandbox::new();
            let mut args = vec!["dev", "--surface", surface];
            if write {
                args.push("--mcp-write");
            }
            support::assert_exit(&sandbox.harness(&args).output().unwrap(), 0);
            let root = support::root();
            let calls = sandbox.calls();
            let home = std::path::PathBuf::from(&calls[0][8]);
            assert!(home.is_absolute() && !home.starts_with(&root));
            if let Some(previous) = shared_home.replace(home.clone()) {
                assert_eq!(previous, home);
            }
            let mut expected = vec![
                "run".into(),
                "--locked".into(),
                "--package".into(),
                format!("gpui-agent-{surface}"),
                "--".into(),
                "--cwd".into(),
                root.display().to_string(),
                "--home".into(),
                home.display().to_string(),
            ];
            if surface == "server" {
                expected.extend(["--port".into(), "0".into()]);
            }
            if write {
                expected.push("--mcp-write".into());
            }
            assert_eq!(sandbox.calls(), [expected]);
        }
    }
}

#[test]
fn desktop_development_forwards_explicit_home_and_workspace() {
    let sandbox = support::Sandbox::new();
    let args = ["dev", "--surface", "desktop", "--", "--home=custom home", "--cwd", "project path"];
    support::assert_exit(&sandbox.harness(&args).output().unwrap(), 0);
    assert_eq!(
        sandbox.calls(),
        [vec![
            "run",
            "--locked",
            "--package",
            "gpui-agent-desktop",
            "--",
            "--home=custom home",
            "--cwd",
            "project path"
        ]]
    );
}

#[test]
fn given_supply_chain_commands_when_run_then_each_delegates_to_its_cargo_tool() {
    for (args, expected) in [
        (vec!["deny"], vec![vec!["deny", "check"]]),
        (vec!["unused"], vec![vec!["machete", "crates", "apps"]]),
        (vec!["audit"], vec![vec!["deny", "check"], vec!["machete", "crates", "apps"]]),
    ] {
        let sandbox = support::Sandbox::new();
        let output = sandbox.harness(&args).output().unwrap();
        // `audit` continues into the unsafe gate, which fails closed on the
        // fixture's empty report.
        support::assert_exit(&output, if args == ["audit"] { 1 } else { 0 });
        let calls = sandbox.calls();
        assert_eq!(calls[.. expected.len()], expected);
    }
}

#[test]
fn given_a_failing_deny_when_audit_runs_then_later_gates_do_not_run() {
    let sandbox = support::Sandbox::new();
    let output = sandbox.harness(&["audit"]).env("HARNESS_TEST_FAIL", "0").output().unwrap();
    support::assert_exit(&output, 23);
    assert_eq!(sandbox.calls(), [vec!["deny", "check"]]);
}

#[test]
fn given_no_geiger_report_when_unsafe_runs_then_the_budget_fails_closed() {
    let sandbox = support::Sandbox::new();
    let output = sandbox.harness(&["unsafe"]).output().unwrap();
    support::assert_exit(&output, 1);
    let calls = sandbox.calls();
    assert!(calls.len() > 1);
    assert!(calls.iter().all(|call| call[0] == "geiger" && call.contains(&"--forbid-only".to_string())));
    assert!(String::from_utf8_lossy(&output.stderr).contains("did not produce JSON"));
}

#[test]
fn given_the_removed_bazel_command_when_run_then_it_is_rejected_without_starting_tools() {
    let sandbox = support::Sandbox::new();
    let output = sandbox.harness(&["bazel"]).output().unwrap();
    support::assert_exit(&output, 2);
    assert!(sandbox.calls().is_empty());
}
