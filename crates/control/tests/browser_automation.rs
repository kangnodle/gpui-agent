use anyhow::{Result,
             bail};
use gpui_agent_contracts::*;
use gpui_agent_control::Controls;
use gpui_agent_ports::HostControl;
use tokio::io::{AsyncReadExt,
                AsyncWriteExt};

fn snapshot(reply: BrowserReply) -> BrowserSnapshot {
    match reply {
        | BrowserReply::Snapshot(value) => value,
        | other => panic!("Expected snapshot: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires a locally installed Chromium browser"]
async fn given_inline_html_when_rendered_then_the_artifact_is_visible_interactive_and_capturable() -> Result<()> {
    // Given a complete HTML result, when rendered through the public host port.
    let controls = Controls::default();
    let command: BrowserCommand = serde_json::from_value(serde_json::json!({
        "type":"render_html", "mobile":true,
        "html":r#"<!doctype html><meta charset="utf-8"><title>HTML result</title>
        <style>body{background:#142238;color:white;font:22px sans-serif;padding:24px}button{padding:16px}</style>
        <h1>HTML 결과</h1><button onclick="document.querySelector('p').innerText='Clicked ✓'">Try it</button><p>Ready</p>
        <p id="mobile">Mobile viewport</p><style>@media(min-width:600px){#mobile{display:none}}</style>"#
    }))?;
    let rendered = snapshot(controls.automate(None, command).await?);
    assert_eq!(rendered.title, "HTML result");
    assert!(rendered.text.contains("HTML 결과"));
    assert!(rendered.text.contains("Mobile viewport"), "Mobile preview must use the viewport's CSS width");
    let target = rendered.targets.iter().find(|target| target.label == "Try it").unwrap().id;
    let clicked = snapshot(
        controls
            .automate(
                None,
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: rendered.revision,
                        target,
                    },
                },
            )
            .await?,
    );
    assert!(clicked.text.contains("Clicked ✓"));
    // Then the same image interface can show the actual result to a user.
    match controls.automate(None, BrowserCommand::Capture {}).await? {
        | BrowserReply::Capture(frame) => {
            assert_eq!((frame.width, frame.height), (390, 844));
            frame.image.validate().map_err(anyhow::Error::msg)?;
            if let Ok(path) = std::env::var("GPUI_HTML_EVIDENCE") {
                std::fs::write(path, frame.image.image_bytes().map_err(anyhow::Error::msg)?)?;
            }
        },
        | other => bail!("Missing HTML capture: {other:?}"),
    }
    controls.automate(None, BrowserCommand::Close {}).await?;
    controls.shutdown();
    Ok(())
}

#[tokio::test]
#[ignore = "requires a locally installed Chromium browser"]
async fn given_broken_html_when_inspected_then_console_errors_are_scoped_bounded_and_reset_on_render() -> Result<()> {
    let controls = Controls::default();
    let rendered = snapshot(
        controls
            .automate(
                None,
                serde_json::from_value(serde_json::json!({
                    "type":"render_html",
                    "html":r#"<title>Diagnostics</title><button onclick="for(let i=0;i<120;i++) console.log('🦀'.repeat(2100))">Flood</button>
                    <script>console.log('render marker');console.error('error marker');throw new Error('broken preview')</script>"#
                }))?,
            )
            .await?,
    );
    let read = || serde_json::from_value::<BrowserCommand>(serde_json::json!({"type":"diagnostics"}));
    let first = serde_json::to_value(match controls.automate(None, read()?).await? {
        | BrowserReply::Diagnostics(value) => value,
        | other => bail!("Missing diagnostics: {other:?}"),
    })?;
    let entries = first["entries"].as_array().unwrap();
    assert!(entries.iter().any(|entry| entry["kind"] == "console" && entry["text"] == "render marker"));
    assert!(entries.iter().any(|entry| entry["level"] == "error" && entry["text"] == "error marker"));
    assert!(
        entries
            .iter()
            .any(|entry| entry["kind"] == "exception" && entry["text"].as_str().unwrap().contains("broken preview"))
    );
    let foreign = McpScope {
        provider: Provider::Claude,
        account_id: "default".into(),
        workspace_path: "other".into(),
    };
    assert!(matches!(controls.automate(Some(foreign), read()?).await?, BrowserReply::Rejected(_)));
    let target = rendered.targets.iter().find(|target| target.label == "Flood").unwrap().id;
    controls
        .automate(
            None,
            BrowserCommand::Act {
                action: BrowserAction::Click {
                    revision: rendered.revision,
                    target,
                },
            },
        )
        .await?;
    match controls.automate(None, read()?).await? {
        | BrowserReply::Diagnostics(value) => {
            assert!(value.truncated);
            assert_eq!(value.entries.len(), 100);
            assert!(value.entries.iter().all(|entry| entry.text.chars().count() <= 2000));
        },
        | other => bail!("Missing bounded diagnostics: {other:?}"),
    }
    controls
        .automate(
            None,
            serde_json::from_value(serde_json::json!({"type":"render_html","html":"<h1>Fresh result</h1>"}))?,
        )
        .await?;
    let fresh = format!("{:?}", controls.automate(None, read()?).await?);
    assert!(!fresh.contains("render marker") && !fresh.contains("broken preview"));
    controls.automate(None, BrowserCommand::Close {}).await?;
    controls.shutdown();
    Ok(())
}

#[tokio::test]
#[ignore = "requires a locally installed Chromium browser"]
async fn given_untrusted_html_when_rendered_then_host_network_parent_storage_and_popups_are_blocked() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let (seen, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepted = connections.clone();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut request = [0; 8192];
                let size = socket.read(&mut request).await.unwrap_or(0);
                let _ = seen.send(
                    String::from_utf8_lossy(&request[.. size])
                        .lines()
                        .next()
                        .unwrap_or("empty connection")
                        .to_owned(),
                );
                let body = "<title>Live website</title><h1>Ordinary browsing works</h1>";
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    let controls = Controls::default();
    let html = format!(
        r#"<title>Sandbox</title><p id="out">Sandbox</p>
        <img src="{url}image"><iframe src="{url}frame"></iframe><script src="{url}script"></script>
        <a target="_top" href="{url}escape">Escape</a><button onclick="window.open('{url}popup')">Popup</button>
        <script>
        try {{parent.document.body.innerHTML='escaped'}} catch {{out.innerText+=' parent blocked'}}
        try {{localStorage.setItem('secret','leak')}} catch {{out.innerText+=' storage blocked'}}
        fetch('{url}fetch').catch(()=>out.innerText+=' network blocked');
        </script>"#
    );
    controls
        .automate(None, serde_json::from_value(serde_json::json!({"type":"render_html","html":html}))?)
        .await?;
    let ready = match controls
        .automate(
            None,
            BrowserCommand::Wait {
                text: "network blocked".into(),
                timeout_ms: 2000,
            },
        )
        .await?
    {
        | BrowserReply::Wait {
            matched: true,
            snapshot,
        } => snapshot,
        | other => bail!("Network did not reject: {other:?}"),
    };
    assert!(ready.text.contains("parent blocked") && ready.text.contains("storage blocked"));
    let popup = ready.targets.iter().find(|target| target.label == "Popup").unwrap().id;
    let after_popup = snapshot(
        controls
            .automate(
                None,
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: ready.revision,
                        target: popup,
                    },
                },
            )
            .await?,
    );
    let escape = after_popup.targets.iter().find(|target| target.label == "Escape").unwrap().id;
    let after_escape = snapshot(
        controls
            .automate(
                None,
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: after_popup.revision,
                        target: escape,
                    },
                },
            )
            .await?,
    );
    assert_eq!(after_escape.title, "Sandbox");
    let received = requests.try_recv();
    assert!(received.is_err(), "Untrusted HTML contacted the host network: {received:?}");
    assert_eq!(
        connections.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "HTML opened a speculative network connection"
    );
    // Ordinary URL browsing replaces the sandbox with a new profile and remains
    // usable.
    let live = snapshot(
        controls
            .automate(
                None,
                BrowserCommand::Open {
                    url,
                    mobile: false,
                },
            )
            .await?,
    );
    assert_eq!(live.title, "Live website");
    tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv()).await?.unwrap();
    controls.automate(None, BrowserCommand::Close {}).await?;
    controls.shutdown();
    server.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires a locally installed Chromium browser"]
