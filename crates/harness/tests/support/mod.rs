use serde_json::Value;
use std::{env,
          fs,
          path::{Path,
                 PathBuf},
          process::{Command,
                    Output}};

pub fn root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }

pub struct Sandbox {
    pub directory: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let directory = tempfile::Builder::new().prefix("gpui harness 한글 ").tempdir().unwrap();
        let binary = directory.path().join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
        fs::copy(env!("CARGO_BIN_EXE_harness-cargo-fixture"), &binary).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            directory,
        }
    }

    pub fn configure(&self, command: &mut Command) {
        let paths = std::iter::once(self.directory.path().to_owned())
            .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default()))
            .collect::<Vec<_>>();
        command
            .current_dir(self.directory.path())
            .env("PATH", env::join_paths(paths).unwrap())
            .env("HARNESS_TEST_LOG", self.directory.path().join("calls.jsonl"));
    }

    pub fn calls_in(&self, expected_root: &Path) -> Vec<Vec<String>> {
        let log = fs::read_to_string(self.directory.path().join("calls.jsonl")).unwrap_or_default();
        log.lines()
            .map(|line| {
                let call: Value = serde_json::from_str(line).unwrap();
                assert_eq!(
                    Path::new(call["cwd"].as_str().unwrap()).canonicalize().unwrap(),
                    expected_root.canonicalize().unwrap()
                );
                call["args"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect()
            })
            .collect()
    }
}

/// Whether this host lets the current user create file symlinks. Unix always
/// does; Windows needs Developer Mode or elevation.
#[allow(dead_code)]
pub fn file_symlinks_allowed() -> bool {
    #[cfg(windows)]
    {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        fs::write(&target, "").unwrap();
        std::os::windows::fs::symlink_file(&target, directory.path().join("link")).is_ok()
    }
    #[cfg(not(windows))]
    true
}

pub fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
