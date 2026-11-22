#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]

use anyhow::{Context,
             Result};
use clap::{CommandFactory,
           Parser};
use std::{env,
          path::{Path,
                 PathBuf},
          process::Command};
mod plan;
use plan::{Action,
           Cli};

fn executable(name: &str) -> Result<PathBuf> {
    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        for extension in if cfg!(windows) { &["exe", "cmd", "bat"][..] } else { &[""][..] } {
            let path = directory.join(name).with_extension(extension);
            if path.is_file() {
                return Ok(std::path::absolute(path)?);
            }
        }
    }
    anyhow::bail!("{name} is missing; install Rust with rustup, run mise install for project tools, or mise run audit:install for Cargo audit tools")
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()?;
    if matches!(
        cli.command,
        Action::Test {
            filter: Some(_),
            ..
        }
    ) {
        let workspace = manifest(&root.join("Cargo.toml"))?;
        let members = workspace["workspace"]["members"].as_array().context("workspace members must be an array")?;
        let manifests = members
            .iter()
            .map(|member| {
                let member = member.as_str().context("workspace member must be a path")?;
                manifest(&root.join(member).join("Cargo.toml"))
            })
            .collect::<Result<Vec<_>>>()?;
        if let Some(error) = plan::unsupported_filter(&cli.command, &manifests) {
            Cli::command().error(clap::error::ErrorKind::ValueValidation, error).exit();
        }
    }
    let development_home = if matches!(cli.command, Action::Doctor | Action::Dev { .. }) {
        use sha2::{Digest,
                   Sha256};
        let key = Sha256::digest(root.as_os_str().as_encoded_bytes());
        let directory = dirs::data_local_dir()
            .context("Cannot locate the user data directory")?
            .join("gpui-agent-dev")
            .join(format!("{key:x}"));
        let ancestor = directory.ancestors().find(|path| path.exists()).context("Missing development home parent")?;
        anyhow::ensure!(
            !ancestor.canonicalize()?.starts_with(&root),
            "The development home must be outside the checkout"
        );
        directory
    } else {
        PathBuf::new()
    };
    if matches!(cli.command, Action::Doctor) {
        println!("Checkout: {}\nIsolated home: {}", root.display(), development_home.display());
    }
    for command in plan::commands(&cli.command, &root, &development_home) {
        if let Some(code) = execute(&command, &root)? {
            return Ok(code);
        }
    }
    if matches!(cli.command, Action::Unsafe | Action::Audit) {
        return unsafe_gate(&root);
    }
    Ok(0)
}

/// `cargo run` leaks the harness's own `CARGO_PKG_*` into children; cargo
/// subcommands such as cargo-machete read them to decide how they were invoked.
fn clean(command: &mut Command) -> &mut Command {
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("CARGO_PKG_") {
            command.env_remove(key);
        }
    }
    command
}

fn unsafe_gate(root: &Path) -> Result<i32> {
    let workspace = manifest(&root.join("Cargo.toml"))?;
    let packages = plan::unsafe_free(&workspace);
    anyhow::ensure!(!packages.is_empty(), "Cargo.toml has no [workspace.metadata.harness] unsafe-free list");
    let members = workspace["workspace"]["members"].as_array().context("workspace members must be an array")?;
    let mut findings = Vec::new();
    for package in &packages {
        let path = members
            .iter()
            .filter_map(toml::Value::as_str)
            .map(|member| root.join(member).join("Cargo.toml"))
            .find(|path| {
                manifest(path)
                    .ok()
                    .and_then(|m| m.get("package")?.get("name")?.as_str().map(|name| name == package))
                    .unwrap_or(false)
            })
            .with_context(|| format!("unsafe-free package {package} is not a workspace member"))?;
        let command = plan::geiger_command(&std::path::absolute(&path)?);
        let program = executable(&command[0].to_string_lossy())?;
        println!("+ {} {:?}", program.display(), &command[1 ..]);
        let output = clean(Command::new(&program).args(&command[1 ..]).current_dir(root)).output()?;
        if !output.status.success() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
            return Ok(output.status.code().unwrap_or(1));
        }
        findings.extend(plan::unsafe_findings(package, &String::from_utf8_lossy(&output.stdout)));
    }
    for finding in &findings {
        eprintln!("unsafe budget: {finding}");
    }
    Ok(i32::from(!findings.is_empty()))
}

fn execute(command: &[std::ffi::OsString], root: &Path) -> Result<Option<i32>> {
    let program = executable(&command[0].to_string_lossy())?;
    println!("+ {} {:?}", program.display(), &command[1 ..]);
    let status = clean(Command::new(&program).args(&command[1 ..]).current_dir(root))
        .status()
        .with_context(|| format!("could not run {}", program.display()))?;
    Ok((!status.success()).then(|| status.code().unwrap_or(1)))
}

fn manifest(path: &Path) -> Result<toml::Value> {
    let source = std::fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("could not parse {}", path.display()))
}

fn main() {
    let code = match run() {
        | Ok(code) => code,
        | Err(error) => {
            eprintln!("harness: {error:#}");
            1
        },
    };
    std::process::exit(code);
}