async fn given_isolated_pages_when_an_agent_acts_then_dom_targets_images_waits_and_scope_boundaries_hold() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = [0; 8192];
                let size = socket.read(&mut request).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&request[.. size]);
                let body = if request.starts_with("GET /large") {
                    format!("<body>{}{}</body>", "🦀".repeat(18000), "<button>target</button>".repeat(120))
                } else {
                    r#"<!doctype html><meta charset="utf-8"><title>Owned automation</title><body>
                    <label>Name<input id="name" aria-label="Name"></label><input type="password" value="must-not-leak">
                    <button id="save" onclick="out.innerText='Saved:'+document.getElementById('name').value;localStorage.setItem('marker',document.getElementById('name').value)">Save</button>
                    <button onclick="setTimeout(()=>out.innerText='Async complete',100)">Async</button>
                    <input aria-label="Read only" readonly><button disabled>Disabled</button><button style="display:none">Hidden</button>
                    <div id="out"></div><div id="remembered"></div>
                    <script>remembered.innerText=localStorage.getItem('marker')||'clean profile'</script></body>"#.into()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let controls = Controls::default();
    let owner = McpScope {
        provider: Provider::Codex,
        account_id: "default".into(),
        workspace_path: "fixture-workspace".into(),
    };
    let url = format!("http://{address}/");
    let first = snapshot(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Open {
                    url: url.clone(),
                    mobile: false,
                },
            )
            .await?,
    );
    assert_eq!(first.title, "Owned automation");
    assert!(!serde_json::to_string(&first)?.contains("must-not-leak"));
    assert!(!first.targets.iter().any(|target| target.label == "Hidden"));
    let disabled = first.targets.iter().find(|target| target.label == "Disabled").unwrap().id;
    assert!(matches!(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: first.revision.clone(),
                        target: disabled
                    }
                }
            )
            .await?,
        BrowserReply::Rejected(_)
    ));
    let readonly = first.targets.iter().find(|target| target.label == "Read only").unwrap().id;
    assert!(matches!(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Act {
                    action: BrowserAction::Fill {
                        revision: first.revision.clone(),
                        target: readonly,
                        text: "rejected".into()
                    }
                }
            )
            .await?,
        BrowserReply::Rejected(_)
    ));
    for invalid in ["file:///etc/passwd", "javascript:alert(1)", "https://user:secret@example.com/"] {
        assert!(
            controls
                .automate(
                    Some(owner.clone()),
                    BrowserCommand::Open {
                        url: invalid.into(),
                        mobile: false
                    }
                )
                .await
                .is_err()
        );
    }
    let target = first.targets.iter().find(|target| target.label == "Name").unwrap().id;
    let filled = snapshot(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Act {
                    action: BrowserAction::Fill {
                        revision: first.revision.clone(),
                        target,
                        text: "O'Reilly 한글".into(),
                    },
                },
            )
            .await?,
    );
    assert!(matches!(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: first.revision,
                        target
                    }
                }
            )
            .await?,
        BrowserReply::Rejected(_)
    ));
    let save = filled.targets.iter().find(|target| target.label == "Save").unwrap().id;
    let saved = snapshot(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: filled.revision,
                        target: save,
                    },
                },
            )
            .await?,
    );
    assert!(saved.text.contains("Saved:O'Reilly 한글"));
    let async_target = saved.targets.iter().find(|target| target.label == "Async").unwrap().id;
    controls
        .automate(
            Some(owner.clone()),
            BrowserCommand::Act {
                action: BrowserAction::Click {
                    revision: saved.revision,
                    target: async_target,
                },
            },
        )
        .await?;
    match controls
        .automate(
            Some(owner.clone()),
            BrowserCommand::Wait {
                text: "Async complete".into(),
                timeout_ms: 2000,
            },
        )
        .await?
    {
        | BrowserReply::Wait {
            matched: true, ..
        } => {},
        | other => bail!("Wait failed: {other:?}"),
    }
    assert!(matches!(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Wait {
                    text: "never appears".into(),
                    timeout_ms: 0
                }
            )
            .await?,
        BrowserReply::Wait {
            matched: false,
            ..
        }
    ));
    match controls.automate(Some(owner.clone()), BrowserCommand::Capture {}).await? {
        | BrowserReply::Capture(frame) => {
            assert_eq!((frame.width, frame.height), (1024, 768));
            frame.image.validate().map_err(anyhow::Error::msg)?;
        },
        | other => bail!("Missing image: {other:?}"),
    }
    for other in [
        McpScope {
            provider: Provider::Claude,
            ..owner.clone()
        },
        McpScope {
            account_id: "a".repeat(32),
            ..owner.clone()
        },
        McpScope {
            workspace_path: "other-workspace".into(),
            ..owner.clone()
        },
    ] {
        assert!(matches!(
            controls.automate(Some(other.clone()), BrowserCommand::Snapshot {}).await?,
            BrowserReply::Rejected(_)
        ));
        let page = snapshot(
            controls
                .automate(
                    Some(other.clone()),
                    BrowserCommand::Open {
                        url: url.clone(),
                        mobile: true,
                    },
                )
                .await?,
        );
        assert!(page.text.contains("clean profile") && !page.text.contains("O'Reilly"));
        controls.automate(Some(other), BrowserCommand::Close {}).await?;
    }
    assert!(!controls.status().await?.browser_running, "automation must not reuse the manual browser");
    assert!(matches!(controls.automate(None, BrowserCommand::Snapshot {}).await?, BrowserReply::Rejected(_)));
    let persisted = snapshot(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Open {
                    url: url.clone(),
                    mobile: false,
                },
            )
            .await?,
    );
    assert!(persisted.text.contains("O'Reilly 한글"));
    let large = snapshot(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Open {
                    url: format!("http://{address}/large"),
                    mobile: false,
                },
            )
            .await?,
    );
    assert!(large.text_truncated && large.targets_truncated);
    assert_eq!(large.text.chars().count(), BROWSER_TEXT_LIMIT);
    assert_eq!(large.targets.len(), BROWSER_TARGET_LIMIT);
    controls.automate(Some(owner.clone()), BrowserCommand::Close {}).await?;
    controls.automate(Some(owner.clone()), BrowserCommand::Close {}).await?;
    let reset = snapshot(
        controls
            .automate(
                Some(owner.clone()),
                BrowserCommand::Open {
                    url,
                    mobile: false,
                },
            )
            .await?,
    );
    assert!(reset.text.contains("clean profile"));
    assert!(matches!(
        controls
            .automate(
                Some(owner),
                BrowserCommand::Act {
                    action: BrowserAction::Click {
                        revision: large.revision,
                        target: 0
                    }
                }
            )
            .await?,
        BrowserReply::Rejected(_)
    ));
    controls.shutdown();
    server.abort();
    Ok(())
}

