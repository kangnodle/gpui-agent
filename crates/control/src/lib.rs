#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! Owned Chromium automation, manual browser and Windows control adapters.
mod automation;
mod computer;
mod devices;
mod diagnostics;
#[cfg(windows)]
mod overlay;
mod windows;
use anyhow::{Context,
             Result,
             bail,
             ensure};
use futures_util::{SinkExt,
                   StreamExt};
use gpui_agent_contracts::{Attachment,
                           ControlCommand,
                           ControlFrame,
                           ControlStatus};
use gpui_agent_ports::{HostControl,
                       RuntimeFuture};
use serde_json::{Value,
                 json};
use std::{path::PathBuf,
          sync::{Arc,
                 atomic::{AtomicBool,
                          AtomicI64,
                          AtomicIsize,
                          AtomicU64,
                          Ordering}},
          time::Duration};
use tokio::{net::TcpStream,
            sync::{Mutex,
                   watch}};
use tokio_tungstenite::{MaybeTlsStream,
                        WebSocketStream,
                        tungstenite::{Message,
                                      protocol::WebSocketConfig}};
pub use windows::allow_computer_foreground;

pub struct Controls {
    desktop_driver: Option<Arc<dyn DesktopDriver>>,
    computer_job: Arc<AtomicI64>,
    devices: devices::Devices,
    automation: automation::AutomationBrowsers,
    executable: Option<PathBuf>,
    browser: Mutex<Option<Browser>>,
    cancel: watch::Sender<u64>,
    desktop_until: Arc<AtomicU64>,
    automatic_desktop: Arc<AtomicBool>,
    desktop_target: Arc<AtomicIsize>,
    desktop_lock: Arc<std::sync::Mutex<computer::DesktopState>>,
    #[cfg(windows)]
    indicator_generation: Arc<AtomicU64>,
}
fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }

impl Default for Controls {
    fn default() -> Self {
        Self {
            desktop_driver: None,
            computer_job: Arc::new(AtomicI64::new(0)),
            devices: devices::Devices::new(devices::discover()),
            automation: automation::AutomationBrowsers::default(),
            executable: find_browser(),
            browser: Mutex::new(None),
            cancel: watch::channel(0).0,
            desktop_until: Arc::new(AtomicU64::new(0)),
            automatic_desktop: Arc::new(AtomicBool::new(false)),
            desktop_target: Arc::new(AtomicIsize::new(0)),
            desktop_lock: Arc::new(std::sync::Mutex::new(computer::DesktopState::default())),
            #[cfg(windows)]
            indicator_generation: Arc::new(AtomicU64::new(0)),
        }
    }
}
impl Controls {
    pub fn with_automatic_desktop(self) -> Self {
        if cfg!(windows) || self.desktop_driver.is_some() {
            self.automatic_desktop.store(true, Ordering::SeqCst);
        }
        self
    }

    /// Replace only native screen/input I/O; grants and session policy stay
    /// intact.
    pub fn with_desktop_driver(mut self, driver: Arc<dyn DesktopDriver>) -> Self {
        self.desktop_driver = Some(driver);
        self
    }

    pub fn with_adb(mut self, executable: PathBuf) -> Result<Self> {
        ensure!(executable.is_absolute() && executable.is_file(), "ADB must be an absolute executable file");
        self.devices = devices::Devices::new(Some(executable));
        Ok(self)
    }
}
/// Native screen/input boundary. Drivers must recheck `allowed` immediately
/// before any focus or input side effect and before returning captured pixels.
pub trait DesktopDriver: Send + Sync {
    fn execute(&self, command: ControlCommand, target: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>>;
}
fn find_browser() -> Option<PathBuf> {
    let mut candidates = vec![];
    if let Some(path) = std::env::var_os("T3_BROWSER_EXECUTABLE") {
        candidates.push(PathBuf::from(path));
    }
    for root in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(root) {
            for exe in ["Microsoft/Edge/Application/msedge.exe", "Google/Chrome/Application/chrome.exe"] {
                candidates.push(PathBuf::from(&root).join(exe));
            }
        }
    }
    candidates.extend(
        [
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ]
        .map(PathBuf::from),
    );
    candidates.into_iter().find(|path| path.is_absolute() && path.is_file())
}
impl HostControl for Controls {
    fn automatic_computer_enabled(&self) -> bool { self.automatic_desktop.load(Ordering::SeqCst) }

