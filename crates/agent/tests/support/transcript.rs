#[path = "redaction.rs"]
mod redaction;
use anyhow::{Context,
             Result,
             ensure};
use serde_json::{Value,
                 json};
use std::{fs,
          io::{self,
               BufRead,
               BufReader,
               Read,
               Write},
          path::{Path,
                 PathBuf},
          process::{Command,
                    Stdio},
          sync::{Arc,
                 Mutex}};

const LIMIT: u64 = 8 * 1024 * 1024;

fn frames(mut reader: impl BufRead, mut emit: impl FnMut(Value) -> Result<()>) -> Result<()> {
    loop {
        let mut line = Vec::new();
        if (&mut reader).take(LIMIT + 1).read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        ensure!(line.len() as u64 <= LIMIT, "Capture frame exceeds limit");
        let value: Value = serde_json::from_slice(&line).map_err(|_| anyhow::anyhow!("Capture requires JSON protocol frames"))?;
        emit(value)?;
    }
}

pub fn proxy() -> Result<bool> {
    if std::env::var_os("GPUI_CAPTURE_CHILD").is_some() {
        return Ok(false);
    }
    let path = Path::new("capture-proxy.json");
    if !path.exists() {
        return Ok(false);
    }
    let spec: Value = serde_json::from_reader(fs::File::open(path)?)?;
    if spec["mode"] == "replay" {
        replay(Path::new(spec["input"].as_str().context("Replay input")?))?;
    } else {
        ensure!(spec["mode"] == "record", "Unknown capture mode");
        record(&spec)?;
    }
    Ok(true)
}

fn record(spec: &Value) -> Result<()> {
    let output = Path::new(spec["output"].as_str().context("Capture output")?);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(output)?;
    writeln!(
        file,
        "{}",
        json!({"schema":1,"provider":spec["provider"],"source":spec["source"],"runtime_version":spec["runtime_version"]})
    )?;
    file.flush()?;
    let written = file.metadata()?.len();
    let log = Arc::new(Mutex::new((file, written)));
    let mut child = Command::new(spec["executable"].as_str().context("Native executable")?)
        .args(std::env::args_os().skip(1))
        .env("GPUI_CAPTURE_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().context("Native stdin")?;
    let stdout = child.stdout.take().context("Native stdout")?;
    let input_log = log.clone();
    std::thread::spawn(move || -> Result<()> {
        frames(io::stdin().lock(), |message| {
            append(&input_log, "client", &message)?;
            writeln!(input, "{message}")?;
            input.flush()?;
            Ok(())
        })
    });
    let output_thread = std::thread::spawn(move || -> Result<()> {
        let mut output = io::stdout().lock();
        frames(BufReader::new(stdout), |message| {
            append(&log, "provider", &message)?;
            writeln!(output, "{message}")?;
            output.flush()?;
            Ok(())
        })
    });
    let status = child.wait()?;
    output_thread.join().map_err(|_| anyhow::anyhow!("Capture reader stopped"))??;
    ensure!(status.success(), "Native capture process failed");
    Ok(())
}

fn append(log: &Mutex<(fs::File, u64)>, direction: &str, message: &Value) -> Result<()> {
    let mut log = log.lock().map_err(|_| anyhow::anyhow!("Capture writer stopped"))?;
    let line = json!({"direction":direction,"message":redaction::redact(message, "")}).to_string();
    let length = line.len() as u64 + 1;
    ensure!(log.1 + length <= LIMIT, "Capture exceeds limit");
    writeln!(log.0, "{line}")?;
    log.0.flush()?;
    log.1 += length;
    Ok(())
}

fn replay(path: &Path) -> Result<()> {
    ensure!(fs::metadata(path)?.len() <= LIMIT, "Replay exceeds limit");
    let mut records = BufReader::new(fs::File::open(path)?).lines();
    let header: Value = serde_json::from_str(&records.next().context("Missing capture metadata")??)?;
    ensure!(header["schema"] == 1, "Unsupported capture schema");
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout().lock();
    for record in records {
        let record: Value = serde_json::from_str(&record?)?;
        let expected = &record["message"];
        if record["direction"] == "client" {
            let mut line = String::new();
            ensure!(
                (&mut input).take(LIMIT + 1).read_line(&mut line)? > 0 && line.len() as u64 <= LIMIT,
                "Replay request missing or too large"
            );
            let actual: Value = serde_json::from_str(&line)?;
            ensure!(redaction::redact(&actual, "") == *expected, "Native request differs from recording");
        } else {
            ensure!(record["direction"] == "provider", "Invalid capture direction");
            writeln!(output, "{expected}")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub async fn record_live() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(2).collect();
    ensure!(args.len() == 2, "Usage: --record-live <absolute-codex-executable> <new-output.jsonl>");
    let executable = fs::canonicalize(&args[0])?;
    let output = std::path::absolute(PathBuf::from(&args[1]))?;
    let version = Command::new(&executable).arg("--version").output()?;
    ensure!(version.status.success(), "Cannot inspect runtime version");
    let version = String::from_utf8(version.stdout)?;
    ensure!(version.len() < 128 && version.trim().starts_with("codex-cli "), "Expected Codex runtime");
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join("capture-proxy.json"),
        json!({"mode":"record","provider":"codex","source":"live","runtime_version":version.trim(),"executable":executable,"output":output}).to_string(),
    )?;
    let status = gpui_agent::Agent::new(super::config(root.path(), gpui_agent::Provider::Codex))?
        .status()
        .await?;
    ensure!(!status.authenticated, "Live capture must use an empty isolated profile");
    println!("Recorded isolated Codex initialize/account-read traffic; no login or model invocation");
    Ok(())
}

pub async fn replay_committed() -> Result<()> {
    let capture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/recordings/codex-status.jsonl");
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("capture-proxy.json"), json!({"mode":"replay","input":capture}).to_string())?;
    ensure!(
        !gpui_agent::Agent::new(super::config(root.path(), gpui_agent::Provider::Codex))?
            .status()
            .await?
            .authenticated
    );
    Ok(())
}
