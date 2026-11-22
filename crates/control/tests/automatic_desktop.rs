use anyhow::Result;
use gpui_agent_contracts::{Attachment,
                           ComputerCommand,
                           ComputerGrantRequest,
                           ControlCommand,
                           ControlFrame};
use gpui_agent_control::{Controls,
                         DesktopDriver};
use gpui_agent_ports::{ComputerSession,
                       HostControl};
use std::sync::{Arc,
                atomic::AtomicIsize};

#[cfg(windows)]
#[tokio::test]
async fn given_a_computer_click_when_the_execution_ends_then_the_indicator_disappears() {
    use std::sync::atomic::{AtomicBool,
                            Ordering};
    use windows_sys::Win32::{Graphics::Gdi::*,
                             UI::{HiDpi::*,
                                  WindowsAndMessaging::*}};
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    struct Screen;
    impl DesktopDriver for Screen {
        fn execute(&self, command: ControlCommand, target: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
            let mut frame = Desktop.execute(command, target, allowed)?.unwrap();
            frame.width = 32768;
            frame.height = 32768;
            Ok(Some(frame))
        }
    }
    let monitor = unsafe {
        MonitorFromPoint(
            windows_sys::Win32::Foundation::POINT {
                x: 0,
                y: 0,
            },
            MONITOR_DEFAULTTOPRIMARY,
        )
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    assert_ne!(unsafe { GetMonitorInfoW(monitor, &mut info) }, 0);
    let x = (info.rcMonitor.left - unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) } + 20) as u32;
    let y = (info.rcMonitor.top - unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) } + 20) as u32;
    let name: Vec<u16> = "gpui-agent computer use indicator\0".encode_utf16().collect();
    let visible = || unsafe {
        let hwnd = FindWindowW(std::ptr::null(), name.as_ptr());
        !hwnd.is_null() && IsWindowVisible(hwnd) != 0
    };
    let active = Arc::new(AtomicBool::new(true));
    let validity = active.clone();
    let controls = Controls::default().with_desktop_driver(Arc::new(Screen)).with_automatic_desktop();
    let session = || ComputerSession {
        job_id: 1,
        execution_id: 1,
        active: Arc::new({
            let validity = validity.clone();
            move || validity.load(Ordering::SeqCst)
        }),
    };
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await.unwrap().unwrap();
    assert!(!visible());
    controls
        .computer(
            session(),
            ComputerCommand::Act {
                revision: frame.revision,
                action: ControlCommand::Click {
                    x,
                    y,
                },
            },
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(visible(), "a successful computer click must show the indicator");
    let hwnd = unsafe { FindWindowW(std::ptr::null(), name.as_ptr()) };
    let mut affinity = 0;
    assert_ne!(unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }, 0);
    assert_eq!(affinity, WDA_EXCLUDEFROMCAPTURE);
    let style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    assert_eq!(
        style & (WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_LAYERED),
        WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_LAYERED
    );
    let mut bounds = windows_sys::Win32::Foundation::RECT::default();
    assert_ne!(unsafe { GetWindowRect(hwnd, &mut bounds) }, 0);
    assert_eq!(
        (bounds.left, bounds.top, bounds.right, bounds.bottom),
        (info.rcMonitor.left, info.rcMonitor.top, info.rcMonitor.right, info.rcMonitor.bottom)
    );
    let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    assert_ne!(unsafe { GetWindowRgn(hwnd, region) }, ERROR);
    let px = x as i32 + unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) } - bounds.left;
    let py = y as i32 + unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) } - bounds.top;
    let samples: Vec<_> = (0 .. 25).map(|dy| unsafe { PtInRegion(region, px + 2, py + dy) }).collect();
    assert_ne!(samples[16], 0, "indicator includes a pointer shape at {px},{py}: {samples:?}");
    assert_ne!(
        unsafe { PtInRegion(region, 8, 100) },
        0,
        "the border must remain visible at eight physical pixels"
    );
    assert_ne!(unsafe { PtInRegion(region, px + 2, py + 30) }, 0, "the pointer must be large enough to see");
    unsafe {
        DeleteObject(region);
    }
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut windows_sys::Win32::Foundation::RECT, data: isize) -> i32 {
        unsafe {
            (*(data as *mut Vec<HMONITOR>)).push(monitor);
        }
        1
    }
    let mut monitors = Vec::new();
    assert_ne!(
        unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(collect), &mut monitors as *mut _ as isize) },
        0
    );
    if let Some(other) = monitors.into_iter().find(|candidate| *candidate != monitor) {
        let mut other_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        assert_ne!(unsafe { GetMonitorInfoW(other, &mut other_info) }, 0);
        let other_x = (other_info.rcMonitor.left - unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) } + 20) as u32;
        let other_y = (other_info.rcMonitor.top - unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) } + 20) as u32;
        let frame = controls.computer(session(), ComputerCommand::Capture {}).await.unwrap().unwrap();
        controls
            .computer(
                session(),
                ComputerCommand::Act {
                    revision: frame.revision,
                    action: ControlCommand::Click {
                        x: other_x,
                        y: other_y,
                    },
                },
            )
            .await
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let window = unsafe { FindWindowW(std::ptr::null(), name.as_ptr()) };
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            if !window.is_null()
                && unsafe { IsWindowVisible(window) != 0 && GetWindowRect(window, &mut rect) != 0 }
                && rect.left == other_info.rcMonitor.left
                && rect.top == other_info.rcMonitor.top
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "indicator did not move to the second monitor");
            tokio::task::yield_now().await;
        }
    }
    active.store(false, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(!visible(), "the indicator must close without another command");
    active.store(true, Ordering::SeqCst);
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await.unwrap().unwrap();
    controls
        .computer(
            session(),
            ComputerCommand::Act {
                revision: frame.revision,
                action: ControlCommand::Click {
                    x,
                    y,
                },
            },
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(visible());
    controls
        .grant_computer(
            ComputerGrantRequest {
                job_id: 1,
                enabled: false,
            },
            Arc::new(|| true),
        )
        .await
        .unwrap();
    controls
        .grant_computer(
            ComputerGrantRequest {
                job_id: 1,
                enabled: true,
            },
            Arc::new(|| true),
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(!visible(), "regranting must not revive a revoked indicator");
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await.unwrap().unwrap();
    controls
        .computer(
            session(),
            ComputerCommand::Act {
                revision: frame.revision,
                action: ControlCommand::Click {
                    x,
                    y,
                },
            },
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(visible());
    controls.enable_desktop(false).await.unwrap();
    controls
        .grant_computer(
            ComputerGrantRequest {
                job_id: 1,
                enabled: true,
            },
            Arc::new(|| true),
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(!visible(), "disabling desktop control must invalidate the previous indicator");
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await.unwrap().unwrap();
    controls
        .computer(
            session(),
            ComputerCommand::Act {
                revision: frame.revision,
                action: ControlCommand::Click {
                    x,
                    y,
                },
            },
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(visible());
    controls.computer(session(), ComputerCommand::Close {}).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(!visible(), "computer_close must remove the indicator");
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await.unwrap().unwrap();
    controls
        .computer(
            session(),
            ComputerCommand::Act {
                revision: frame.revision,
                action: ControlCommand::Click {
                    x,
                    y,
                },
            },
        )
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(visible());
    controls.shutdown();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while visible() && std::time::Instant::now() < deadline {
        tokio::task::yield_now().await;
    }
    assert!(!visible(), "host shutdown must remove the indicator");
}

struct Desktop;
impl DesktopDriver for Desktop {
    fn execute(&self, _command: ControlCommand, _target: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
        assert!(allowed());
        Ok(Some(ControlFrame {
            width: 1,
            height: 1,
            url: None,
            image: Attachment::Text {
                name: "fixture".into(),
                text: String::new(),
                source: None,
            },
        }))
    }
}

#[tokio::test]
async fn given_automatic_access_when_a_job_captures_then_only_its_execution_owns_the_desktop() {
    let controls = Controls::default().with_desktop_driver(Arc::new(Desktop)).with_automatic_desktop();
    let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let session = |job_id, execution_id| {
        let active = active.clone();
        ComputerSession {
            job_id,
            execution_id,
            active: Arc::new(move || active.load(std::sync::atomic::Ordering::SeqCst)),
        }
    };
    assert!(controls.computer(session(1, 1), ComputerCommand::Capture {}).await.is_ok());
    assert!(controls.computer(session(2, 2), ComputerCommand::Capture {}).await.is_err());
    assert!(controls.desktop(ControlCommand::Capture {}).await.is_err());
    active.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(controls.desktop(ControlCommand::Capture {}).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn given_an_active_automatic_execution_when_ten_minutes_pass_then_it_keeps_ownership() {
    let controls = Controls::default().with_desktop_driver(Arc::new(Desktop)).with_automatic_desktop();
    let session = |job_id| ComputerSession {
        job_id,
        execution_id: job_id as u64,
        active: Arc::new(|| true),
    };
    assert!(controls.computer(session(1), ComputerCommand::Capture {}).await.is_ok());
    tokio::time::advance(std::time::Duration::from_secs(601)).await;
    assert!(controls.computer(session(1), ComputerCommand::Capture {}).await.is_ok());
    assert!(controls.computer(session(2), ComputerCommand::Capture {}).await.is_err());
}

#[tokio::test]
async fn given_automatic_manual_access_when_an_active_job_is_granted_then_the_desktop_has_one_owner() {
    let controls = Controls::default().with_desktop_driver(Arc::new(Desktop)).with_automatic_desktop();
    controls
        .grant_computer(
            ComputerGrantRequest {
                job_id: 42,
                enabled: true,
            },
            Arc::new(|| true),
        )
        .await
        .unwrap();
    assert!(controls.desktop(ControlCommand::Capture {}).await.is_err());
    controls
        .grant_computer(
            ComputerGrantRequest {
                job_id: 42,
                enabled: false,
            },
            Arc::new(|| true),
        )
        .await
        .unwrap();
    assert!(controls.desktop(ControlCommand::Capture {}).await.is_ok());
}

#[tokio::test]
async fn given_an_available_desktop_when_captured_then_no_manual_grant_is_required() {
    let controls = Controls::default().with_desktop_driver(Arc::new(Desktop)).with_automatic_desktop();
    assert!(controls.desktop(ControlCommand::Capture {}).await.is_ok());
}

#[tokio::test]
async fn given_an_agent_target_when_execution_ends_then_manual_input_cannot_inherit_it() {
    struct Target;
    impl DesktopDriver for Target {
        fn execute(&self, command: ControlCommand, target: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
            if matches!(command, ControlCommand::Capture {}) {
                target.store(42, std::sync::atomic::Ordering::SeqCst);
            } else {
                assert_eq!(target.load(std::sync::atomic::Ordering::SeqCst), 0);
            }
            Desktop.execute(command, target, allowed)
        }
    }
    let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let current = active.clone();
    let controls = Controls::default().with_desktop_driver(Arc::new(Target)).with_automatic_desktop();
    controls
        .computer(
            ComputerSession {
                job_id: 1,
                execution_id: 1,
                active: Arc::new(move || current.load(std::sync::atomic::Ordering::SeqCst)),
            },
            ComputerCommand::Capture {},
        )
        .await
        .unwrap();
    active.store(false, std::sync::atomic::Ordering::SeqCst);
    controls
        .desktop(ControlCommand::Text {
            text: "manual".into(),
        })
        .await
        .unwrap();
    controls.shutdown();
    assert!(!controls.automatic_computer_enabled());
    assert!(controls.desktop(ControlCommand::Capture {}).await.is_err());
}
