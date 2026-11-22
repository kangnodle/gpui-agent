use gpui_agent::McpOAuth;
#[path = "support/oauth_fixture.rs"]
mod oauth_fixture;
use gpui_agent_domain::{McpOAuthCommand as Command,
                        McpOAuthState as State,
                        McpServer};
use serde_json::json;
use std::{io::{Read,
               Write},
          net::TcpListener,
          time::{Duration,
                 Instant}};

#[test]
fn given_configured_scopes_when_the_resource_challenges_then_required_scopes_are_also_requested() -> anyhow::Result<()> {
    let fixture = oauth_fixture::OAuthFixture::new("valid")?;
    let home = tempfile::tempdir()?;
    let oauth = McpOAuth::default();
    let server: McpServer = serde_json::from_value(json!({"name":"remote","url":format!("{}/mcp",fixture.origin),"oauth":{"scopes":["tools:write"]}}))?;
    let pending = oauth.execute(home.path(), &server, Command::Login)?;
    let url = url::Url::parse(pending.authorization_url.as_deref().unwrap())?;
    let scope = url.query_pairs().find(|(key, _)| key == "scope").unwrap().1.into_owned();
    assert_eq!(
        scope.split_whitespace().collect::<std::collections::BTreeSet<_>>(),
        ["tools:read", "tools:write"].into_iter().collect()
    );
    oauth.execute(home.path(), &server, Command::Cancel)?;
    Ok(())
}
#[test]
fn given_a_bearer_grant_without_an_expiry_then_login_and_subsequent_turns_can_use_the_grant() -> anyhow::Result<()> {
    let fixture = oauth_fixture::OAuthFixture::new("no-expiry")?;
    let home = tempfile::tempdir()?;
    let oauth = McpOAuth::default();
    fixture.login(home.path(), &oauth)?;
    assert_eq!(McpOAuth::tokens(home.path(), &[fixture.server()])?["remote"], "fixture-oauth-access");
    Ok(())
}

#[test]
fn given_a_short_lived_token_without_refresh_then_it_remains_usable_until_its_actual_expiry() -> anyhow::Result<()> {
    let fixture = oauth_fixture::OAuthFixture::new("no-refresh")?;
    let home = tempfile::tempdir()?;
    let oauth = McpOAuth::default();
    fixture.login(home.path(), &oauth)?;
    assert_eq!(McpOAuth::tokens(home.path(), &[fixture.server()])?["remote"], "fixture-oauth-access");
    Ok(())
}

#[test]
fn given_an_oauth_login_when_callbacks_are_forged_declined_or_cancelled_then_no_credentials_are_saved() -> anyhow::Result<()> {
    let fixture = oauth_fixture::OAuthFixture::new("valid")?;
    for action in ["wrong-state", "duplicate-code", "wrong-issuer", "decline", "cancel"] {
        let home = tempfile::tempdir()?;
        let oauth = McpOAuth::default();
        let server = fixture.server();
        let pending = oauth.execute(home.path(), &server, Command::Login)?;
        let mut callback = oauth_fixture::callback(pending.authorization_url.as_deref().unwrap(), "fixture-code")?;
        match action {
            | "wrong-state" => {
                callback.set_query(Some("state=forged&code=fixture-code"));
            },
            | "duplicate-code" => {
                callback.query_pairs_mut().append_pair("code", "other");
            },
            | "wrong-issuer" => {
                callback.query_pairs_mut().append_pair("iss", "https://wrong.invalid");
            },
            | "decline" => {
                callback.query_pairs_mut().append_pair("error", "access_denied");
            },
            | _ => {
                oauth.execute(home.path(), &server, Command::Cancel)?;
            },
        }
        if action != "cancel" {
            assert_eq!(reqwest::blocking::get(callback)?.status(), 400);
        }
        assert!(McpOAuth::tokens(home.path(), std::slice::from_ref(&server)).is_err());
        let status = oauth.execute(home.path(), &server, Command::Status)?;
        assert_eq!(
            status.state,
            if action == "cancel" {
                State::Cancelled
            } else if action == "decline" {
                State::Failed
            } else {
                State::Pending
            }
        );
        oauth.execute(home.path(), &server, Command::Cancel)?;
    }
    Ok(())
}

#[test]
fn given_untrusted_oauth_metadata_when_login_starts_then_resource_issuer_and_pkce_are_enforced() -> anyhow::Result<()> {
    for mode in ["wrong-resource", "wrong-issuer", "no-pkce", "duplicate-query"] {
        let fixture = oauth_fixture::OAuthFixture::new(mode)?;
        let home = tempfile::tempdir()?;
        assert!(McpOAuth::default().execute(home.path(), &fixture.server(), Command::Login).is_err());
    }
    let fixture = oauth_fixture::OAuthFixture::new("valid")?;
    let home = tempfile::tempdir()?;
    let oauth = McpOAuth::default();
    fixture.login(home.path(), &oauth)?;
    let changed: McpServer = serde_json::from_value(json!({"name":"remote","url":format!("{}/other",fixture.origin),"oauth":{}}))?;
    assert!(McpOAuth::tokens(home.path(), &[changed]).is_err());
    Ok(())
}

