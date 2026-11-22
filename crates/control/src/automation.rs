use super::*;
use gpui_agent_contracts::{BROWSER_TARGET_LIMIT,
                           BROWSER_TEXT_LIMIT,
                           BrowserAction,
                           BrowserCommand,
                           BrowserReply,
                           BrowserSnapshot,
                           McpScope};
use std::sync::atomic::AtomicBool;

const IDLE: Duration = Duration::from_secs(600);
const CAPACITY: usize = 8;

#[derive(Default)]
pub(super) struct AutomationBrowsers {
    sessions: std::sync::Mutex<Vec<(Option<McpScope>, Arc<Session>)>>,
}
struct Session {
    page: Mutex<Option<Page>>,
    cancel: watch::Sender<u64>,
    closed: AtomicBool,
    used: std::sync::Mutex<std::time::Instant>,
}
struct CancelOnDrop(Option<Arc<Session>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            session.cancel();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    session.close().await;
                });
            }
        }
    }
}
impl Session {
    fn new() -> Arc<Self> {
        let session = Arc::new(Self {
            page: Mutex::new(None),
            cancel: watch::channel(0).0,
            closed: AtomicBool::new(false),
            used: std::sync::Mutex::new(std::time::Instant::now()),
        });
        let weak = Arc::downgrade(&session);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(session) = weak.upgrade() else {
                    break;
                };
                let expired = session.used.lock().map(|used| used.elapsed() >= IDLE).unwrap_or(true);
                if expired || session.closed.load(Ordering::SeqCst) {
                    session.close().await;
                    break;
                }
            }
        });
        session
    }

    fn cancel(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.cancel.send_modify(|value| *value = value.wrapping_add(1));
        if let Ok(mut page) = self.page.try_lock() {
            page.take();
        }
    }

    async fn close(&self) {
        self.cancel();
        self.page.lock().await.take();
    }
}

impl AutomationBrowsers {
    pub(super) async fn execute(&self, executable: Option<&std::path::Path>, scope: Option<McpScope>, command: BrowserCommand) -> Result<BrowserReply> {
        command.validate().map_err(anyhow::Error::msg)?;
        if let BrowserCommand::Open {
            url, ..
        } = &command
        {
            validate_url(url)?;
        }
        let session = {
            let mut sessions = self.sessions.lock().map_err(|_| anyhow::anyhow!("Browser registry lock poisoned"))?;
            sessions.retain(|(_, session)| !session.closed.load(Ordering::SeqCst));
            if matches!(command, BrowserCommand::Close {}) {
                sessions.iter().position(|(owner, _)| owner == &scope).map(|index| sessions.remove(index).1)
            } else if let Some((_, session)) = sessions.iter().find(|(owner, _)| owner == &scope) {
                Some(session.clone())
            } else if matches!(command, BrowserCommand::Open { .. } | BrowserCommand::RenderHtml { .. }) {
                ensure!(
                    sessions.len() < CAPACITY,
                    "At most 8 automation browsers are allowed; close an idle browser first"
                );
                let session = Session::new();
                sessions.push((scope, session.clone()));
                Some(session)
            } else {
                None
            }
        };
        if matches!(command, BrowserCommand::Close {}) {
            if let Some(session) = session {
                session.close().await;
            }
            return Ok(BrowserReply::Closed);
        }
        let Some(session) = session else {
            return Ok(BrowserReply::Rejected("Open an automation browser first"));
        };
        // Dropping a request mid-CDP exchange must not leave a reusable
        // profile.
        let mut cleanup = CancelOnDrop(Some(session.clone()));
        *session.used.lock().map_err(|_| anyhow::anyhow!("Browser activity lock poisoned"))? = std::time::Instant::now();
        let mut cancel = session.cancel.subscribe();
        let work = async {
            let mut slot = session.page.lock().await;
            ensure!(!session.closed.load(Ordering::SeqCst), "Automation browser was closed");
            let opening = match &command {
                | BrowserCommand::Open {
                    mobile, ..
                }
                | BrowserCommand::RenderHtml {
                    mobile, ..
                } => Some(*mobile),
                | _ => None,
            };
            if matches!(command, BrowserCommand::RenderHtml { .. }) || (opening.is_some() && slot.as_ref().is_some_and(|page| page.html_preview)) {
                slot.take();
            }
            if let Some(mobile) = opening
                && slot.is_none()
            {
                let browser = Browser::start(
                    executable.context("Install Edge/Chrome/Chromium or set T3_BROWSER_EXECUTABLE")?,
                    mobile,
                    matches!(command, BrowserCommand::RenderHtml { .. }),
                )
                .await?;
                *slot = Some(Page {
                    browser,
                    context: 0,
                    revision: String::new(),
                    sequence: 0,
                    stamp: format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()),
                    html_preview: matches!(command, BrowserCommand::RenderHtml { .. }),
                });
            }
            slot.as_mut().context("Open an automation browser first")?.execute(command).await
        };
        let result = tokio::select! {
            biased;
            _ = cancel.changed() => Err(anyhow::anyhow!("Browser command cancelled")),
            result = tokio::time::timeout(Duration::from_secs(15), work) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("Browser command timed out"))),
        };
        if result.is_err() {
            session.close().await;
        }
        cleanup.0.take();
        result
    }

    pub(super) fn shutdown(&self) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|error| error.into_inner());
        for (_, session) in sessions.drain(..) {
            session.cancel();
        }
    }
}

