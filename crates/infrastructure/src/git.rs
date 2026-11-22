//! Native Git only: argument arrays, literal pathspecs, no hooks or external
//! diff drivers.
use crate::projects::{failure,
                      invalid};
use gpui_agent_application::{ApplicationError,
                             ApplicationResult,
                             ErrorKind};
use gpui_agent_domain::ChangedFile;
use std::{io::Read,
          path::Path,
          process::{Command,
                    Stdio},
          sync::mpsc,
          time::{Duration,
                 Instant}};
const LIMIT: usize = 2 * 1024 * 1024;
pub(crate) struct Output {
    pub code: i32,
    pub bytes: Vec<u8>,
    pub error: String,
    pub truncated: bool,
}
impl Output {
    pub fn text(&self) -> String { String::from_utf8_lossy(&self.bytes).into_owned() }

    pub fn require(self) -> ApplicationResult<Self> {
        if self.code != 0 {
            return Err(failure(format!("Git: {}", self.error.trim())));
        }
        Ok(self)
    }
}
pub(crate) fn run(cwd: &Path, hooks: &Path, args: &[&str]) -> ApplicationResult<Output> { run_env(cwd, hooks, args, &[]) }
pub(crate) fn run_env(cwd: &Path, hooks: &Path, args: &[&str], env: &[(&str, &std::ffi::OsStr)]) -> ApplicationResult<Output> {
    let mut command = Command::new("git");
    command
        .args(["--no-pager", "--literal-pathspecs", "-c", "core.fsmonitor=false", "-c", "color.ui=false", "-c"])
        .arg(format!("core.hooksPath={}", hooks.display()))
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The caller selects a repository; inherited Git routing must not override
    // it.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().to_ascii_uppercase().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
    for (key, value) in env {
        command.env(key, value);
    }
    execute(command)
}
pub(crate) fn execute(mut command: Command) -> ApplicationResult<Output> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| ApplicationError(ErrorKind::Unavailable, format!("Cannot start host tool: {error}")))?;
    let stdout = match child.stdout.take() {
        | Some(pipe) => pipe,
        | None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure("Host tool stdout pipe is unavailable"));
        },
    };
    let stderr = match child.stderr.take() {
        | Some(pipe) => pipe,
        | None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure("Host tool stderr pipe is unavailable"));
        },
    };
    let (tx, rx) = mpsc::channel();
    let err_tx = tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send((false, read_bounded(stdout, LIMIT)));
    });
    std::thread::spawn(move || {
        let _ = err_tx.send((true, read_bounded(stderr, 65536)));
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| failure(error.to_string()))? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure("Git operation timed out"));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut output = Output {
        code: status.code().unwrap_or(-1),
        bytes: Vec::new(),
        error: String::new(),
        truncated: false,
    };
    for _ in 0 .. 2 {
        let (stderr, result) = rx.recv_timeout(Duration::from_secs(1)).map_err(|_| failure("Git output pipe did not close"))?;
        let (bytes, truncated) = result.map_err(|error| failure(error.to_string()))?;
        if stderr {
            output.error = String::from_utf8_lossy(&bytes).into_owned();
        } else {
            output.bytes = bytes;
            output.truncated = truncated;
        }
    }
    Ok(output)
}
fn read_bounded(mut input: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut chunk = [0; 8192];
    loop {
        let n = input.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[.. keep]);
        truncated |= keep < n;
    }
    Ok((bytes, truncated))
}
pub(crate) fn repository(path: &Path, hooks: &Path) -> bool {
    run(path, hooks, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out.code == 0 && out.text().trim() == "true")
}
pub(crate) fn branch(path: &Path, hooks: &Path) -> Option<String> {
    run(path, hooks, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|out| out.code == 0)
        .map(|out| out.text().trim().to_owned())
}
pub(crate) fn changes(path: &Path, hooks: &Path, ignored: bool) -> ApplicationResult<Vec<ChangedFile>> {
    let mut args = vec!["status", "--porcelain=v1", "-z", "--untracked-files=all"];
    if ignored {
        args.push("--ignored=matching");
    }
    let output = run(path, hooks, &args)?.require()?;
    if output.truncated {
        return Err(failure("Too many changed files to list safely"));
    }
    parse_status(&output.bytes)
}
fn parse_status(bytes: &[u8]) -> ApplicationResult<Vec<ChangedFile>> {
    let mut fields = bytes.split(|byte| *byte == 0).filter(|field| !field.is_empty());
    let mut result = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 || field[2] != b' ' {
            return Err(failure("Invalid Git status record"));
        }
        let path = std::str::from_utf8(&field[3 ..])
            .map_err(|_| invalid("Git paths must be valid UTF-8"))?
            .to_owned();
        let renamed = matches!(field[0], b'R' | b'C') || matches!(field[1], b'R' | b'C');
        let original_path = if renamed {
            Some(
                std::str::from_utf8(fields.next().ok_or_else(|| failure("Missing rename source"))?)
                    .map_err(|_| invalid("Git paths must be valid UTF-8"))?
                    .to_owned(),
            )
        } else {
            None
        };
        result.push(ChangedFile {
            path,
            original_path,
            index_status: (field[0] as char).to_string(),
            worktree_status: (field[1] as char).to_string(),
        });
    }
    Ok(result)
}
pub(crate) struct Tree {
    pub path: String,
    pub branch: Option<String>,
    pub main: bool,
    pub locked: bool,
}
pub(crate) fn trees(path: &Path, hooks: &Path) -> ApplicationResult<Vec<Tree>> {
    let out = run(path, hooks, &["worktree", "list", "--porcelain", "-z"])?.require()?;
    if out.truncated {
        return Err(failure("Too many Git worktrees"));
    }
    let mut trees = Vec::new();
    for field in out.bytes.split(|byte| *byte == 0).filter(|part| !part.is_empty()) {
        let field = std::str::from_utf8(field).map_err(|_| invalid("Git worktree paths must be UTF-8"))?;
        if let Some(path) = field.strip_prefix("worktree ") {
            trees.push(Tree {
                path: path.into(),
                branch: None,
                main: trees.is_empty(),
                locked: false,
            });
        } else if let Some(tree) = trees.last_mut() {
            if let Some(branch) = field.strip_prefix("branch refs/heads/") {
                tree.branch = Some(branch.into());
            }
            if field == "locked" || field.starts_with("locked ") {
                tree.locked = true;
            }
        }
    }
    Ok(trees)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_output_pipes_return_errors_and_reap_the_child() {
        for missing_stdout in [true, false] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.arg("--list").stdin(Stdio::null());
            command.stdout(if missing_stdout { Stdio::null() } else { Stdio::piped() });
            command.stderr(if missing_stdout { Stdio::piped() } else { Stdio::null() });
            let error = match execute(command) {
                | Ok(_) => panic!("missing pipe was accepted"),
                | Err(error) => error,
            };
            assert!(error.1.contains(if missing_stdout {
                "stdout pipe is unavailable"
            } else {
                "stderr pipe is unavailable"
            }));
        }
    }
}