#[tokio::test]
#[ignore = "requires a locally installed Chromium browser"]
async fn given_a_hanging_navigation_when_the_request_is_dropped_then_the_protocol_and_profile_are_discarded() -> Result<()> {
    use std::sync::Arc;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (seen, mut received) = tokio::sync::mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut request = [0; 8192];
                let _ = socket.read(&mut request).await;
                let _ = seen.send(());
                std::future::pending::<()>().await;
            });
        }
    });
    let controls = Arc::new(Controls::default());
    let owned = controls.clone();
    let request = tokio::spawn(async move {
        owned
            .automate(
                None,
                BrowserCommand::Open {
                    url: format!("http://{address}/hang"),
                    mobile: false,
                },
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), received.recv()).await?.unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert!(
        matches!(controls.automate(None, BrowserCommand::Snapshot {}).await?, BrowserReply::Rejected(_)),
        "A cancelled exchange must not be reused"
    );
    let owned = controls.clone();
    let request = tokio::spawn(async move {
        owned
            .automate(
                None,
                BrowserCommand::Open {
                    url: format!("http://{address}/hang"),
                    mobile: false,
                },
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), received.recv()).await?.unwrap();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(3), controls.automate(None, BrowserCommand::Close {})).await??,
        BrowserReply::Closed
    ));
    assert!(request.await?.is_err());
    controls.shutdown();
    server.abort();
    Ok(())
}
