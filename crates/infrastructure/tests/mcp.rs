use anyhow::Result;
use gpui_agent_application::ports::JobRepository;
use gpui_agent_contracts::{JobScope,
                           McpServer,
                           Provider};
use gpui_agent_infrastructure::{JobStore,
                                Settings};
use serde_json::json;

#[test]
fn given_mixed_mcp_settings_when_the_store_reopens_then_refs_and_stdio_survive_and_invalid_updates_do_not_replace_them() -> Result<()> {
    let root = tempfile::tempdir()?;
    let settings = Settings::discover(root.path().into(), Some(root.path().join("home")))?;
    let workspaces = settings.workspaces(settings.application()?)?;
    let scope = JobScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_id: Some(workspaces.default_workspace().unwrap()),
        workspace_path: settings.cwd.display().to_string(),
    };
    let wire = json!([
        {"name":"stdio","command":std::env::current_exe()?,"args":["--stdio"]},
        {"name":"remote","url":"https://example.invalid/mcp","bearer_token_env_var":"MISSING_UNTIL_RUNTIME"}
    ]);
    let config: Vec<McpServer> = serde_json::from_value(wire.clone())?;
    let store = JobStore::open(settings.clone(), workspaces.clone())?;
    store.mcp(&scope, Some(&config))?;
    drop(store);
    let store = JobStore::open(settings, workspaces)?;
    assert_eq!(serde_json::to_value(store.mcp(&scope, None)?)?, wire);
    let invalid: Vec<McpServer> = serde_json::from_value(json!([{"name":"invalid","command":root.path().join("missing-executable"),"args":[]}]))?;
    assert!(store.mcp(&scope, Some(&invalid)).is_err());
    assert_eq!(serde_json::to_value(store.mcp(&scope, None)?)?, wire);
    Ok(())
}
