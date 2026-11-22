use gpui_agent_application::ports::{ProjectVerifier,
                                    RuntimeFuture};
use gpui_agent_domain::{CodeVersion,
                        JobScope,
                        VerificationCheck,
                        VerificationStatus};
use sha2::{Digest,
           Sha256};
use std::{io::Read,
          path::{Path,
                 PathBuf},
          process::Stdio};
use tokio::io::AsyncReadExt;

pub(crate) struct Verifier {
    pub home: PathBuf,
}
impl ProjectVerifier for Verifier {
    fn check<'a>(&'a self, scope: &'a JobScope, command: &'a [String], timeout_seconds: u32) -> RuntimeFuture<'a, VerificationCheck> {
        Box::pin(async move { Ok(execute(scope, command, timeout_seconds).await) })
    }

    fn revision<'a>(&'a self, scope: &'a JobScope) -> RuntimeFuture<'a, CodeVersion> {
        let path = PathBuf::from(&scope.workspace_path);
        let home = self.home.clone();
        Box::pin(async move { tokio::task::spawn_blocking(move || revision(&path, &home)).await? })
    }
}

async fn execute(scope: &JobScope, argv: &[String], seconds: u32) -> VerificationCheck {
    let result = tokio::time::timeout(std::time::Duration::from_secs(u64::from(seconds)), async {
        let (program, args) = argv.split_first().ok_or_else(|| anyhow::anyhow!("Verification command is empty"))?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .current_dir(&scope.workspace_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = gpui_agent::process::spawn(&mut command)?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("Verifier stdout missing"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("Verifier stderr missing"))?;
        let bounded = async |reader: Box<dyn tokio::io::AsyncRead + Unpin + Send>| -> std::io::Result<Vec<u8>> {
            let mut bytes = vec![];
            let mut reader = reader.take(16384);
            reader.read_to_end(&mut bytes).await?;
            tokio::io::copy(&mut reader.into_inner(), &mut tokio::io::sink()).await?;
            Ok(bytes)
        };
        let (out, err, status) = tokio::try_join!(bounded(Box::new(stdout)), bounded(Box::new(stderr)), child.wait())?;
        Ok::<_, anyhow::Error>((status.code(), format!("{}\n{}", String::from_utf8_lossy(&out), String::from_utf8_lossy(&err))))
    })
    .await;
    let (exit_code, output) = match result {
        | Ok(Ok(result)) => result,
        | Ok(Err(error)) => (None, error.to_string()),
        | Err(_) => (None, format!("Host verification timed out after {seconds} seconds")),
    };
    VerificationCheck {
        command: argv.to_vec(),
        status: if exit_code == Some(0) {
            VerificationStatus::Passed
        } else {
            VerificationStatus::Failed
        },
        exit_code,
        output,
    }
}

fn revision(root: &Path, home: &Path) -> anyhow::Result<CodeVersion> {
    let root = std::fs::canonicalize(root)?;
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_owned());
    let hooks = home.join("empty-hooks");
    let is_git = crate::git::repository(&root, &hooks);
    let mut paths = vec![];
    let mut head = None;
    if is_git {
        let output = crate::git::run(&root, &hooks, &["rev-parse", "--verify", "HEAD"])?;
        if output.code == 0 {
            head = Some(output.text().trim().to_owned());
        }
        let output = crate::git::run(&root, &hooks, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"])?.require()?;
        anyhow::ensure!(!output.truncated, "Project file list is too large to fingerprint safely");
        for name in output.bytes.split(|b| *b == 0).filter(|name| !name.is_empty()) {
            let name = std::str::from_utf8(name)?;
            let relative = Path::new(name);
            anyhow::ensure!(
                !relative.is_absolute() && !relative.components().any(|part| matches!(part, std::path::Component::ParentDir)),
                "Invalid project file path"
            );
            paths.push(root.join(relative));
        }
    } else {
        collect(&root, &home, &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    let mut hash = Sha256::new();
    hash.update(head.as_deref().unwrap_or("unborn-or-plain-folder").as_bytes());
    let mut total = 0u64;
    for path in paths {
        if path.starts_with(&home) {
            continue;
        }
        let name = path
            .strip_prefix(&root)?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Project paths must be UTF-8"))?
            .replace('\\', "/");
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if !path.try_exists()? {
            hash.update(b"deleted");
            continue;
        }
        let canonical = std::fs::canonicalize(&path)?;
        anyhow::ensure!(canonical.starts_with(&root), "Cannot fingerprint project links outside the workspace");
        let meta = std::fs::metadata(&path)?;
        anyhow::ensure!(meta.is_file(), "Cannot fingerprint a non-file project entry");
        total = total.saturating_add(meta.len());
        anyhow::ensure!(
            total <= 1024 * 1024 * 1024,
            "Project sources exceed the 1 GiB fingerprint limit; exclude generated files with Git ignore rules"
        );
        hash.update(meta.len().to_le_bytes());
        let mut file = std::fs::File::open(&path)?;
        let mut buffer = [0u8; 32768];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[.. size]);
        }
    }
    Ok(CodeVersion {
        head,
        fingerprint: format!("{:x}", hash.finalize()),
    })
}
fn collect(root: &Path, home: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.starts_with(home) || entry.file_name() == ".git" {
            continue;
        }
        let kind = entry.file_type()?;
        anyhow::ensure!(!kind.is_symlink(), "Plain-folder verification cannot follow symbolic links");
        if kind.is_dir() {
            collect(&path, home, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
        anyhow::ensure!(files.len() <= 100_000, "Project contains too many files to fingerprint safely");
    }
    Ok(())
}
