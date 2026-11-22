use gpui_agent_domain::{RuntimeInfo,
                        RuntimeRelease};
use serde_json::json;

#[test]
fn given_a_new_registry_version_when_reading_it_then_the_platform_archive_and_checksum_are_offered() {
    let manifest = json!({
        "id":"antigravity-acp", "version":"9.0.0",
        "distribution":{"binary":{
            "windows-x86_64":{
                "archive":"https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-9.0.0-windows-x86_64.zip",
                "sha256":"ab".repeat(32)
            },
            "darwin-aarch64":{
                "archive":"https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-9.0.0-darwin-arm64.zip",
                "sha256":"cd".repeat(32)
            }
        }}
    });
    let windows = RuntimeRelease::antigravity_from_registry(&manifest, "windows", "x86_64").unwrap();
    assert_eq!(windows.version, "9.0.0");
    assert_eq!(windows.sha256, "ab".repeat(32));
    let mac = RuntimeRelease::antigravity_from_registry(&manifest, "darwin", "aarch64").unwrap();
    assert_eq!(mac.sha256, "cd".repeat(32));
    assert!(mac.url.ends_with("9.0.0-darwin-arm64.zip"));
    assert!(RuntimeRelease::antigravity_from_registry(&manifest, "darwin", "x86_64").is_err());
    for (field, value) in [
        ("archive", "https://evil.test/runtime.zip"),
        (
            "archive",
            "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-8.0.0-windows-x86_64.zip",
        ),
        (
            "archive",
            "https://dl.google.com/agy-extensions/releases/windows/agy-acp-server-9.0.0-windows-arm64.zip",
        ),
        ("sha256", "invalid"),
    ] {
        let mut invalid = manifest.clone();
        invalid["distribution"]["binary"]["windows-x86_64"][field] = json!(value);
        assert!(RuntimeRelease::antigravity_from_registry(&invalid, "windows", "x86_64").is_err());
    }
    for version in ["../9.0.0", "", "9/0"] {
        let mut invalid = manifest.clone();
        invalid["version"] = json!(version);
        assert!(RuntimeRelease::antigravity_from_registry(&invalid, "windows", "x86_64").is_err());
    }
}

#[test]
fn given_a_legacy_runtime_response_when_reading_versions_then_its_single_release_remains_selectable() {
    let legacy: RuntimeInfo = serde_json::from_value(json!({
        "provider":"antigravity", "active_path":null, "managed_version":null,
        "override_active":false, "issue":null,
        "release":{"version":"1.4.1","url":"https://dl.google.com/old.zip","sha256":"ab".repeat(32),"size":null}
    }))
    .unwrap();
    assert_eq!(legacy.available_releases().len(), 1);
    assert_eq!(legacy.available_releases()[0].version, "1.4.1");
}
