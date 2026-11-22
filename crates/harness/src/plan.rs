use clap::{Args,
           Parser,
           Subcommand,
           ValueEnum};
use std::{ffi::OsString,
          path::Path};

#[derive(Parser)]
#[command(about = "Portable, non-destructive developer commands for this Cargo workspace")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Action,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Surface {
    Server,
    Desktop,
}

#[derive(Args)]
pub struct Scope {
    #[arg(long, conflicts_with = "include_desktop")]
    pub package: Vec<String>,
    #[arg(long)]
    pub include_desktop: bool,
}

impl Scope {
    fn arguments(&self) -> Vec<OsString> {
        if !self.package.is_empty() {
            self.package.iter().flat_map(|p| ["--package".into(), p.into()]).collect()
        } else if self.include_desktop {
            strings(&["--workspace"])
        } else {
            strings(&["--workspace", "--exclude", "gpui-agent-desktop"])
        }
    }
}

#[derive(Subcommand)]
pub enum Action {
    Doctor,
    Setup,
    Fmt,
    #[command(about = "Serve the real web client with isolated native protocol fixtures; Enter stops it")]
    ClientFixture,
    Lint(Scope),
    Test {
        #[command(flatten)]
        scope: Scope,
        #[arg(long, help = "Cargo integration-test target")]
        test: Option<String>,
        #[arg(long, help = "Rust test name filter")]
        filter: Option<String>,
    },
    Check(Scope),
    #[command(about = "Supply-chain policy: advisories, licenses, bans, sources (cargo-deny, deny.toml)")]
    Deny,
    #[command(about = "Unused dependencies in crates/ and apps/ (cargo-machete)")]
    Unused,
    #[command(about = "Unsafe budget: workspace.metadata.harness.unsafe-free crates must have no unsafe (cargo-geiger)")]
    Unsafe,
    #[command(about = "Deny, Unused and Unsafe together")]
    Audit,
    Dev {
        #[arg(long, value_enum, default_value = "server")]
        surface: Surface,
        #[arg(long, help = "Allow scoped conversation/job updates")]
        mcp_write: bool,
    },
}

fn strings(values: &[&str]) -> Vec<OsString> { values.iter().map(OsString::from).collect() }

pub fn unsupported_filter(action: &Action, manifests: &[toml::Value]) -> Option<String> {
    let Action::Test {
        scope,
        test,
        filter: Some(_),
        ..
    } = action
    else {
        return None;
    };
    for manifest in manifests {
        let package = manifest.get("package")?.get("name")?.as_str()?;
        if !scope.package.is_empty() && !scope.package.iter().any(|selected| selected.split('@').next() == Some(package)) {
            continue;
        }
        for target in manifest.get("test").and_then(toml::Value::as_array).into_iter().flatten() {
            let name = target.get("name")?.as_str()?;
            if target.get("harness").and_then(toml::Value::as_bool) == Some(false) && test.as_ref().is_none_or(|selected| selected == name) {
                return Some(format!(
                    "--filter is unsupported by custom runner {package}/{name}; select a libtest target with --test, or run this runner without --filter"
                ));
            }
        }
    }
    None
}