#[test]
fn given_an_http_mcp_when_oauth_completes_then_pkce_credentials_are_scoped_persisted_refreshed_and_removable() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}", listener.local_addr()?);
    let url = origin.clone();
    let host = std::thread::spawn(move || -> anyhow::Result<()> {
        for stream in listener.incoming() {
            let mut stream = stream?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let header_end = loop {
                let n = stream.read(&mut buf)?;
                anyhow::ensure!(n > 0);
                bytes.extend_from_slice(&buf[.. n]);
                if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8(bytes[.. header_end].to_vec())?;
            let len = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + len {
                let n = stream.read(&mut buf)?;
                anyhow::ensure!(n > 0);
                bytes.extend_from_slice(&buf[.. n]);
            }
            let path = headers.split_whitespace().nth(1).unwrap();
            let mut end = false;
            let body = match path {
                "/mcp" => json!({}),
                "/.well-known/oauth-protected-resource/mcp" => json!({"resource":format!("{url}/mcp"),"authorization_servers":[url],"scopes_supported":["tools:read"]}),
                "/.well-known/oauth-authorization-server" => json!({"issuer":url,"authorization_endpoint":format!("{url}/authorize"),"token_endpoint":format!("{url}/token"),"registration_endpoint":format!("{url}/register"),"code_challenge_methods_supported":["S256"]}),
                "/register" => {
                    let registration: serde_json::Value = serde_json::from_slice(&bytes[header_end..])?;
                    anyhow::ensure!(registration["token_endpoint_auth_method"] == "none");
                    anyhow::ensure!(registration["redirect_uris"][0].as_str().unwrap().starts_with("http://127.0.0.1:"));
                    json!({"client_id":"fixture-public","token_endpoint_auth_method":"none"})
                },
                "/token" => {
                    let form: std::collections::HashMap<_,_> = url::form_urlencoded::parse(&bytes[header_end..]).into_owned().collect();
                    anyhow::ensure!(form.get("resource") == Some(&format!("{url}/mcp")));
                    if form["grant_type"] == "authorization_code" {
                        anyhow::ensure!(form["code_verifier"].len() >= 43 && form["code"] == "fixture-code");
                        json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","token_type":"Bearer","expires_in":20})
                    } else {
                        anyhow::ensure!(form["refresh_token"] == "fixture-refresh");
                        end = true;
                        json!({"access_token":"fixture-refreshed","token_type":"Bearer","expires_in":3600})
                    }
                },
                _ => anyhow::bail!("Unexpected fixture path: {path}"),
            }.to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
            if end {
                break;
            }
        }
        Ok(())
    });
    let home = tempfile::tempdir()?;
    let server: McpServer = serde_json::from_value(json!({"name":"remote","url":format!("{origin}/mcp"),"oauth":{}}))?;
    let oauth = McpOAuth::default();
    assert_eq!(oauth.execute(home.path(), &server, Command::Status)?.state, State::SignedOut);
    let pending = oauth.execute(home.path(), &server, Command::Login)?;
    assert_eq!(pending.state, State::Pending);
    let login = url::Url::parse(pending.authorization_url.as_deref().unwrap())?;
    let params: std::collections::HashMap<_, _> = login.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["resource"], format!("{origin}/mcp"));
    assert_eq!(params["scope"], "tools:read");
    let mut callback = url::Url::parse(&params["redirect_uri"])?;
    callback
        .query_pairs_mut()
        .append_pair("state", &params["state"])
        .append_pair("code", "fixture-code");
    let response = reqwest::blocking::Client::new().get(callback).send()?;
    assert!(response.status().is_success());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = oauth.execute(home.path(), &server, Command::Status)?;
        if status.state == State::Connected {
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "OAuth did not complete: {status:?}");
        std::thread::yield_now();
    }
    drop(oauth);
    let tokens = McpOAuth::tokens(home.path(), std::slice::from_ref(&server))?;
    assert_eq!(tokens["remote"], "fixture-refreshed");
    host.join().unwrap()?;
    let oauth = McpOAuth::default();
    assert_eq!(oauth.execute(home.path(), &server, Command::Status)?.state, State::Connected);
    let other = tempfile::tempdir()?;
    assert_eq!(oauth.execute(other.path(), &server, Command::Status)?.state, State::SignedOut);
    oauth.execute(home.path(), &server, Command::Logout)?;
    assert!(McpOAuth::tokens(home.path(), &[server]).is_err());
    Ok(())
}
