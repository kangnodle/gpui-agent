use super::*;
use gpui_agent_contracts::{ComputerCommand,
                           ComputerFrame,
                           ComputerGrantRequest,
                           ComputerLease,
                           ComputerStatus};
use gpui_agent_ports::{ComputerSession,
                       ControlValidity};

pub(super) struct Grant {
    lease: ComputerLease,
    expires: Option<tokio::time::Instant>,
    active: ControlValidity,
    owner: Option<ControlValidity>,
}

impl Grant {
    fn valid(&self) -> bool {
        self.expires.is_none_or(|expires| expires > tokio::time::Instant::now()) && (self.active)() && self.owner.as_ref().is_none_or(|owner| owner())
    }
}

#[derive(Default)]
pub(super) struct DesktopState {
    pub(super) computer_target: bool,
    grant: Option<Grant>,
    sequence: u64,
}
impl DesktopState {
    pub(super) fn granted(&mut self) -> bool {
        if self.grant.as_ref().is_some_and(|grant| !grant.valid()) {
            self.grant = None;
        }
        self.grant.is_some()
    }

    pub(super) fn clear(&mut self) { self.grant = None; }
}

impl Controls {
    pub(super) async fn computer_state(&self) -> Result<ComputerStatus> {
        let lock = self.desktop_lock.clone();
        let manual = self.desktop_until.clone();
        let automatic = self.automatic_desktop.clone();
        let available = cfg!(windows) || self.desktop_driver.is_some();
        tokio::task::spawn_blocking(move || {
            let mut state = lock.lock().map_err(|_| anyhow::anyhow!("Desktop lock poisoned"))?;
            state.granted();
            Ok(ComputerStatus {
                automatic: automatic.load(Ordering::SeqCst),
                available,
                foreground: super::windows::foreground_identity(),
                manual_enabled: automatic.load(Ordering::SeqCst) || manual.load(Ordering::SeqCst) > now(),
                job_id: state.grant.as_ref().map(|grant| grant.lease.job_id),
                execution_bound: state.grant.as_ref().is_some_and(|grant| grant.owner.is_some()),
                expires_in_seconds: state.grant.as_ref().map_or(0, |grant| {
                    grant.expires.map_or(0, |expires| {
                        expires.saturating_duration_since(tokio::time::Instant::now()).as_secs_f64().ceil() as u64
                    })
                }),
            })
        })
        .await?
    }