pub fn commands(action: &Action, root: &Path) -> Vec<Vec<OsString>> {
    match action {
        | Action::Doctor => vec![
            strings(&["cargo", "--version"]),
            strings(&["rustc", "--version"]),
            strings(&["rustup", "component", "list", "--installed"]),
        ],
        | Action::Setup => vec![strings(&["cargo", "fetch", "--locked"])],
        | Action::Fmt => vec![strings(&["cargo", "fmt", "--all", "--", "--check"])],
        | Action::Deny => vec![strings(&["cargo", "deny", "check"])],
        | Action::Unused => vec![strings(&["cargo", "machete", "crates", "apps"])],
        | Action::Audit => [Action::Deny, Action::Unused].iter().flat_map(|action| commands(action, root)).collect(),
        // Geiger output is parsed, not just exit-coded; see `unsafe_commands`.
        | Action::Unsafe => Vec::new(),
        | Action::ClientFixture => vec![strings(&[
            "cargo",
            "test",
            "--locked",
            "--package",
            "gpui-agent-api",
            "--test",
            "jobs",
            "--",
            "--client-fixture",
        ])],
        | Action::Dev {
            surface,
            mcp_write,
        } => {
            let package = match surface {
                | Surface::Server => "gpui-agent-server",
                | Surface::Desktop => "gpui-agent-desktop",
            };
            let mut command = vec![
                "cargo".into(),
                "run".into(),
                "--locked".into(),
                "--package".into(),
                package.into(),
                "--".into(),
                "--cwd".into(),
                root.into(),
                "--home".into(),
                root.join(".dev/home").into(),
            ];
            if matches!(surface, Surface::Server) {
                command.extend(["--port".into(), "0".into()]);
            }
            if *mcp_write {
                command.push("--mcp-write".into());
            }
            vec![command]
        },
        | Action::Lint(scope)
        | Action::Check(scope)
        | Action::Test {
            scope, ..
        } => {
            let mut commands = Vec::new();
            if matches!(action, Action::Check(_)) {
                commands.push(strings(&["cargo", "fmt", "--all", "--", "--check"]));
            }
            if matches!(action, Action::Lint(_) | Action::Check(_)) {
                let mut command = strings(&["cargo", "clippy", "--locked"]);
                command.extend(scope.arguments());
                command.extend(strings(&["--all-targets", "--", "-D", "warnings"]));
                commands.push(command);
            }
            if cfg!(windows)
                && matches!(action, Action::Check(_))
                && (scope.include_desktop || scope.package.iter().any(|package| package.split('@').next() == Some("gpui-agent-desktop")))
            {
                commands.push(strings(&["cargo", "test", "--locked", "--package", "accesskit_windows", "--lib"]));
            }
            if matches!(action, Action::Test { .. } | Action::Check(_)) {
                let mut command = strings(&["cargo", "test", "--locked"]);
                command.extend(scope.arguments());
                if let Action::Test {
                    test,
                    filter,
                    ..
                } = action
                {
                    if let Some(test) = test {
                        command.extend(["--test".into(), test.into()]);
                    }
                    if let Some(filter) = filter {
                        command.push(filter.into());
                    }
                }
                commands.push(command);
            }
            if matches!(action, Action::Check(_)) {
                commands.push(strings(&["node", "--test", "apps/web/test.cjs"]));
            }
            commands
        },
    }
}

/// Workspace packages that must contain no `unsafe`, from
/// `[workspace.metadata.harness] unsafe-free` in the root manifest.
pub fn unsafe_free(workspace: &toml::Value) -> Vec<String> {
    workspace
        .get("workspace")
        .and_then(|w| w.get("metadata"))
        .and_then(|m| m.get("harness"))
        .and_then(|h| h.get("unsafe-free"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect()
}

pub fn geiger_command(manifest: &Path) -> Vec<OsString> {
    vec![
        "cargo".into(),
        "geiger".into(),
        "--forbid-only".into(),
        "--manifest-path".into(),
        manifest.into(),
        "--output-format".into(),
        "Json".into(),
    ]
}

/// Violations of the unsafe budget for `package`, given `cargo geiger
/// --forbid-only` JSON. The compiler rejects `unsafe` under
/// `#![forbid(unsafe_code)]`, so the gate only has to prove the attribute is
/// present on every target root; a full geiger scan would re-walk the whole
/// dependency tree per crate for no extra guarantee.
pub fn unsafe_findings(package: &str, geiger_json: &str) -> Vec<String> {
    let Ok(report) = serde_json::from_str::<serde_json::Value>(geiger_json) else {
        return vec![format!("{package}: cargo-geiger did not produce JSON")];
    };
    let Some(entry) = report["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["package"]["id"]["name"].as_str() == Some(package))
    else {
        return vec![format!("{package}: missing from cargo-geiger report")];
    };
    if entry["forbids_unsafe"].as_bool() == Some(true) {
        Vec::new()
    } else {
        vec![format!(
            "{package}: a target root lacks #![forbid(unsafe_code)]; add it or move the code to an FFI-owning crate"
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(forbids: bool) -> String {
        serde_json::json!({"packages": [{"package": {"id": {"name": "gpui-agent-domain"}}, "forbids_unsafe": forbids}]}).to_string()
    }

    #[test]
    fn given_a_forbidding_crate_when_scanned_then_no_findings() {
        assert!(unsafe_findings("gpui-agent-domain", &report(true)).is_empty());
    }

    #[test]
    fn given_a_crate_without_forbid_when_scanned_then_it_is_reported() {
        assert_eq!(unsafe_findings("gpui-agent-domain", &report(false)).len(), 1);
    }

    #[test]
    fn given_garbage_or_unknown_package_when_scanned_then_it_fails_closed() {
        assert_eq!(unsafe_findings("gpui-agent-domain", "not json").len(), 1);
        assert_eq!(unsafe_findings("gpui-agent-ports", &report(true)).len(), 1);
    }
}
