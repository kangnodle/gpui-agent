use super::*;
use gpui_agent_application::ports::ConversationRepository;
use gpui_agent_contracts::{Completion,
                           Prompt};

fn conversation(host: &Host, provider: Provider) -> Result<i64> {
    let history = gpui_agent_infrastructure::History::open(
        &host.root.path().join("home/history.sqlite3"),
        Path::new(&host.catalog.projects[0].workspaces[0].path),
    )?;
    let mut prompt = Prompt {
        mcp_servers: None,
        skills: vec![],
        guidance: None,
        text: "Review this change".into(),
        model: None,
        session_id: None,
        attachments: vec![],
        claude_permission_mode: None,
        codex_execution_policy: None,
    };
    let turn = history.begin(provider, None, &mut prompt)?;
    history.finish(
        turn,
        Some(&Completion {
            session_id: "fixture-session".into(),
            text: "Ready".into(),
        }),
    )?;
    Ok(turn.conversation_id)
}

#[tokio::test]
async fn linked_pull_requests_are_idempotent_private_and_removable_without_a_native_provider() -> Result<()> {
    // Given a saved conversation and read/write credentials in its scope.
    let host = Host::with_jobs(true).await?;
    let id = conversation(&host, Provider::Codex)?;
    let args = json!({"provider":"codex","id":id,"url":"https://github.com/Owner/Repo/pull/7"});
    // When the same canonical PR is linked concurrently, then only one link
    // exists.
    let (a, b) = tokio::join!(
        host.call(&host.writer, "link_pull_request", args.clone()),
        host.call(&host.writer, "link_pull_request", args.clone())
    );
    let replies = [a?, b?];
    assert!(replies.iter().all(|v| v["result"]["isError"] == false), "{replies:?}");
    assert_eq!(
        replies.iter().filter(|v| v["result"]["structuredContent"]["already_linked"] == false).count(),
        1
    );
    let list = json!({"provider":"codex","id":id});
    let reply = host.call(&host.reader, "list_thread_pull_requests", list.clone()).await?;
    let links = reply["result"]["structuredContent"]["items"].as_array().expect("links");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["target"]["repository"], "owner/repo");
    assert_eq!(host.call(&host.reader, "link_pull_request", args.clone()).await?["error"]["code"], -32001);
    let mut foreign = list.clone();
    foreign["workspace"] = json!(host.catalog.projects[1].workspaces[0].id);
    assert_eq!(host.call(&host.writer, "list_thread_pull_requests", foreign).await?["result"]["isError"], true);
    assert_eq!(
        host.call(&host.writer, "list_thread_pull_requests", json!({"provider":"claude","id":id}))
            .await?["result"]["isError"],
        true
    );
    let account = host.store.as_ref().unwrap().create_account(Provider::Codex, "Other")?;
    assert_eq!(
        host.call(
            &host.writer,
            "list_thread_pull_requests",
            json!({"provider":"codex","account":account.id,"id":id})
        )
        .await?["result"]["isError"],
        true
    );
    for url in [
        "https://token@github.com/o/r/pull/1",
        "https://evil.test/o/r/pull/1",
        "https://github.com/o/r/pull/0",
        "https://github.com/o/r/pull/1?x=y",
    ] {
        let mut bad = args.clone();
        bad["url"] = json!(url);
        assert_eq!(host.call(&host.writer, "link_pull_request", bad).await?["result"]["isError"], true);
    }
    assert_eq!(
        host.call(&host.writer, "unlink_pull_request", args.clone()).await?["result"]["structuredContent"]["was_linked"],
        true
    );
    assert_eq!(
        host.call(&host.writer, "unlink_pull_request", args).await?["result"]["structuredContent"]["was_linked"],
        false
    );
    assert_eq!(
        host.call(&host.reader, "list_thread_pull_requests", list).await?["result"]["structuredContent"]["items"],
        json!([])
    );
    host.stop().await
}
