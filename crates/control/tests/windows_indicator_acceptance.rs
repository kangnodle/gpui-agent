#![cfg(windows)]

use anyhow::{Result,
             ensure};
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
                atomic::{AtomicIsize,
                         AtomicUsize,
                         Ordering}};
use windows_sys::Win32::{Foundation::*,
                         Graphics::Gdi::*,
                         UI::{HiDpi::*,
                              Input::KeyboardAndMouse::*,
                              WindowsAndMessaging::*}};

static CLICKS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn fixture_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_LBUTTONDOWN {
        CLICKS.fetch_add(1, Ordering::SeqCst);
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

struct Screen;
impl DesktopDriver for Screen {
    fn execute(&self, _: ControlCommand, _: &AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
        ensure!(allowed());
        Ok(Some(ControlFrame {
            width: 32768,
            height: 32768,
            url: None,
            image: Attachment::Text {
                name: "fixture".into(),
                text: String::new(),
                source: None,
            },
        }))
    }
}

unsafe fn capture_pixel(x: i32, y: i32) -> Result<u32> {
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        ensure!(!screen.is_null());
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, 1, 1);
        ensure!(!dc.is_null() && !bitmap.is_null());
        let old = SelectObject(dc, bitmap);
        let success = StretchBlt(dc, 0, 0, 1, 1, screen, x, y, 1, 1, SRCCOPY | CAPTUREBLT);
        let pixel = GetPixel(dc, 0, 0);
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        ensure!(success != 0 && pixel != u32::MAX);
        Ok(pixel)
    }
}

