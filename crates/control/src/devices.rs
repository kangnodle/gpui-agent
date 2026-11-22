use super::*;
use gpui_agent_contracts::{DeviceAction,
                           DeviceCommand,
                           DeviceGrantRequest,
                           DeviceInfo,
                           DeviceList,
                           DeviceReply,
                           McpScope,
                           attachment::MAX_IMAGE_BYTES,
                           device_png_dimensions};
use std::{process::Stdio,
          sync::{Mutex as StateMutex,
                 atomic::AtomicBool}};
use tokio::io::AsyncReadExt;

struct Frame {
    revision: String,
    width: u32,
    height: u32,
}
struct Grant {
    owner: Option<McpScope>,
    device_id: String,
    until: u64,
    expires: tokio::time::Instant,
    generation: u64,
    opened: bool,
    frame: Option<Frame>,
    cancel: watch::Sender<u64>,
}
pub(super) struct Devices {
    executable: Option<PathBuf>,
    grants: StateMutex<Vec<Grant>>,
    io: Mutex<()>,
    next: AtomicU64,
    stopped: AtomicBool,
    cancel: watch::Sender<u64>,
}

pub(super) fn discover() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("GPUI_AGENT_ADB_EXECUTABLE") {
        let path = PathBuf::from(path);
        return (path.is_absolute() && path.is_file()).then_some(path);
    }
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut candidates = Vec::new();
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(root).join("platform-tools").join(exe));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|root| root.join(exe)));
    }
    candidates.into_iter().find(|path| path.is_absolute() && path.is_file())
}