    fn computer_status(&self) -> RuntimeFuture<'_, gpui_agent_contracts::ComputerStatus> { Box::pin(self.computer_state()) }

    fn computer(
        &self,
        session: gpui_agent_ports::ComputerSession,
        command: gpui_agent_contracts::ComputerCommand,
    ) -> RuntimeFuture<'_, Option<gpui_agent_contracts::ComputerFrame>> {
        Box::pin(self.computer_command(session, command))
    }

    fn grant_computer(&self, grant: gpui_agent_contracts::ComputerGrantRequest, active: gpui_agent_ports::ControlValidity) -> RuntimeFuture<'_, ()> {
        Box::pin(self.computer_grant(grant, active))
    }

    fn device(
        &self,
        scope: Option<gpui_agent_contracts::McpScope>,
        command: gpui_agent_contracts::DeviceCommand,
    ) -> RuntimeFuture<'_, gpui_agent_contracts::DeviceReply> {
        Box::pin(self.devices.execute(scope, command))
    }

    fn grant_device(&self, grant: gpui_agent_contracts::DeviceGrantRequest) -> RuntimeFuture<'_, ()> { Box::pin(self.devices.grant(grant)) }

    fn automate(
        &self,
        scope: Option<gpui_agent_contracts::McpScope>,
        command: gpui_agent_contracts::BrowserCommand,
    ) -> RuntimeFuture<'_, gpui_agent_contracts::BrowserReply> {
        Box::pin(self.automation.execute(self.executable.as_deref(), scope, command))
    }

    fn status(&self) -> RuntimeFuture<'_, ControlStatus> {
        Box::pin(async {
            Ok(ControlStatus {
                browser_available: self.executable.is_some(),
                browser_running: self.browser.lock().await.is_some(),
                desktop_available: cfg!(windows) || self.desktop_driver.is_some(),
                desktop_enabled_until: self.desktop_until.load(Ordering::SeqCst),
            })
        })
    }

    fn browser(&self, command: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> {
        Box::pin(async move {
            command.validate().map_err(anyhow::Error::msg)?;
            if matches!(command, ControlCommand::Stop {}) {
                self.cancel.send_modify(|v| *v = v.wrapping_add(1));
                self.browser.lock().await.take();
                return Ok(None);
            }
            let mut cancel = self.cancel.subscribe();
            let mut slot = self.browser.lock().await;
            let work = async {
                if let ControlCommand::Open {
                    url,
                    mobile,
                } = &command
                {
                    validate_url(url)?;
                    if slot.is_none() {
                        *slot = Some(
                            Browser::start(
                                self.executable
                                    .as_ref()
                                    .context("Install Edge/Chrome/Chromium or set T3_BROWSER_EXECUTABLE to an absolute executable")?,
                                *mobile,
                                false,
                            )
                            .await?,
                        );
                    }
                }
                let browser = slot.as_mut().context("Open the embedded browser first")?;
                browser.action(command).await?;
                Ok(Some(browser.frame().await?))
            };
            let result = tokio::select! {
                result = tokio::time::timeout(Duration::from_secs(15), work) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("Browser command timed out"))),
                _ = cancel.changed() => Err(anyhow::anyhow!("Browser command cancelled")),
            };
            // A timed-out protocol exchange cannot be safely reused.
            if result.is_err() {
                slot.take();
            }
            result
        })
    }

    fn desktop(&self, command: ControlCommand) -> RuntimeFuture<'_, Option<ControlFrame>> {
        let driver = self.desktop_driver.clone();
        let until = self.desktop_until.clone();
        let automatic = self.automatic_desktop.clone();
        let lock = self.desktop_lock.clone();
        let target = self.desktop_target.clone();
        Box::pin(async move {
            command.validate().map_err(anyhow::Error::msg)?;
            tokio::task::spawn_blocking(move || {
                let mut state = lock.lock().map_err(|_| anyhow::anyhow!("Desktop lock poisoned"))?;
                ensure!(!state.granted(), "An agent holds the desktop computer grant");
                if state.computer_target {
                    target.store(0, Ordering::SeqCst);
                    state.computer_target = false;
                }
                ensure!(
                    automatic.load(Ordering::SeqCst) || until.load(Ordering::SeqCst) > now(),
                    "Enable Windows desktop control first (10 minute grant)"
                );
                let allowed = || automatic.load(Ordering::SeqCst) || until.load(Ordering::SeqCst) > now();
                match driver {
                    | Some(driver) => driver.execute(command, &target, &allowed),
                    | None => windows::execute(command, &target, &allowed),
                }
            })
            .await?
        })
    }

    fn enable_desktop(&self, enabled: bool) -> RuntimeFuture<'_, ()> {
        if !enabled {
            #[cfg(windows)]
            self.indicator_generation.fetch_add(1, Ordering::SeqCst);
            self.computer_job.store(0, Ordering::SeqCst);
        }
        Box::pin(async move {
            let mut state = self.desktop_lock.lock().map_err(|_| anyhow::anyhow!("Desktop lock poisoned"))?;
            if enabled {
                ensure!(!state.granted(), "An agent holds the desktop computer grant");
            } else {
                state.clear();
            }
            ensure!(
                !enabled || cfg!(windows) || self.desktop_driver.is_some(),
                "Desktop control is supported only on Windows"
            );
            self.desktop_target.store(0, Ordering::SeqCst);
            self.desktop_until.store(if enabled { now() + 600 } else { 0 }, Ordering::SeqCst);
            Ok(())
        })
    }

    fn shutdown(&self) {
        #[cfg(windows)]
        self.indicator_generation.fetch_add(1, Ordering::SeqCst);
        self.automatic_desktop.store(false, Ordering::SeqCst);
        self.computer_job.store(0, Ordering::SeqCst);
        self.devices.shutdown();
        self.automation.shutdown();
        self.desktop_until.store(0, Ordering::SeqCst);
        self.cancel.send_modify(|v| *v = v.wrapping_add(1));
        if let Ok(mut browser) = self.browser.try_lock() {
            browser.take();
        }
    }
}
impl Drop for Controls {
    fn drop(&mut self) { self.shutdown(); }
}

