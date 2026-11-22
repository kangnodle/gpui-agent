mod support;
use std::{fs,
          path::{Path,
                 PathBuf},
          process::Command};

fn strings(args: &[&str]) -> Vec<String> { args.iter().map(|s| s.to_string()).collect() }

fn shell_path(path: &Path) -> String { path.display().to_string().trim_start_matches(r"\\?\").replace('\\', "/") }

fn shells(script: &str, root: &Path) -> Vec<Command> {
    let powershell = script.ends_with(".ps1");
    let names: &[&str] = if powershell {
        &["powershell", "pwsh"]
    } else if cfg!(target_os = "macos") {
        &["bash", "/bin/bash"]
    } else {
        &["bash"]
    };
    names
        .iter()
        .filter_map(|name| {
            let available = Command::new(name).arg(if powershell { "-Version" } else { "--version" }).output().is_ok();
            if !available {
                eprintln!("skip {script}: {name} is unavailable");
                return None;
            }
            let mut command = Command::new(name);
            if powershell {
                command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
            }
            command.arg(shell_path(&root.join("scripts").join(script)));
            Some(command)
        })
        .collect()
}

fn release_commands(formats: &str, root: &Path) -> Vec<Vec<String>> {
    let output = shell_path(&root.join("target/release/bundle"));
    let mut commands = vec![
        strings(&["fmt", "--all"]),
        strings(&["clippy", "--workspace", "--all-targets", "--release", "--locked", "--", "-D", "warnings"]),
        strings(&["build", "--workspace", "--all-targets", "--release", "--locked"]),
        strings(&["test", "--workspace", "--release", "--locked"]),
        strings(&["install", "cargo-packager", "--version", "0.11.8", "--locked"]),
        strings(&[
            "packager",
            "--release",
            "--packages",
            "gpui-agent-desktop",
            "--formats",
            formats,
            "--out-dir",
            &output,
        ]),
    ];
    if formats == "deb" {
        commands.extend([
            strings(&["install", "cargo-generate-rpm", "--version", "0.21.1", "--locked"]),
            strings(&["generate-rpm", "--package", "apps/desktop", "--output", &output]),
        ]);
    }
    commands
}

fn write_shell(path: PathBuf, contents: &str) {
    fs::write(&path, contents).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn check_pipeline(script: &str, commands: impl Fn(&Path) -> Vec<Vec<String>>, arguments: &[&str], host: Option<&str>, success: i32, audit: bool) {
    let sandbox = support::Sandbox::new();
    let root = sandbox.directory.path().join("checkout with spaces 한글");
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::create_dir_all(root.join("packaging")).unwrap();
    // PowerShell expands Windows 8.3 aliases in temporary directory paths.
    let root = root.canonicalize().unwrap();
    let config = fs::read_to_string(support::root().join("mise.toml")).unwrap();
    fs::write(root.join("mise.toml"), format!("{config}\n[env]\nHARNESS_TEST_MISE_ENV = 'loaded'\n")).unwrap();
    for name in ["dev.ps1", "dev.sh", "gates.ps1", "gates.sh", "run.ps1", "run.sh", "release.ps1", "release.sh"] {
        fs::copy(support::root().join("scripts").join(name), root.join("scripts").join(name)).unwrap();
    }
    fs::copy(support::root().join("packaging/deb-depends.txt"), root.join("packaging/deb-depends.txt")).unwrap();
    if let Some(host) = host {
        write_shell(
            sandbox.directory.path().join("uname"),
            &format!("#!/usr/bin/env bash\nprintf '%s\\n' '{host}'\n"),
        );
        write_shell(sandbox.directory.path().join("getconf"), "#!/usr/bin/env bash\nprintf 'glibc 2.35\\n'\n");
    }
    let expected = commands(&root);
    for mut shell in shells(script, &root) {
        sandbox.configure(&mut shell);
        shell
            .env("SKIP_GATES", if audit { "" } else { "1" })
            .env("MISE_TRUSTED_CONFIG_PATHS", &root)
            .env("MISE_AUTO_INSTALL", "false")
            .env_remove("HARNESS_TEST_MISE_ENV")
            .args(arguments);
        for failure in std::iter::once(None).chain((0 .. expected.len()).map(Some)) {
            let log = sandbox.directory.path().join("calls.jsonl");
            if log.exists() {
                fs::remove_file(&log).unwrap();
            }
            shell.env("HARNESS_TEST_FAIL", failure.map(|index| index.to_string()).unwrap_or_default());
            let result = shell.output().unwrap();
            let missing_tool = audit && failure.is_some_and(|index| index < 3);
            support::assert_exit(
                &result,
                if missing_tool {
                    1
                } else if failure.is_some() {
                    23
                } else {
                    success
                },
            );
            let mut calls = sandbox.calls_in(&root);
            for call in &mut calls {
                for option in ["--out-dir", "--output", "--home"] {
                    if let Some(index) = call.iter().position(|arg| arg == option) {
                        call[index + 1] = shell_path(Path::new(&call[index + 1]));
                    }
                }
            }
            assert_eq!(
                calls,
                expected[.. if missing_tool { 3 } else { failure.map_or(expected.len(), |i| i + 1) }],
                "{script}, host {host:?}, failure {failure:?}"
            );
            for line in fs::read_to_string(&log).unwrap_or_default().lines() {
                let call: serde_json::Value = serde_json::from_str(line).unwrap();
                assert_eq!(
                    call["mise_environment"], "loaded",
                    "{script} must load the checkout's mise environment for every Cargo command"
                );
            }
            if host == Some("Linux") && failure.is_none() {
                assert_eq!(
                    fs::read_to_string(root.join("target/release/deb-depends.txt")).unwrap().lines().next(),
                    Some("libc6 (>= 2.35)")
                );
            }
        }
    }
}

#[test]
fn given_shell_pipelines_when_each_step_fails_then_exit_and_arguments_are_preserved() {
    for script in ["dev.ps1", "dev.sh"] {
        check_pipeline(script, |_| vec![strings(&["harness", "dev", "--surface", "desktop"])], &[], None, 0, false);
    }
    for arguments in [
        &["--cwd", "workspace with spaces/한글", "--home", "profile with spaces"][..],
        &["--home=inline-profile"][..],
    ] {
        for script in ["run.ps1", "run.sh"] {
            check_pipeline(
                script,
                |_| {
                    vec![
                        strings(&["fmt", "--all"]),
                        strings(&["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]),
                        strings(&["build", "--workspace", "--all-targets", "--locked"]),
                        strings(&["test", "--workspace", "--locked"]),
                        [strings(&["harness", "dev", "--surface", "desktop", "--"]), strings(arguments)].concat(),
                    ]
                },
                arguments,
                None,
                0,
                false,
            );
        }
    }
    let native = if cfg!(windows) {
        "nsis"
    } else if cfg!(target_os = "macos") {
        "app,dmg"
    } else {
        "deb"
    };
    check_pipeline("release.ps1", |root| release_commands(native, root), &[], None, 0, false);
    for (host, formats) in [("Linux", "deb"), ("Darwin", "app,dmg"), ("MINGW64_NT-10.0", "nsis")] {
        check_pipeline("release.sh", |root| release_commands(formats, root), &[], Some(host), 0, false);
    }
    check_pipeline("release.sh", |_| vec![], &[], Some("FreeBSD"), 1, false);
}

#[test]
fn given_legacy_run_scripts_without_a_home_when_started_then_they_use_the_development_harness() {
    for script in ["run.ps1", "run.sh"] {
        check_pipeline(
            script,
            |_| {
                vec![
                    strings(&["fmt", "--all"]),
                    strings(&["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]),
                    strings(&["build", "--workspace", "--all-targets", "--locked"]),
                    strings(&["test", "--workspace", "--locked"]),
                    strings(&["harness", "dev", "--surface", "desktop", "--", "--cwd", "workspace with spaces/한글"]),
                ]
            },
            &["--cwd", "workspace with spaces/한글"],
            None,
            0,
            false,
        );
    }
}

#[test]
fn given_audit_gates_when_tools_or_audit_fail_then_the_app_does_not_start() {
    for script in ["dev.ps1", "dev.sh"] {
        check_pipeline(
            script,
            |_| {
                vec![
                    strings(&["deny", "--version"]),
                    strings(&["machete", "--version"]),
                    strings(&["geiger", "--version"]),
                    strings(&["harness", "audit"]),
                    strings(&["harness", "dev", "--surface", "desktop"]),
                ]
            },
            &[],
            None,
            0,
            true,
        );
    }
}
