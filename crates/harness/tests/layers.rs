//! Cargo metadata includes renamed, build and target-specific dependencies.
//! Keep the approved workspace graph independent of the build tool.
use serde_json::Value;
use std::collections::{BTreeMap,
                       BTreeSet};

#[allow(dead_code)]
mod support;

type Graph = BTreeMap<String, BTreeSet<String>>;

fn short(package: &str) -> &str {
    if package == "gpui-agent" {
        "agent"
    } else {
        package.strip_prefix("gpui-agent-").unwrap_or(package)
    }
}

fn graphs(metadata: &Value) -> (Graph, Graph) {
    let members = metadata["workspace_members"].as_array().unwrap();
    let packages: Vec<_> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|package| members.contains(&package["id"]))
        .collect();
    let names: BTreeSet<_> = packages.iter().map(|package| package["name"].as_str().unwrap()).collect();
    let (mut normal, mut dev) = (Graph::new(), Graph::new());
    for package in &packages {
        let (mut runtime, mut tests) = (BTreeSet::new(), BTreeSet::new());
        for dependency in package["dependencies"].as_array().unwrap() {
            let name = dependency["name"].as_str().unwrap();
            if names.contains(name) {
                let graph = if dependency["kind"] == "dev" { &mut tests } else { &mut runtime };
                graph.insert(short(name).to_owned());
            }
        }
        let name = short(package["name"].as_str().unwrap()).to_owned();
        let tests: BTreeSet<_> = tests.difference(&runtime).cloned().collect();
        normal.insert(name.clone(), runtime);
        if !tests.is_empty() {
            dev.insert(name, tests);
        }
    }
    (normal, dev)
}

#[test]
fn given_the_workspace_when_checked_then_all_dependency_kinds_match_the_approved_layers() {
    let root = support::root();
    let policy: BTreeMap<String, Graph> = toml::from_str(&std::fs::read_to_string(root.join("tools/layers.toml")).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--locked"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let (normal, dev) = graphs(&metadata);
    assert_eq!(
        normal, policy["normal"],
        "Review runtime/build dependency changes in tools/layers.toml and docs/architecture.md"
    );
    assert_eq!(
        dev, policy["dev"],
        "Review test-only dependency changes in tools/layers.toml and docs/requirements/architecture.sdoc (ARCH-INV-7)"
    );
}

#[test]
fn given_renamed_and_target_specific_edges_when_checked_then_build_edges_cannot_hide_as_test_dependencies() {
    let metadata = serde_json::json!({
        "workspace_members": ["domain-id", "ports-id", "app-id", "new-id"],
        "packages": [
            {"id": "domain-id", "name": "gpui-agent-domain", "dependencies": []},
            {"id": "ports-id", "name": "gpui-agent-ports", "dependencies": [
                {"name": "gpui-agent-domain", "rename": "model", "kind": null},
                {"name": "gpui-agent-application", "kind": "build", "target": "cfg(windows)"},
                {"name": "gpui-agent-application", "kind": "dev"},
                {"name": "serde", "kind": null}
            ]},
            {"id": "app-id", "name": "gpui-agent-application", "dependencies": [
                {"name": "gpui-agent-domain", "kind": "dev", "target": "cfg(unix)"}
            ]},
            {"id": "new-id", "name": "new-member", "dependencies": []},
            {"id": "external-id", "name": "serde", "dependencies": []}
        ]
    });
    let (normal, dev) = graphs(&metadata);
    let expected: BTreeMap<String, Graph> = toml::from_str(
        r#"
        [normal]
        domain = []
        ports = ["domain", "application"]
        application = []
        new-member = []
        [dev]
        application = ["domain"]
    "#,
    )
    .unwrap();
    assert_eq!(normal, expected["normal"]);
    assert_eq!(dev, expected["dev"]);
}