impl Devices {
    pub(super) fn new(executable: Option<PathBuf>) -> Self {
        Self {
            executable,
            grants: StateMutex::new(Vec::new()),
            io: Mutex::new(()),
            next: AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
            ),
            stopped: AtomicBool::new(false),
            cancel: watch::channel(0).0,
        }
    }

    fn grants(&self) -> Result<std::sync::MutexGuard<'_, Vec<Grant>>> {
        let mut grants = self.grants.lock().map_err(|_| anyhow::anyhow!("Device registry unavailable"))?;
        grants.retain(|grant| {
            if grant.expires > tokio::time::Instant::now() {
                true
            } else {
                grant.cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
                false
            }
        });
        Ok(grants)
    }

    fn permission<T>(&self, scope: &Option<McpScope>, id: &str, opened: bool, work: impl FnOnce(&mut Grant) -> Result<T>) -> Result<T> {
        ensure!(!self.stopped.load(Ordering::SeqCst), "Device control is stopping");
        let mut grants = self.grants()?;
        let grant = grants
            .iter_mut()
            .find(|grant| &grant.owner == scope && grant.device_id == id)
            .context("Operator device grant is missing or expired")?;
        ensure!(!opened || grant.opened, "Open the granted device first");
        work(grant)
    }

    pub(super) async fn grant(&self, request: DeviceGrantRequest) -> Result<()> {
        request.validate().map_err(anyhow::Error::msg)?;
        if !request.enabled {
            self.grants()?.retain(|grant| {
                if grant.device_id != request.device_id {
                    true
                } else {
                    grant.cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
                    false
                }
            });
            return Ok(());
        }
        let _io = self.io.lock().await;
        ensure!(!self.stopped.load(Ordering::SeqCst), "Device control is stopping");
        let mut cancel = self.cancel.subscribe();
        let ready = tokio::select! {
            result=tokio::time::timeout(Duration::from_secs(15),self.list())=>result.context("ADB discovery timed out")??,
            _=cancel.changed()=>bail!("Device control stopped"),
        };
        ensure!(
            ready.iter().any(|device| device.id == request.device_id && device.state == "device"),
            "Device must be connected and authorized by Android"
        );
        let mut grants = self.grants()?;
        ensure!(!self.stopped.load(Ordering::SeqCst), "Device control is stopping");
        ensure!(
            !grants.iter().any(|g| g.device_id == request.device_id && g.owner != request.scope),
            "Device is reserved by another scope; revoke its grant first"
        );
        ensure!(
            !grants.iter().any(|g| g.owner == request.scope && g.device_id != request.device_id),
            "Close the scope's previous device first"
        );
        grants.retain(|grant| {
            if grant.device_id != request.device_id {
                true
            } else {
                grant.cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
                false
            }
        });
        ensure!(grants.len() < 8, "At most 8 device grants are allowed");
        grants.push(Grant {
            owner: request.scope,
            device_id: request.device_id,
            until: now() + 600,
            expires: tokio::time::Instant::now() + Duration::from_secs(600),
            generation: self.next.fetch_add(1, Ordering::SeqCst),
            opened: false,
            frame: None,
            cancel: watch::channel(0).0,
        });
        Ok(())
    }

    pub(super) async fn execute(&self, scope: Option<McpScope>, command: DeviceCommand) -> Result<DeviceReply> {
        command.validate().map_err(anyhow::Error::msg)?;
        ensure!(!self.stopped.load(Ordering::SeqCst), "Device control is stopping");
        if let DeviceCommand::Close {
            device_id,
        } = &command
        {
            self.grants()?.retain(|grant| {
                if grant.owner != scope || grant.device_id != *device_id {
                    true
                } else {
                    grant.cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
                    false
                }
            });
            return Ok(DeviceReply::Closed {});
        }
        let _io = self.io.lock().await;
        ensure!(!self.stopped.load(Ordering::SeqCst), "Device control is stopping");
        let mut stopped = self.cancel.subscribe();
        if matches!(command, DeviceCommand::List {}) {
            return tokio::select! {
                result=tokio::time::timeout(Duration::from_secs(15),self.discovery(&scope))=>result.context("ADB discovery timed out")?,
                _=stopped.changed()=>bail!("Device control stopped"),
            };
        }
        let id = match &command {
            | DeviceCommand::Open {
                device_id,
            }
            | DeviceCommand::Screenshot {
                device_id,
            }
            | DeviceCommand::Act {
                device_id, ..
            } => device_id,
            | _ => unreachable!(),
        };
        let (expires, generation, mut cancel) = self.permission(&scope, id, !matches!(command, DeviceCommand::Open { .. }), |grant| {
            Ok((grant.expires, grant.generation, grant.cancel.subscribe()))
        })?;
        let deadline = expires.saturating_duration_since(tokio::time::Instant::now()).min(Duration::from_secs(15));
        let reply = tokio::select! {
            result=tokio::time::timeout(deadline,self.perform(&scope,&command,generation))=>result.context("Device command timed out; capture again before retrying")?,
            _=cancel.changed()=>bail!("Device permission revoked or session closed"),
            _=stopped.changed()=>bail!("Device control stopped"),
        }?;
        self.permission(&scope, id, false, |grant| {
            ensure!(grant.generation == generation, "Device permission changed");
            Ok(())
        })?;
        Ok(reply)
    }

    async fn discovery(&self, scope: &Option<McpScope>) -> Result<DeviceReply> {
        let mut devices = if self.executable.is_some() { self.list().await? } else { vec![] };
        let grants = self.grants()?;
        let grant = grants.iter().find(|grant| &grant.owner == scope);
        if scope.is_some() {
            devices.retain(|device| grant.is_some_and(|grant| grant.device_id == device.id));
        }
        Ok(DeviceReply::List(DeviceList {
            available: self.executable.is_some(),
            devices,
            open: grant.filter(|g| g.opened).map(|g| g.device_id.clone()),
            enabled_until: grant.map_or(0, |g| g.until),
        }))
    }

    async fn perform(&self, scope: &Option<McpScope>, command: &DeviceCommand, generation: u64) -> Result<DeviceReply> {
        match command {
            | DeviceCommand::Open {
                device_id,
            } => {
                let device = self
                    .list()
                    .await?
                    .into_iter()
                    .find(|d| d.id == *device_id && d.state == "device")
                    .context("Device is disconnected, offline or unauthorized")?;
                let enabled_until = self.permission(scope, device_id, false, |grant| {
                    ensure!(grant.generation == generation, "Device permission changed");
                    grant.opened = true;
                    grant.frame = None;
                    Ok(grant.until)
                })?;
                Ok(DeviceReply::Opened {
                    device,
                    enabled_until,
                })
            },
            | DeviceCommand::Screenshot {
                device_id,
            } => {
                self.permission(scope, device_id, true, |grant| {
                    ensure!(grant.generation == generation, "Device permission changed");
                    grant.frame = None;
                    Ok(())
                })?;
                let bytes = self.run(&["-s", device_id, "exec-out", "screencap", "-p"], MAX_IMAGE_BYTES).await?;
                let (width, height) = device_png_dimensions(&bytes).map_err(anyhow::Error::msg)?;
                let image = Attachment::from_bytes("device-screen.png".into(), &bytes).map_err(anyhow::Error::msg)?;
                let revision = format!("device:{}", self.next.fetch_add(1, Ordering::SeqCst));
                self.permission(scope, device_id, true, |grant| {
                    ensure!(grant.generation == generation, "Device permission changed");
                    grant.frame = Some(Frame {
                        revision: revision.clone(),
                        width,
                        height,
                    });
                    Ok(())
                })?;
                Ok(DeviceReply::Screenshot {
                    device_id: device_id.clone(),
                    revision,
                    width,
                    height,
                    image,
                })
            },
            | DeviceCommand::Act {
                device_id,
                revision,
                action,
            } => {
                self.permission(scope, device_id, true, |grant| {
                    ensure!(grant.generation == generation, "Device permission changed");
                    let frame = grant.frame.as_ref().context("Capture the device screen before input")?;
                    ensure!(frame.revision == *revision, "Stale device screenshot revision; capture again");
                    action.validate_frame(frame.width, frame.height).map_err(anyhow::Error::msg)?;
                    grant.frame = None;
                    Ok(())
                })?;
                let arguments = action_arguments(action);
                let mut args = vec!["-s", device_id, "shell", "input"];
                args.extend(arguments.iter().map(String::as_str));
                self.run(&args, 8192).await?;
                Ok(DeviceReply::Acted {
                    device_id: device_id.clone(),
                })
            },
            | _ => unreachable!(),
        }
    }

    async fn list(&self) -> Result<Vec<DeviceInfo>> {
        let bytes = self.run(&["devices", "-l"], 65536).await?;
        let text = std::str::from_utf8(&bytes).context("Invalid ADB discovery output")?;
        let mut devices = vec![];
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let Some(id) = fields.next() else {
                continue;
            };
            if line.starts_with("List of devices attached") || id.starts_with('*') {
                continue;
            }
            let Some(state) = fields.next() else {
                continue;
            };
            gpui_agent_contracts::validate_device_id(id).map_err(anyhow::Error::msg)?;
            ensure!(
                matches!(state, "device" | "offline" | "unauthorized" | "recovery" | "sideload" | "bootloader" | "no"),
                "Unsupported ADB device state"
            );
            let name = fields.find_map(|field| field.strip_prefix("model:")).unwrap_or(id).chars().take(100).collect();
            ensure!(devices.len() < 64, "Device discovery exceeds 64 devices");
            devices.push(DeviceInfo {
                id: id.into(),
                name,
                state: state.into(),
            });
        }
        Ok(devices)
    }

    async fn run(&self, args: &[&str], limit: usize) -> Result<Vec<u8>> {
        let executable = self
            .executable
            .as_deref()
            .context("Install Android SDK Platform Tools and start its local ADB server, or set GPUI_AGENT_ADB_EXECUTABLE")?;
        let mut command = tokio::process::Command::new(executable);
        command
            .args(["-H", "127.0.0.1"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut process = gpui_agent_process::spawn(&mut command).context("Cannot start ADB client")?;
        let stdout = process.stdout.take().context("ADB stdout unavailable")?;
        let stderr = process.stderr.take().context("ADB stderr unavailable")?;
        let (output, _, status) = tokio::try_join!(bounded(stdout, limit), bounded(stderr, 8192), process.wait())?;
        ensure!(status.success(), "ADB command failed; check device connection and Android authorization");
        Ok(output)
    }

    pub(super) fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        let mut grants = self.grants.lock().unwrap_or_else(|error| error.into_inner());
        for grant in grants.drain(..) {
            grant.cancel.send_modify(|epoch| *epoch = epoch.wrapping_add(1));
        }
    }
}