struct Page {
    browser: Browser,
    context: i64,
    revision: String,
    sequence: u64,
    stamp: String,
    html_preview: bool,
}
impl Page {
    async fn call_document(&mut self, method: &str, params: Value) -> Result<Value> {
        let session = if self.html_preview { self.browser.preview_session.clone() } else { None };
        self.browser.call_in(session.as_deref(), method, params).await
    }

    async fn evaluate(&mut self, expression: String) -> Result<Value> {
        let result = self
            .call_document(
                "Runtime.evaluate",
                json!({"expression":expression,"contextId":self.context,"returnByValue":true,"awaitPromise":true}),
            )
            .await?;
        ensure!(
            result.get("exceptionDetails").is_none(),
            "Browser document changed or inspection failed; reopen the page"
        );
        Ok(result["result"]["value"].clone())
    }

    async fn snapshot(&mut self) -> Result<BrowserSnapshot> {
        let tree = self.call_document("Page.getFrameTree", json!({})).await?;
        let frame = if self.html_preview && self.browser.preview_session.is_none() {
            tree["frameTree"]["childFrames"]
                .as_array()
                .and_then(|frames| frames.iter().find(|frame| frame["frame"]["name"] == "gpui-html-preview"))
                .context("HTML preview frame is unavailable; render the HTML again")?["frame"]["id"]
                .as_str()
        } else {
            tree["frameTree"]["frame"]["id"].as_str()
        }
        .context("Missing browser frame")?;
        let world = self
            .call_document("Page.createIsolatedWorld", json!({"frameId":frame,"worldName":"gpui-agent-automation"}))
            .await?;
        self.context = world["executionContextId"].as_i64().context("Missing browser execution context")?;
        let value = self
            .evaluate(format!(
                "({})({BROWSER_TEXT_LIMIT},{BROWSER_TARGET_LIMIT})",
                include_str!("browser_snapshot.js")
            ))
            .await?;
        self.sequence = self.sequence.checked_add(1).context("Browser revision exhausted")?;
        self.revision = format!("{}:{}", self.stamp, self.sequence);
        let mut snapshot: BrowserSnapshot = serde_json::from_value(value)?;
        snapshot.revision.clone_from(&self.revision);
        Ok(snapshot)
    }

