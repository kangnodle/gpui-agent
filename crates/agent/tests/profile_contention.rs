use anyhow::{Result,
             ensure};
use gpui_agent::{Agent,
                 Config,
                 Event,
                 Prompt,
                 Provider};
use std::{fs::OpenOptions,
          time::Duration};

// Given a profile that stays locked, when a prompt waits for it, then the
// configured deadline includes that wait and the retry notice is emitted once.
#[tokio::test]
async fn given_a_locked_profile_when_a_prompt_times_out_then_waiting_is_bounded_without_repeated_notices() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let agent = Agent::new(Config {
        provider: Provider::Codex,
        home: home.clone(),
        cwd: root.path().into(),
        executable: root.path().join("must-not-start-native-runtime"),
        harness: None,
        browser_helper: None,
        timeout: Duration::from_millis(100),
        mcp_home: None,
        mcp_selection: None,
        plugin_mcp: vec![],
    })?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join(".operation.lock"))?;
    lock.try_lock()?;
    let mut events = Vec::new();
    let error = agent
        .prompt(
            &Prompt {
                text: "Synthetic profile contention".into(),
                ..Default::default()
            },
            &mut |event| {
                events.push(event);
                false
            },
        )
        .await
        .expect_err("A held profile must exhaust the prompt deadline");
    ensure!(error.is::<tokio::time::error::Elapsed>(), "The prompt deadline must cover lock acquisition");
    ensure!(
        events
            .iter()
            .filter(|event| matches!(event, Event::RuntimeStatus { code: Some(code), .. } if code == "profile_busy"))
            .count()
            == 1
    );
    Ok(())
}