    pub(super) async fn computer_grant(&self, request: ComputerGrantRequest, active: ControlValidity) -> Result<()> {
        request.validate().map_err(anyhow::Error::msg)?;
        if !request.enabled {
            let revoked = self
                .computer_job
                .compare_exchange(request.job_id, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
            #[cfg(windows)]
            if revoked {
                self.indicator_generation.fetch_add(1, Ordering::SeqCst);
            }
            #[cfg(not(windows))]
            let _ = revoked;
            return Ok(());
        }
        let mut state = self.desktop_lock.lock().map_err(|_| anyhow::anyhow!("Desktop lock poisoned"))?;
        ensure!(cfg!(windows) || self.desktop_driver.is_some(), "Computer use requires Windows");
        ensure!(!state.granted(), "Another computer grant holds the desktop; revoke it first");
        ensure!(
            self.automatic_desktop.load(Ordering::SeqCst) || self.desktop_until.load(Ordering::SeqCst) <= now(),
            "Disable manual desktop control first"
        );
        ensure!(active(), "Computer job is no longer active");
        self.desktop_target.store(0, Ordering::SeqCst);
        let current = self.computer_job.clone();
        let job_id = request.job_id;
        current.store(job_id, Ordering::SeqCst);
        let active = Arc::new(move || current.load(Ordering::SeqCst) == job_id && active());
        state.grant = Some(Grant {
            lease: ComputerLease::new(request.job_id),
            expires: Some(tokio::time::Instant::now() + Duration::from_secs(600)),
            active,
            owner: None,
        });
        Ok(())
    }

    pub(super) async fn computer_command(&self, session: ComputerSession, command: ComputerCommand) -> Result<Option<ComputerFrame>> {
        command.validate().map_err(anyhow::Error::msg)?;
        struct Request(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Request {
            fn drop(&mut self) { self.0.store(false, Ordering::SeqCst); }
        }
        let request = Request(Arc::new(std::sync::atomic::AtomicBool::new(true)));
        let pending = request.0.clone();
        let driver = self.desktop_driver.clone();
        let lock = self.desktop_lock.clone();
        let target = self.desktop_target.clone();
        let automatic = self.automatic_desktop.clone();
        let computer_job = self.computer_job.clone();
        #[cfg(windows)]
        let indicator_generation = self.indicator_generation.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = lock.lock().map_err(|_| anyhow::anyhow!("Desktop lock poisoned"))?;
            if !state.granted() && automatic.load(Ordering::SeqCst) && matches!(command, ComputerCommand::Capture {}) {
                ensure!((session.active)(), "Computer execution expired");
                computer_job.store(session.job_id, Ordering::SeqCst);
                target.store(0, Ordering::SeqCst);
                let job_id = session.job_id;
                let session_active = session.active.clone();
                let current = computer_job.clone();
                let enabled = automatic.clone();
                state.grant = Some(Grant {
                    lease: ComputerLease::new(job_id),
                    expires: None,
                    active: Arc::new(move || enabled.load(Ordering::SeqCst) && current.load(Ordering::SeqCst) == job_id && session_active()),
                    owner: None,
                });
            }
            ensure!(state.granted(), "An operator computer grant is required");
            state.sequence += 1;
            state.computer_target = true;
            let revision = state.sequence.to_string();
            let grant = state.grant.as_mut().context("Computer grant expired")?;
            grant.lease.authorize(session.job_id, session.execution_id).map_err(anyhow::Error::msg)?;
            ensure!((session.active)(), "Computer execution expired");
            if matches!(command, ComputerCommand::Close {}) {
                #[cfg(windows)]
                indicator_generation.fetch_add(1, Ordering::SeqCst);
                state.clear();
                target.store(0, Ordering::SeqCst);
                return Ok(None);
            }
            let action = match command {
                | ComputerCommand::Act {
                    revision,
                    action,
                } => {
                    grant.lease.consume(&revision, &action).map_err(anyhow::Error::msg)?;
                    action
                },
                | _ => ControlCommand::Capture {},
            };
            let execution = session.execution_id;
            #[cfg(windows)]
            let click = match &action {
                | ControlCommand::Click {
                    x,
                    y,
                } => Some((*x, *y)),
                | _ => None,
            };
            grant.lease.begin(execution);
            grant.owner = Some(session.active);
            let allowed = || pending.load(Ordering::SeqCst) && grant.valid();
            ensure!(allowed(), "Computer command cancelled before native input");
            #[cfg(windows)]
            let indicator = if let Some((x, y)) = click {
                let expiry = grant.expires;
                let active = grant.active.clone();
                let owner = grant.owner.clone();
                let valid = Arc::new(move || {
                    expiry.is_none_or(|deadline| deadline > tokio::time::Instant::now()) && active() && owner.as_ref().is_none_or(|owner| owner())
                });
                Some(super::overlay::prepare(x, y, indicator_generation, valid)?)
            } else {
                None
            };
            let frame = match driver {
                | Some(driver) => driver.execute(action, &target, &allowed),
                | None => windows::execute(action, &target, &allowed),
            }?
            .context("No Windows capture returned")?;
            ensure!(grant.valid(), "Computer grant expired during capture");
            #[cfg(windows)]
            if let Some(indicator) = indicator {
                indicator.activate();
            }
            grant.lease.observe(revision.clone(), frame.width, frame.height);
            Ok(Some(ComputerFrame {
                revision,
                frame,
            }))
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex as StdMutex,
                    atomic::AtomicBool};

    struct Driver {
        entered: StdMutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: StdMutex<std::sync::mpsc::Receiver<()>>,
        finished: tokio::sync::Notify,
        inputs: AtomicU64,
    }
    impl DesktopDriver for Driver {
        fn execute(&self, command: ControlCommand, _: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
            if matches!(command, ControlCommand::Text { .. }) {
                if let Some(entered) = self.entered.lock().unwrap().take() {
                    let _ = entered.send(());
                }
                self.release.lock().unwrap().recv_timeout(Duration::from_secs(5))?;
                if allowed() {
                    self.inputs.fetch_add(1, Ordering::SeqCst);
                }
                self.finished.notify_one();
            }
            ensure!(allowed(), "Execution cancelled before native input");
            Ok(Some(ControlFrame {
                width: 100,
                height: 100,
                url: None,
                image: Attachment::Text {
                    name: "fixture".into(),
                    text: String::new(),
                    source: None,
                },
            }))
        }
    }
    fn session(active: Arc<AtomicBool>) -> ComputerSession {
        ComputerSession {
            job_id: 1,
            execution_id: 1,
            active: Arc::new(move || active.load(Ordering::SeqCst)),
        }
    }

    #[tokio::test]
    async fn given_a_dropped_command_when_native_input_resumes_then_no_input_occurs() -> Result<()> {
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let driver = Arc::new(Driver {
            entered: StdMutex::new(Some(entered)),
            release: StdMutex::new(resume),
            finished: tokio::sync::Notify::new(),
            inputs: AtomicU64::new(0),
        });
        let controls = Arc::new(Controls::default().with_desktop_driver(driver.clone()));
        let active = Arc::new(AtomicBool::new(true));
        controls
            .grant_computer(
                ComputerGrantRequest {
                    job_id: 1,
                    enabled: true,
                },
                Arc::new(|| true),
            )
            .await?;
        let capture = controls.computer(session(active.clone()), ComputerCommand::Capture {}).await?.unwrap();
        let worker = controls.clone();
        let request = tokio::spawn(async move {
            worker
                .computer(
                    session(active),
                    ComputerCommand::Act {
                        revision: capture.revision,
                        action: ControlCommand::Text {
                            text: "must not type".into(),
                        },
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), ready).await??;
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        release.send(())?;
        tokio::time::timeout(Duration::from_secs(5), driver.finished.notified()).await?;
        assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn given_input_waiting_at_the_os_when_the_operator_revokes_then_it_is_denied_before_input() -> Result<()> {
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let driver = Arc::new(Driver {
            entered: StdMutex::new(Some(entered)),
            release: StdMutex::new(resume),
            finished: tokio::sync::Notify::new(),
            inputs: AtomicU64::new(0),
        });
        let controls = Arc::new(Controls::default().with_desktop_driver(driver.clone()));
        let active = Arc::new(AtomicBool::new(true));
        controls
            .grant_computer(
                ComputerGrantRequest {
                    job_id: 1,
                    enabled: true,
                },
                Arc::new(|| true),
            )
            .await?;
        let capture = controls.computer(session(active.clone()), ComputerCommand::Capture {}).await?.unwrap();
        let worker = controls.clone();
        let request = tokio::spawn(async move {
            worker
                .computer(
                    session(active),
                    ComputerCommand::Act {
                        revision: capture.revision,
                        action: ControlCommand::Text {
                            text: "revoked".into(),
                        },
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), ready).await??;
        let revoke_started = std::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(1),
            controls.grant_computer(
                ComputerGrantRequest {
                    job_id: 1,
                    enabled: false,
                },
                Arc::new(|| true),
            ),
        )
        .await??;
        assert!(revoke_started.elapsed() < Duration::from_secs(1), "Revocation must not wait for native input");
        release.send(())?;
        assert!(request.await?.is_err());
        assert_eq!(driver.inputs.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn given_a_grant_when_ten_minutes_elapse_then_capture_is_denied_without_os_io() -> Result<()> {
        let (entered, _) = tokio::sync::oneshot::channel();
        let (_, resume) = std::sync::mpsc::channel();
        let driver = Arc::new(Driver {
            entered: StdMutex::new(Some(entered)),
            release: StdMutex::new(resume),
            finished: tokio::sync::Notify::new(),
            inputs: AtomicU64::new(0),
        });
        let controls = Controls::default().with_desktop_driver(driver);
        controls
            .grant_computer(
                ComputerGrantRequest {
                    job_id: 1,
                    enabled: true,
                },
                Arc::new(|| true),
            )
            .await?;
        tokio::time::advance(Duration::from_secs(600)).await;
        let expired = controls.computer_status().await?;
        assert!(expired.job_id.is_none() && !expired.execution_bound && expired.expires_in_seconds == 0);
        assert!(
            controls
                .computer(session(Arc::new(AtomicBool::new(true))), ComputerCommand::Capture {})
                .await
                .is_err(),
            "Grant expiry is inclusive"
        );
        Ok(())
    }
}
