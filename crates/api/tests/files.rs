use gpui_agent_api::Api;
use gpui_agent_infrastructure::Settings;
use reqwest::StatusCode;
use serde_json::{Value,
                 json};

const TOKEN: &str = "workspacefilepickertesttoken12345678901234567890";

#[tokio::test]
async fn workspace_file_picker_lists_the_selected_host_workspace_without_reading_or_attaching_files() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir_all(first.join("docs 한글")).unwrap();
    std::fs::create_dir_all(first.join(".git")).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::fs::write(first.join("only-first.txt"), "first\nsecond\nthird\n").unwrap();
    std::fs::write(first.join("docs 한글/readme.md"), "Nested context").unwrap();
    std::fs::write(first.join(".git/private"), "Hidden metadata").unwrap();
    std::fs::write(second.join("only-second.txt"), "Other workspace").unwrap();
    // Given a host with two workspaces, both browsed over authenticated HTTP.
    let mut settings = Settings::discover(first.clone(), Some(first.join("host-state"))).unwrap();
    let private_token = second.join("operator-credential");
    settings.load_host_token(&private_token).unwrap();
    std::fs::rename(&private_token, second.join("moved-credential")).unwrap();
    let app = settings.application().unwrap();
    let workspaces = settings.workspaces(app.clone()).unwrap();
    let api = Api::new(app, TOKEN.into()).unwrap().with_workspaces(workspaces);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(gpui_agent_api::serve(listener, api, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let catalog: Value = client
        .post(format!("{origin}/v1/projects"))
        .bearer_auth(TOKEN)
        .json(&json!({"path":second}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_id = catalog["registered_workspace_id"].as_i64().unwrap();
    let first_id = catalog["default_workspace_id"].as_i64().unwrap();
    for (workspace, expected, absent) in [
        (first_id, "only-first.txt", "only-second.txt"),
        (second_id, "only-second.txt", "only-first.txt"),
    ] {
        // When the client lists the root, then only that workspace's relative
        // paths appear.
        let listing: Value = client
            .get(format!("{origin}/v1/providers/claude/files"))
            .bearer_auth(TOKEN)
            .header("X-KND-Workspace", workspace)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listing["path"], "");
        let entries = listing["entries"].as_array().unwrap();
        assert!(entries.iter().any(|entry| entry["path"] == expected && entry["directory"] == false));
        assert!(!entries.iter().any(|entry| entry["path"] == absent || entry["path"] == ".git"));
        assert!(!entries.iter().any(|entry| entry["path"] == "host-state" || entry["path"] == "moved-credential"));
        assert!(!listing.to_string().contains("Nested context"), "Listing never reads file contents");
    }
    let listing: Value = client
        .get(format!("{origin}/v1/providers/claude/files"))
        .bearer_auth(TOKEN)
        .header("X-KND-Workspace", first_id)
        .query(&[("path", "docs 한글")])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listing["entries"], json!([{"path":"docs 한글/readme.md","directory":false}]));
    // Then traversal, absolute paths, Git metadata and missing authorization
    // are rejected.
    for path in ["../second", ".git", "/private", "C:\\private", "only-first.txt"] {
        assert_eq!(
            client
                .get(format!("{origin}/v1/providers/claude/files"))
                .bearer_auth(TOKEN)
                .header("X-KND-Workspace", first_id)
                .query(&[("path", path)])
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        client.get(format!("{origin}/v1/providers/claude/files")).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    // Selection is separate from the existing explicit, ranged context
    // snapshot.
    for (workspace, path) in [(first_id, "host-state/history.sqlite3"), (second_id, "moved-credential")] {
        let response = client
            .post(format!("{origin}/v1/providers/claude/context"))
            .bearer_auth(TOKEN)
            .header("X-KND-Workspace", workspace)
            .json(&json!({"path":path}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let context: Value = client
        .post(format!("{origin}/v1/providers/claude/context"))
        .bearer_auth(TOKEN)
        .header("X-KND-Workspace", first_id)
        .json(&json!({"path":"only-first.txt","start_line":2,"end_line":2}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(context["text"], "second\n");
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
