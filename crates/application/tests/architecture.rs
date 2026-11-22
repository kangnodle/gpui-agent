use serde_json::Value;
use std::collections::HashSet;

#[path = "architecture/purity.rs"]
mod purity;

fn production_closure(packages: &[Value], start: &str) -> HashSet<String> {
    let mut visited = HashSet::from([start.to_owned()]);
    let mut expanded = HashSet::new();
    let mut pending = vec![start.to_owned()];
    while let Some(name) = pending.pop() {
        if !expanded.insert(name.clone()) {
            continue;
        }
        let package = packages.iter().find(|package| package["name"] == name).unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["kind"] == "dev" {
                continue;
            }
            let name = dependency["name"].as_str().unwrap();
            visited.insert(name.to_owned());
            if dependency["path"].is_string() && packages.iter().any(|package| package["name"] == name) {
                pending.push(name.to_owned());
            }
        }
    }
    visited
}

#[test]
fn production_dependencies_point_inward() {
    let output = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules: &[(&str, &[&str])] = &[
        ("gpui-agent-domain", &["serde", "serde_json", "base64", "url", "serde_yaml_ng"]),
        ("gpui-agent-contracts", &["serde", "gpui-agent-domain"]),
        ("gpui-agent-dci", &["serde_json", "gpui-agent-domain", "gpui-agent-contracts"]),
        ("gpui-agent-presentation", &["gpui-agent-contracts"]),
        ("gpui-agent-ports", &["anyhow", "gpui-agent-domain", "gpui-agent-contracts"]),
        (
            "gpui-agent-application",
            &[
                "anyhow",
                "tokio",
                "serde_json",
                "gpui-agent-domain",
                "gpui-agent-contracts",
                "gpui-agent-dci",
                "gpui-agent-ports",
            ],
        ),
        ("gpui-agent", &["gpui-agent-domain", "gpui-agent-ports", "gpui-agent-process"]),
        ("gpui-agent-process", &[]),
        ("gpui-agent-harness", &[]),
        ("gpui-agent-control", &["gpui-agent-ports", "gpui-agent-contracts", "gpui-agent-process"]),
        (
            "gpui-agent-history",
            &["anyhow", "gpui-agent-ports", "gpui-agent-domain", "rusqlite", "serde_json"],
        ),
        (
            "gpui-agent-infrastructure",
            &[
                "gpui-agent",
                "gpui-agent-domain",
                "gpui-agent-application",
                "gpui-agent-contracts",
                "gpui-agent-history",
            ],
        ),
        ("gpui-agent-api", &["gpui-agent-application", "gpui-agent-contracts"]),
        ("gpui-agent-cli", &["gpui-agent", "gpui-agent-infrastructure"]),
        (
            "gpui-agent-server",
            &["gpui-agent", "gpui-agent-api", "gpui-agent-infrastructure", "gpui-agent-control"],
        ),
        (
            "gpui-agent-desktop",
            &[
                "gpui-agent",
                "gpui-agent-api",
                "gpui-agent-contracts",
                "gpui-agent-infrastructure",
                "gpui-agent-control",
                "gpui-agent-presentation",
            ],
        ),
    ];
    for (name, allowed) in rules {
        let package = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == *name)
            .unwrap();
        if *name == "gpui-agent-harness" {
            let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("harness/Cargo.toml");
            assert_eq!(
                std::path::Path::new(package["manifest_path"].as_str().unwrap()),
                manifest,
                "Given the workspace, when Cargo resolves the harness, then its manifest must be in crates/harness"
            );
        }
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["kind"] == "dev" {
                continue;
            }
            let dependency_name = dependency["name"].as_str().unwrap();
            assert!(!dependency_name.starts_with("codex-"), "Shared guidance must not depend on Codex crates");
            if let Some(path) = dependency["path"].as_str() {
                let workspace = std::path::Path::new(metadata["workspace_root"].as_str().unwrap()).canonicalize().unwrap();
                assert!(
                    std::path::Path::new(path).canonicalize().unwrap().starts_with(&workspace),
                    "Workspace dependencies must remain standalone: {dependency_name}"
                );
            }
            if dependency["path"].is_string()
                || matches!(
                    *name,
                    "gpui-agent-domain"
                        | "gpui-agent-contracts"
                        | "gpui-agent-dci"
                        | "gpui-agent-application"
                        | "gpui-agent-presentation"
                        | "gpui-agent-history"
                        | "gpui-agent-ports"
                )
            {
                assert!(allowed.contains(&dependency_name), "{name} must not depend on {dependency_name}");
            }
            if *name == "gpui-agent-application" && dependency_name == "tokio" {
                for feature in dependency["features"].as_array().unwrap() {
                    let feature = feature.as_str().unwrap();
                    assert!(
                        ["macros", "rt", "sync", "time"].contains(&feature),
                        "Use cases must not enable Tokio host I/O feature {feature}"
                    );
                }
            }
        }
    }
    for member in metadata["workspace_members"].as_array().unwrap() {
        let package = metadata["packages"].as_array().unwrap().iter().find(|p| p["id"] == *member).unwrap();
        let name = package["name"].as_str().unwrap();
        assert!(
            rules.iter().any(|(known, _)| *known == name),
            "Define dependency rules for workspace member {name}"
        );
    }
    for adapter in ["gpui-agent-dci", "gpui-agent", "gpui-agent-control", "gpui-agent-history", "gpui-agent-ports"] {
        let reachable = production_closure(metadata["packages"].as_array().unwrap(), adapter);
        assert!(
            !reachable.contains("gpui-agent-application"),
            "{adapter} must not pull in use-case execution, even transitively"
        );
        if matches!(adapter, "gpui-agent-dci" | "gpui-agent-ports" | "gpui-agent-history") {
            assert!(
                !reachable.contains("tokio"),
                "{adapter} must not pull in a runtime through workspace production dependencies"
            );
        }
    }
}

#[test]
fn dependency_walk_follows_indirect_production_edges_and_excludes_dev_edges() {
    let packages = serde_json::json!([
        {"name": "adapter", "dependencies": [
            {"name": "ports", "kind": null, "path": "ports"},
            {"name": "test-runtime", "kind": "dev", "path": "test-runtime"}
        ]},
        {"name": "ports", "dependencies": [
            {"name": "use-cases", "kind": "dev", "path": "use-cases"},
            {"name": "use-cases", "kind": "build", "path": "use-cases", "target": "cfg(windows)"}
        ]},
        {"name": "use-cases", "dependencies": [
            {"name": "ports", "kind": null, "path": "ports"},
            {"name": "tokio", "kind": null, "path": null}
        ]},
        {"name": "test-runtime", "dependencies": []}
    ]);
    let reachable = production_closure(packages.as_array().unwrap(), "adapter");
    assert!(
        reachable.contains("use-cases"),
        "an indirect dependency through a target-specific build script is still production coupling"
    );
    assert!(reachable.contains("tokio"), "external dependencies of workspace packages must be included");
    assert!(
        !reachable.contains("test-runtime"),
        "test-only dependencies must not count as production coupling"
    );
}