async fn bounded(stream: impl tokio::io::AsyncRead + Unpin, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = vec![];
    stream.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        return Err(std::io::Error::other("ADB output exceeds limit"));
    }
    Ok(bytes)
}
fn action_arguments(action: &DeviceAction) -> Vec<String> {
    match action {
        | DeviceAction::Tap {
            x,
            y,
        } => vec!["tap".into(), x.to_string(), y.to_string()],
        | DeviceAction::Swipe {
            x,
            y,
            to_x,
            to_y,
            duration_ms,
        } => vec![
            "swipe".into(),
            x.to_string(),
            y.to_string(),
            to_x.to_string(),
            to_y.to_string(),
            duration_ms.to_string(),
        ],
        | DeviceAction::Text {
            text,
        } => vec!["text".into(), format!("'{}'", text.replace(' ', "%s").replace('\'', "'\\''"))],
        | DeviceAction::Key {
            key,
        } => vec![
            "keyevent".into(),
            match key.as_str() {
                | "Home" => "3",
                | "Back" => "4",
                | "Enter" => "66",
                | "Tab" => "61",
                | "Backspace" => "67",
                | "ArrowUp" => "19",
                | "ArrowDown" => "20",
                | "ArrowLeft" => "21",
                | "ArrowRight" => "22",
                | _ => unreachable!(),
            }
            .into(),
        ],
    }
}