#[tokio::test]
#[ignore = "takes ten minutes and displays an owned Windows test window"]
async fn given_an_indicator_when_the_real_grant_expires_then_it_closes_without_another_request() -> Result<()> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let point = POINT {
        x: 200,
        y: 200,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let fixture = std::thread::spawn(move || unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class: Vec<u16> = "gpui-agent-indicator-acceptance\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(fixture_proc),
            lpszClassName: class.as_ptr(),
            hbrBackground: GetStockObject(WHITE_BRUSH) as _,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP | WS_VISIBLE,
            point.x - 80,
            point.y - 80,
            160,
            160,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        sender.send(hwnd as isize).ok();
        if hwnd.is_null() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    let hwnd = receiver.recv()? as HWND;
    ensure!(!hwnd.is_null());
    struct Fixture(HWND);
    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                let thread = GetWindowThreadProcessId(self.0, std::ptr::null_mut());
                PostMessageW(self.0, WM_CLOSE, 0, 0);
                PostThreadMessageW(thread, WM_QUIT, 0, 0);
            }
        }
    }
    let owned = Fixture(hwnd);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let before = loop {
        let pixel = unsafe { capture_pixel(point.x + 2, point.y + 2)? };
        if pixel == 0x00ff_ffff {
            break pixel;
        }
        ensure!(std::time::Instant::now() < deadline, "Fixture did not paint white");
        tokio::task::yield_now().await;
    };
    let controls = Controls::default().with_desktop_driver(Arc::new(Screen));
    controls
        .grant_computer(
            ComputerGrantRequest {
                job_id: 1,
                enabled: true,
            },
            Arc::new(|| true),
        )
        .await?;
    let session = || ComputerSession {
        job_id: 1,
        execution_id: 1,
        active: Arc::new(|| true),
    };
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await?.unwrap();
    controls
        .computer(
            session(),
            ComputerCommand::Act {
                revision: frame.revision,
                action: ControlCommand::Click {
                    x: (point.x - left) as u32,
                    y: (point.y - top) as u32,
                },
            },
        )
        .await?;
    let name: Vec<u16> = "gpui-agent computer use indicator\0".encode_utf16().collect();
    let indicator = || unsafe {
        let hwnd = FindWindowW(std::ptr::null(), name.as_ptr());
        !hwnd.is_null() && IsWindowVisible(hwnd) != 0
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !indicator() {
        ensure!(std::time::Instant::now() < deadline, "Indicator did not appear");
        tokio::task::yield_now().await;
    }
    ensure!(
        unsafe { capture_pixel(point.x + 2, point.y + 2)? } == before,
        "Indicator contaminated the real StretchBlt capture"
    );
    ensure!(unsafe { WindowFromPoint(point) } == hwnd, "Indicator intercepted the fixture hit test");
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let mut previous = POINT::default();
    ensure!(unsafe { GetCursorPos(&mut previous) } != 0);
    let mouse = |flags, dx, dy| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let input = [
        mouse(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            (((point.x - left) as i64 * 65535) / (width - 1) as i64) as i32,
            (((point.y - top) as i64 * 65535) / (height - 1) as i64) as i32,
        ),
        mouse(MOUSEEVENTF_LEFTDOWN, 0, 0),
        mouse(MOUSEEVENTF_LEFTUP, 0, 0),
    ];
    ensure!(unsafe { SendInput(input.len() as u32, input.as_ptr(), std::mem::size_of::<INPUT>() as i32) } == input.len() as u32);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while CLICKS.load(Ordering::SeqCst) == 0 {
        ensure!(std::time::Instant::now() < deadline, "Fixture did not receive the click");
        tokio::task::yield_now().await;
    }
    let mut current = POINT::default();
    if unsafe { GetCursorPos(&mut current) } != 0 && (current.x - point.x).abs() <= 1 && (current.y - point.y).abs() <= 1 {
        unsafe {
            SetCursorPos(previous.x, previous.y);
        }
    }
    println!("ok: capture excludes the actual indicator and a real click reaches the owned fixture; waiting for the 10-minute grant to expire");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(620);
    while indicator() {
        ensure!(tokio::time::Instant::now() < deadline, "Expired indicator remained visible");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    ensure!(controls.computer_status().await?.job_id.is_none());
    println!("ok: indicator disappeared on real grant expiry without a subsequent request");
    drop(owned);
    fixture.join().map_err(|_| anyhow::anyhow!("Fixture thread panicked"))?;
    Ok(())
}

#[tokio::test]
#[ignore = "temporarily changes and restores the primary monitor resolution"]
async fn given_a_display_mode_change_when_an_indicator_is_visible_then_it_follows_the_monitor() -> Result<()> {
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut original = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    ensure!(
        unsafe { EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut original) } != 0,
        "Cannot read the original display mode"
    );
    let mut alternate = None;
    for index in 0 .. 4096 {
        let mut candidate = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if unsafe { EnumDisplaySettingsW(std::ptr::null(), index, &mut candidate) } == 0 {
            break;
        }
        if candidate.dmPelsWidth * 100 >= original.dmPelsWidth * 65
            && candidate.dmPelsHeight * 100 >= original.dmPelsHeight * 65
            && candidate.dmPelsWidth <= original.dmPelsWidth
            && candidate.dmPelsHeight <= original.dmPelsHeight
            && (candidate.dmPelsWidth as i64 * original.dmPelsHeight as i64 - original.dmPelsWidth as i64 * candidate.dmPelsHeight as i64).abs()
                < original.dmPelsWidth as i64 * candidate.dmPelsHeight as i64 / 50
            && candidate.dmBitsPerPel == original.dmBitsPerPel
            && candidate.dmDisplayFrequency == original.dmDisplayFrequency
            && (candidate.dmPelsWidth != original.dmPelsWidth || candidate.dmPelsHeight != original.dmPelsHeight)
            && unsafe { ChangeDisplaySettingsExW(std::ptr::null(), &candidate, std::ptr::null_mut(), CDS_TEST, std::ptr::null()) } == DISP_CHANGE_SUCCESSFUL
        {
            let score = candidate.dmPelsWidth as u64 * candidate.dmPelsHeight as u64;
            if alternate
                .as_ref()
                .is_none_or(|best: &DEVMODEW| score > best.dmPelsWidth as u64 * best.dmPelsHeight as u64)
            {
                alternate = Some(candidate);
            }
        }
    }
    let alternate = alternate.ok_or_else(|| anyhow::anyhow!("No safe alternate display resolution available"))?;
    let controls = Controls::default().with_desktop_driver(Arc::new(Screen)).with_automatic_desktop();
    let session = || ComputerSession {
        job_id: 1,
        execution_id: 1,
        active: Arc::new(|| true),
    };
    let frame = controls.computer(session(), ComputerCommand::Capture {}).await?.unwrap();
    let x = (200 - unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) }) as u32;
    let y = (200 - unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) }) as u32;
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
        .await?;
    let name: Vec<u16> = "gpui-agent computer use indicator\0".encode_utf16().collect();
    let wait = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let hwnd = loop {
        let hwnd = unsafe { FindWindowW(std::ptr::null(), name.as_ptr()) };
        if !hwnd.is_null() && unsafe { IsWindowVisible(hwnd) } != 0 {
            break hwnd;
        }
        ensure!(std::time::Instant::now() < wait, "Indicator did not appear");
        tokio::task::yield_now().await;
    };
    struct Restore {
        original: DEVMODEW,
        armed: bool,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if self.armed {
                unsafe {
                    ChangeDisplaySettingsExW(std::ptr::null(), &self.original, std::ptr::null_mut(), 0, std::ptr::null());
                }
            }
        }
    }
    let mut restore = Restore {
        original,
        armed: true,
    };
    ensure!(
        unsafe { ChangeDisplaySettingsExW(std::ptr::null(), &alternate, std::ptr::null_mut(), 0, std::ptr::null()) } == DISP_CHANGE_SUCCESSFUL,
        "Display mode change failed"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let mut rect = RECT::default();
        ensure!(
            unsafe { IsWindowVisible(hwnd) } != 0 && unsafe { GetWindowRect(hwnd, &mut rect) } != 0,
            "Indicator disappeared after display change"
        );
        if rect.right - rect.left == alternate.dmPelsWidth as i32 && rect.bottom - rect.top == alternate.dmPelsHeight as i32 {
            break;
        }
        ensure!(std::time::Instant::now() < deadline, "Indicator did not adapt to the new display dimensions");
        tokio::task::yield_now().await;
    }
    ensure!(
        unsafe { ChangeDisplaySettingsExW(std::ptr::null(), &restore.original, std::ptr::null_mut(), 0, std::ptr::null()) } == DISP_CHANGE_SUCCESSFUL,
        "Original display mode could not be restored"
    );
    restore.armed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let mut rect = RECT::default();
        ensure!(
            unsafe { IsWindowVisible(hwnd) } != 0 && unsafe { GetWindowRect(hwnd, &mut rect) } != 0,
            "Indicator disappeared during display restore"
        );
        if rect.right - rect.left == restore.original.dmPelsWidth as i32 && rect.bottom - rect.top == restore.original.dmPelsHeight as i32 {
            break;
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "Indicator did not return to the restored display dimensions"
        );
        tokio::task::yield_now().await;
    }
    println!("ok: indicator resized with the real display mode and followed the restored mode");
    Ok(())
}