fn validate_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() && parsed.username().is_empty() && parsed.password().is_none(),
        "Only HTTP(S) browser URLs without credentials are supported"
    );
    Ok(())
}

struct Browser {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next: u64,
    width: u32,
    height: u32,
    preview_session: Option<String>,
    diagnostics: gpui_agent_contracts::BrowserDiagnostics,
    _network_sink: Option<tokio::net::TcpListener>,
    _process: gpui_agent_process::Process,
    _profile: tempfile::TempDir,
}
impl Browser {
    async fn start(executable: &std::path::Path, mobile: bool, offline: bool) -> Result<Self> {
        let profile = tempfile::tempdir()?;
        let mut command = tokio::process::Command::new(executable);
        let network_sink = if offline {
            // CSP/CDP block requests; this owned, non-forwarding proxy also
            // traps speculative connections.
            let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            command
                .arg(format!("--proxy-server=http://{}", sink.local_addr()?))
                .arg("--proxy-bypass-list=<-loopback>")
                .arg("--host-resolver-rules=MAP * ~NOTFOUND")
                .arg("--force-webrtc-ip-handling-policy=disable_non_proxied_udp");
            Some(sink)
        } else {
            None
        };
        command
            .args([
                "--headless=new",
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--disable-sync",
                "--disable-background-networking",
            ])
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg("about:blank")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        #[cfg(unix)]
        command.process_group(0);
        let mut process = gpui_agent_process::spawn(&mut command)?;
        let port = loop {
            ensure!(process.try_wait()?.is_none(), "Browser exited before initialization");
            if let Ok(active) = tokio::fs::read_to_string(profile.path().join("DevToolsActivePort")).await
                && let Some(port) = active.lines().next().and_then(|port| port.parse::<u16>().ok())
            {
                break port;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()?;
        let target: Value = client
            .put(format!("http://127.0.0.1:{port}/json/new?about:blank"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let endpoint = target["webSocketDebuggerUrl"].as_str().context("Browser omitted its page endpoint")?;
        let parsed = reqwest::Url::parse(endpoint)?;
        ensure!(
            parsed.scheme() == "ws" && parsed.host_str() == Some("127.0.0.1") && parsed.port() == Some(port),
            "Unexpected browser endpoint"
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(4 * 1024 * 1024))
            .max_frame_size(Some(4 * 1024 * 1024));
        let (socket, _) = tokio_tungstenite::connect_async_with_config(endpoint, Some(config), false).await?;
        let mut browser = Self {
            socket,
            next: 0,
            width: 1024,
            height: 768,
            preview_session: None,
            diagnostics: gpui_agent_contracts::BrowserDiagnostics::default(),
            _network_sink: network_sink,
            _process: process,
            _profile: profile,
        };
        browser.viewport(mobile).await?;
        browser.call("Browser.setDownloadBehavior", json!({"behavior":"deny"})).await?;
        browser.call("Runtime.enable", json!({})).await?;
        browser.call("Log.enable", json!({})).await?;
        Ok(browser)
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value> { self.call_in(None, method, params).await }

    async fn call_in(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        let mut request = json!({"id":self.next,"method":method,"params":params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        self.socket.send(Message::Text(request.to_string().into())).await?;
        while let Some(message) = self.socket.next().await {
            let message = message?;
            if let Message::Text(text) = message {
                let response: Value = serde_json::from_str(&text)?;
                self.record_diagnostic(&response);
                if response["method"] == "Target.attachedToTarget" && response["params"]["targetInfo"]["type"] == "iframe" && self.preview_session.is_none() {
                    self.preview_session = response["params"]["sessionId"].as_str().map(str::to_owned);
                }
                if response["id"].as_u64() == Some(self.next) {
                    if let Some(error) = response.get("error") {
                        bail!("Browser: {error}");
                    }
                    return Ok(response["result"].clone());
                }
            }
        }
        bail!("Browser connection closed")
    }

    async fn viewport(&mut self, mobile: bool) -> Result<()> {
        (self.width, self.height) = if mobile { (390, 844) } else { (1024, 768) };
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({"width":self.width,"height":self.height,"deviceScaleFactor":1,"mobile":mobile}),
        )
        .await?;
        Ok(())
    }

    async fn action(&mut self, command: ControlCommand) -> Result<()> {
        match command {
            | ControlCommand::Open {
                url,
                mobile,
            } => {
                self.viewport(mobile).await?;
                let response = self.call("Page.navigate", json!({"url":url})).await?;
                if let Some(error) = response.get("errorText") {
                    bail!("Navigation failed: {error}");
                }
                loop {
                    let state = self
                        .call("Runtime.evaluate", json!({"expression":"document.readyState","returnByValue":true}))
                        .await?;
                    if matches!(state["result"]["value"].as_str(), Some("complete" | "interactive")) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
            },
            | ControlCommand::Click {
                x,
                y,
            } => {
                ensure!(x < self.width && y < self.height, "Click is outside the browser viewport");
                for kind in ["mousePressed", "mouseReleased"] {
                    self.call("Input.dispatchMouseEvent", json!({"type":kind,"x":x,"y":y,"button":"left","clickCount":1}))
                        .await?;
                }
            },
            | ControlCommand::Text {
                text,
            } => {
                self.call("Input.insertText", json!({"text":text})).await?;
            },
            | ControlCommand::Key {
                key,
            } => {
                let code = windows::key_code(&key)?;
                for kind in ["keyDown", "keyUp"] {
                    let mut params = json!({"type":kind,"key":key,"windowsVirtualKeyCode":code});
                    if kind == "keyDown" && key == "Enter" {
                        params["text"] = json!("\r");
                    }
                    self.call("Input.dispatchKeyEvent", params).await?;
                }
            },
            | ControlCommand::Scroll {
                delta,
            } => {
                self.call(
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseWheel","x":self.width/2,"y":self.height/2,"deltaX":0,"deltaY":delta}),
                )
                .await?;
            },
            | ControlCommand::Reload {} => {
                self.call("Page.reload", json!({})).await?;
            },
            | ControlCommand::Capture {} => {},
            | ControlCommand::Stop {} => unreachable!(),
        }
        Ok(())
    }

    async fn frame(&mut self) -> Result<ControlFrame> {
        let tree = self.call("Page.getFrameTree", json!({})).await?;
        let response = self.call("Page.captureScreenshot", json!({"format":"jpeg","quality":80})).await?;
        let image = Attachment::Image {
            name: "browser-capture.jpg".into(),
            mime_type: "image/jpeg".into(),
            data: response["data"].as_str().context("Browser omitted screenshot")?.into(),
        };
        image.validate().map_err(anyhow::Error::msg)?;
        Ok(ControlFrame {
            width: self.width,
            height: self.height,
            url: tree["frameTree"]["frame"]["url"].as_str().map(str::to_owned),
            image,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn desktop_grant_and_browser_boundary() {
        let controls = Controls::default();
        assert!(controls.desktop(ControlCommand::Capture {}).await.is_err());
        for url in ["file:///etc/passwd", "javascript:alert(1)", "https://user:pass@example.com"] {
            assert!(validate_url(url).is_err());
        }
        assert!(validate_url("http://127.0.0.1:3000/app").is_ok());
        assert!(
            ControlCommand::Scroll {
                delta: i32::MIN
            }
            .validate()
            .is_err()
        );
        assert!(
            ControlCommand::Key {
                key: "F24".into()
            }
            .validate()
            .is_err()
        );
        controls.enable_desktop(false).await.unwrap();
        controls.browser(ControlCommand::Stop {}).await.unwrap();
        assert!(!controls.status().await.unwrap().browser_running);
    }

    #[tokio::test]
    #[ignore = "requires a locally installed Chromium browser"]
    async fn live_browser_controls_capture_and_cancel() -> Result<()> {
        use tokio::io::{AsyncReadExt,
                        AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (requests, mut seen) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let requests = requests.clone();
                tokio::spawn(async move {
                    let mut buffer = [0; 8192];
                    let count = socket.read(&mut buffer).await.unwrap_or(0);
                    let line = String::from_utf8_lossy(&buffer[.. count]).lines().next().unwrap_or("").to_owned();
                    let _ = requests.send(line.clone());
                    if line.contains("/hang") {
                        std::future::pending::<()>().await;
                    }
                    let body = "<!doctype html><body style='margin:0;background:#eef'><form><input name='marker' style='width:300px;height:50px'><button>Submit</button></form><div style='height:3000px'>Browser QA</div></body>";
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        let controls = Arc::new(Controls::default());
        let frame = controls
            .browser(ControlCommand::Open {
                url: format!("http://{address}/"),
                mobile: false,
            })
            .await?
            .unwrap();
        assert_eq!((frame.width, frame.height), (1024, 768));
        frame.image.validate().map_err(anyhow::Error::msg)?;
        controls
            .browser(ControlCommand::Click {
                x: 40,
                y: 25,
            })
            .await?;
        controls
            .browser(ControlCommand::Text {
                text: "t3_capture_qa".into(),
            })
            .await?;
        controls
            .browser(ControlCommand::Key {
                key: "Enter".into(),
            })
            .await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(line) = seen.recv().await {
                if line.contains("marker=t3_capture_qa") {
                    return;
                }
            }
            panic!("form did not receive the typed value");
        })
        .await?;
        controls
            .browser(ControlCommand::Scroll {
                delta: 480,
            })
            .await?;
        controls.browser(ControlCommand::Reload {}).await?;
        let frame = controls
            .browser(ControlCommand::Open {
                url: format!("http://{address}/"),
                mobile: true,
            })
            .await?
            .unwrap();
        assert_eq!((frame.width, frame.height), (390, 844));
        let running = controls.clone();
        let task = tokio::spawn(async move {
            running
                .browser(ControlCommand::Open {
                    url: format!("http://{address}/hang"),
                    mobile: false,
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        tokio::time::timeout(Duration::from_secs(3), controls.browser(ControlCommand::Stop {})).await??;
        assert!(task.await?.is_err());
        assert!(!controls.status().await?.browser_running);
        server.abort();
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "creates an owned test window and briefly moves the Windows pointer"]
    async fn live_windows_capture_and_input() -> Result<()> {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let (tx, rx) = std::sync::mpsc::channel();
        let window = std::thread::spawn(move || unsafe {
            let class: Vec<u16> = "EDIT\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE | 0x0004,
                100,
                100,
                600,
                300,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            tx.send(hwnd as isize).unwrap();
            if hwnd.is_null() {
                return;
            }
            let mut message = MSG::default();
            while IsWindow(hwnd) != 0 && GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        });
        let hwnd = rx.recv()?;
        ensure!(hwnd != 0, "Cannot create Windows QA window");
        struct OwnedWindow(isize);
        impl Drop for OwnedWindow {
            fn drop(&mut self) {
                unsafe {
                    PostMessageW(self.0 as _, WM_CLOSE, 0, 0);
                    PostThreadMessageW(GetWindowThreadProcessId(self.0 as _, std::ptr::null_mut()), WM_QUIT, 0, 0);
                }
            }
        }
        let owned = OwnedWindow(hwnd);
        if std::env::var_os("GPUI_AGENT_QA_EDITOR").is_some() {
            println!("Owned Windows EDIT ready at (100,100), 600x300. No input is injected by this test.");
            let expected = if std::env::var_os("GPUI_AGENT_QA_EDITOR_EMPTY").is_some() {
                ""
            } else {
                "안녕하세요. Codex 컴퓨터 사용 검증입니다."
            };
            println!("After verification, press Enter to assert the expected editor value and close it.");
            tokio::task::spawn_blocking(|| std::io::stdin().read_line(&mut String::new())).await??;
            let mut text = vec![0u16; 4096];
            let count = unsafe { GetWindowTextW(hwnd as _, text.as_mut_ptr(), text.len() as i32) };
            let matches = String::from_utf16_lossy(&text[.. count as usize]) == expected;
            drop(owned);
            window.join().unwrap();
            ensure!(matches, "Owned editor did not contain the exact expected value");
            println!("ok: owned editor value matches the expected value exactly (empty: {})", expected.is_empty());
            return Ok(());
        }
        let controls = Controls::default();
        controls.enable_desktop(true).await?;
        let frame = controls.desktop(ControlCommand::Capture {}).await?.unwrap();
        frame.image.validate().map_err(anyhow::Error::msg)?;
        use windows_sys::Win32::UI::HiDpi::*;
        let (x, y) = unsafe {
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            GetWindowRect(hwnd as _, &mut rect);
            let pos = (
                (rect.left + 70 - GetSystemMetrics(SM_XVIRTUALSCREEN)) as u32,
                (rect.top + 80 - GetSystemMetrics(SM_YVIRTUALSCREEN)) as u32,
            );
            SetThreadDpiAwarenessContext(previous);
            pos
        };
        controls
            .desktop(ControlCommand::Click {
                x,
                y,
            })
            .await?;
        ensure!(
            controls.desktop_target.load(Ordering::SeqCst) == hwnd,
            "QA click did not select our own window; input skipped"
        );
        controls
            .desktop(ControlCommand::Text {
                text: "KND Windows 한글 QA".into(),
            })
            .await?;
        controls
            .desktop(ControlCommand::Key {
                key: "End".into(),
            })
            .await?;
        controls
            .desktop(ControlCommand::Scroll {
                delta: 120,
            })
            .await?;
        let mut text = vec![0u16; 256];
        let count = unsafe { GetWindowTextW(hwnd as _, text.as_mut_ptr(), text.len() as i32) };
        assert_eq!(String::from_utf16_lossy(&text[.. count as usize]), "KND Windows 한글 QA");
        controls.desktop_until.store(now() - 1, Ordering::SeqCst);
        assert!(controls.desktop(ControlCommand::Capture {}).await.is_err());
        controls.enable_desktop(false).await?;
        assert!(
            controls
                .desktop(ControlCommand::Text {
                    text: "must not type".into()
                })
                .await
                .is_err()
        );
        // Given an owned interactive test window and a job grant, when its
        // execution observes, clicks and types, then a fresh capture follows
        // the actual Unicode input. No personal application is targeted.
        use gpui_agent_contracts::{ComputerCommand,
                                   ComputerGrantRequest};
        let session = gpui_agent_ports::ComputerSession {
            job_id: 1,
            execution_id: 1,
            active: Arc::new(|| true),
        };
        controls
            .grant_computer(
                ComputerGrantRequest {
                    job_id: 1,
                    enabled: true,
                },
                Arc::new(|| true),
            )
            .await?;
        let capture = controls.computer(session.clone(), ComputerCommand::Capture {}).await?.unwrap();
        let clicked = controls
            .computer(
                session.clone(),
                ComputerCommand::Act {
                    revision: capture.revision.clone(),
                    action: ControlCommand::Click {
                        x,
                        y,
                    },
                },
            )
            .await?
            .unwrap();
        ensure!(
            controls.desktop_target.load(Ordering::SeqCst) == hwnd,
            "Computer click did not target our QA window; typing skipped"
        );
        let positioned = controls
            .computer(
                session.clone(),
                ComputerCommand::Act {
                    revision: clicked.revision.clone(),
                    action: ControlCommand::Key {
                        key: "End".into(),
                    },
                },
            )
            .await?
            .unwrap();
        let entered = controls
            .computer(
                session.clone(),
                ComputerCommand::Act {
                    revision: positioned.revision.clone(),
                    action: ControlCommand::Text {
                        text: " Computer MVP 한글".into(),
                    },
                },
            )
            .await?
            .unwrap();
        ensure!(capture.revision != clicked.revision && clicked.revision != entered.revision);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count = unsafe { GetWindowTextW(hwnd as _, text.as_mut_ptr(), text.len() as i32) };
                if String::from_utf16_lossy(&text[.. count as usize]) == "KND Windows 한글 QA Computer MVP 한글" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;
        let observed = controls.computer(session.clone(), ComputerCommand::Capture {}).await?.unwrap();
        observed.frame.image.validate().map_err(anyhow::Error::msg)?;
        if let Some(directory) = std::env::var_os("GPUI_AGENT_QA_DIR") {
            let directory = PathBuf::from(directory);
            ensure!(directory.is_absolute(), "QA evidence directory must be absolute");
            std::fs::create_dir_all(&directory)?;
            std::fs::write(
                directory.join("windows-computer-mvp.jpg"),
                observed.frame.image.image_bytes().map_err(anyhow::Error::msg)?,
            )?;
        }
        controls
            .grant_computer(
                ComputerGrantRequest {
                    job_id: 1,
                    enabled: false,
                },
                Arc::new(|| true),
            )
            .await?;
        assert!(controls.computer(session, ComputerCommand::Capture {}).await.is_err());
        drop(owned);
        window.join().unwrap();
        Ok(())
    }
}