    async fn execute(&mut self, command: BrowserCommand) -> Result<BrowserReply> {
        match command {
            | BrowserCommand::RenderHtml {
                html,
                mobile,
            } => {
                self.browser.viewport(mobile).await?;
                self.browser.call("Page.enable", json!({})).await?;
                self.browser
                    .call("Target.setAutoAttach", json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}))
                    .await?;
                self.browser.call("Network.enable", json!({})).await?;
                self.browser
                    .call(
                        "Network.setBlockedURLs",
                        json!({"urls":["http://*","https://*","file://*","ftp://*","ws://*","wss://*"]}),
                    )
                    .await?;
                let tree = self.browser.call("Page.getFrameTree", json!({})).await?;
                let frame = tree["frameTree"]["frame"]["id"].as_str().context("Missing browser frame")?;
                self.browser
                    .call(
                        "Page.setDocumentContent",
                        json!({"frameId":frame,"html":gpui_agent_contracts::html_preview_document(&html)}),
                    )
                    .await?;
                loop {
                    let tree = self.browser.call("Page.getFrameTree", json!({})).await?;
                    if self.browser.preview_session.is_some()
                        || tree["frameTree"]["childFrames"].as_array().is_some_and(|frames| {
                            frames
                                .iter()
                                .any(|frame| frame["frame"]["name"] == "gpui-html-preview" && frame["frame"]["url"] == "about:srcdoc")
                        })
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                if let Some(session) = self.browser.preview_session.clone() {
                    self.browser.call_in(Some(&session), "Runtime.enable", json!({})).await?;
                    self.browser.call_in(Some(&session), "Log.enable", json!({})).await?;
                    self.browser.call_in(Some(&session), "Network.enable", json!({})).await?;
                    self.browser
                        .call_in(
                            Some(&session),
                            "Network.setBlockedURLs",
                            json!({"urls":["http://*","https://*","file://*","ftp://*","ws://*","wss://*"]}),
                        )
                        .await?;
                    self.browser.call_in(Some(&session), "Runtime.runIfWaitingForDebugger", json!({})).await?;
                }
            },
            | BrowserCommand::Open {
                url,
                mobile,
            } =>
                self.browser
                    .action(ControlCommand::Open {
                        url,
                        mobile,
                    })
                    .await?,
            | BrowserCommand::Snapshot {} => {},
            | BrowserCommand::Capture {} => return Ok(BrowserReply::Capture(self.browser.frame().await?)),
            | BrowserCommand::Diagnostics {} => {
                self.call_document("Runtime.evaluate", json!({"expression":"0","returnByValue":true})).await?;
                return Ok(BrowserReply::Diagnostics(self.browser.diagnostics.clone()));
            },
            | BrowserCommand::Close {} => unreachable!(),
            | BrowserCommand::Wait {
                text,
                timeout_ms,
            } => {
                let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
                loop {
                    let snapshot = self.snapshot().await?;
                    let matched = self
                        .evaluate(format!("(document.body?.innerText || '').includes({})", serde_json::to_string(&text)?))
                        .await?
                        == true;
                    if matched || tokio::time::Instant::now() >= deadline {
                        return Ok(BrowserReply::Wait {
                            matched,
                            snapshot,
                        });
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
            },
            | BrowserCommand::Act {
                action,
            } => match action {
                | BrowserAction::Key {
                    key,
                } =>
                    self.browser
                        .action(ControlCommand::Key {
                            key,
                        })
                        .await?,
                | BrowserAction::Scroll {
                    delta,
                } =>
                    self.browser
                        .action(ControlCommand::Scroll {
                            delta,
                        })
                        .await?,
                | BrowserAction::Click {
                    revision, ..
                }
                | BrowserAction::Fill {
                    revision, ..
                } if revision != self.revision => return Ok(BrowserReply::Rejected("Browser snapshot is stale; read a new snapshot")),
                | action => {
                    let (target, text) = match action {
                        | BrowserAction::Click {
                            target, ..
                        } => (target, None),
                        | BrowserAction::Fill {
                            target,
                            text,
                            ..
                        } => (target, Some(text)),
                        | _ => unreachable!(),
                    };
                    let result = self
                        .evaluate(format!("({})({target},{})", include_str!("browser_target.js"), serde_json::to_string(&text)?))
                        .await?;
                    if result["error"] == true {
                        return Ok(BrowserReply::Rejected("Target is detached, hidden, covered, disabled, or not a writable input"));
                    }
                    if text.is_none() {
                        let x = result["x"].as_u64().context("Missing target coordinates")? as u32;
                        let y = result["y"].as_u64().context("Missing target coordinates")? as u32;
                        self.browser
                            .action(ControlCommand::Click {
                                x,
                                y,
                            })
                            .await?;
                    }
                },
            },
        }
        Ok(BrowserReply::Snapshot(self.snapshot().await?))
    }
}
