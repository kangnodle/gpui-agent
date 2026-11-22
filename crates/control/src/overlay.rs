use anyhow::{Context,
             Result};
use std::sync::{Arc,
                atomic::{AtomicU64,
                         Ordering}};
use windows_sys::{Wdk::System::SystemServices::RtlGetVersion,
                  Win32::{Foundation::*,
                          Graphics::Gdi::*,
                          System::SystemInformation::OSVERSIONINFOW,
                          UI::{HiDpi::*,
                               WindowsAndMessaging::*}}};

fn pointer_points(x: i32, y: i32) -> [POINT; 7] {
    [
        POINT {
            x,
            y,
        },
        POINT {
            x,
            y: y + 44,
        },
        POINT {
            x: x + 12,
            y: y + 32,
        },
        POINT {
            x: x + 20,
            y: y + 48,
        },
        POINT {
            x: x + 28,
            y: y + 44,
        },
        POINT {
            x: x + 20,
            y: y + 28,
        },
        POINT {
            x: x + 36,
            y: y + 28,
        },
    ]
}

unsafe extern "system" fn paint(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_ERASEBKGND {
            return 1;
        }
        if msg == WM_PAINT {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect);
            let orange = CreateSolidBrush(0x0000a5ff);
            if !orange.is_null() {
                FillRect(dc, &rect, orange);
                let black = GetStockObject(BLACK_BRUSH) as HBRUSH;
                for edge in [
                    RECT {
                        left: 0,
                        top: 0,
                        right: rect.right,
                        bottom: 2,
                    },
                    RECT {
                        left: 0,
                        top: rect.bottom - 2,
                        right: rect.right,
                        bottom: rect.bottom,
                    },
                    RECT {
                        left: 0,
                        top: 0,
                        right: 2,
                        bottom: rect.bottom,
                    },
                    RECT {
                        left: rect.right - 2,
                        top: 0,
                        right: rect.right,
                        bottom: rect.bottom,
                    },
                    RECT {
                        left: 10,
                        top: 10,
                        right: rect.right - 10,
                        bottom: 12,
                    },
                    RECT {
                        left: 10,
                        top: rect.bottom - 12,
                        right: rect.right - 10,
                        bottom: rect.bottom - 10,
                    },
                    RECT {
                        left: 10,
                        top: 10,
                        right: 12,
                        bottom: rect.bottom - 10,
                    },
                    RECT {
                        left: rect.right - 12,
                        top: 10,
                        right: rect.right - 10,
                        bottom: rect.bottom - 10,
                    },
                ] {
                    FillRect(dc, &edge, black);
                }
                let point = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const POINT;
                if !point.is_null() {
                    let mut screen = RECT::default();
                    GetWindowRect(hwnd, &mut screen);
                    let points = pointer_points((*point).x - screen.left, (*point).y - screen.top);
                    let pen = CreatePen(PS_SOLID, 3, 0);
                    if !pen.is_null() {
                        let old_pen = SelectObject(dc, pen);
                        let old_brush = SelectObject(dc, orange);
                        Polygon(dc, points.as_ptr(), points.len() as i32);
                        SelectObject(dc, old_brush);
                        SelectObject(dc, old_pen);
                        DeleteObject(pen);
                    }
                }
                DeleteObject(orange);
            }
            EndPaint(hwnd, &paint);
            return 0;
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

unsafe fn shape(hwnd: HWND, r: RECT, point: POINT, pulse: bool) -> bool {
    unsafe {
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        let region = CreateRectRgn(0, 0, w, h);
        let inner = CreateRectRgn(12, 12, w - 12, h - 12);
        if region.is_null() || inner.is_null() {
            if !region.is_null() {
                DeleteObject(region);
            }
            if !inner.is_null() {
                DeleteObject(inner);
            }
            return false;
        }
        let mut ok = CombineRgn(region, region, inner, RGN_DIFF) != ERROR;
        DeleteObject(inner);
        let x = point.x - r.left;
        let y = point.y - r.top;
        let points = pointer_points(x, y);
        let pointer = CreatePolygonRgn(points.as_ptr(), points.len() as i32, WINDING);
        if pointer.is_null() {
            ok = false;
        } else {
            ok &= CombineRgn(region, region, pointer, RGN_OR) != ERROR;
            DeleteObject(pointer);
        }
        if pulse {
            let circle = CreateEllipticRgn(x - 20, y - 20, x + 21, y + 21);
            if circle.is_null() {
                ok = false;
            } else {
                ok &= CombineRgn(region, region, circle, RGN_OR) != ERROR;
                DeleteObject(circle);
            }
        }
        if !ok || SetWindowRgn(hwnd, region, 1) == 0 {
            DeleteObject(region);
            return false;
        }
        true
    }
}

pub(super) struct Prepared {
    generation: Arc<AtomicU64>,
    previous: u64,
    activate: std::sync::mpsc::Sender<u64>,
}

impl Prepared {
    pub(super) fn activate(self) {
        if self
            .generation
            .compare_exchange(self.previous, self.previous.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let _ = self.activate.send(self.previous.wrapping_add(1));
        }
    }
}

pub(super) fn prepare(x: u32, y: u32, generation: Arc<AtomicU64>, valid: Arc<dyn Fn() -> bool + Send + Sync>) -> Result<Prepared> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    anyhow::ensure!(
        unsafe { RtlGetVersion(&mut version) } == 0 && version.dwMajorVersion >= 10 && version.dwBuildNumber >= 19041,
        "Computer indicator requires Windows 10 version 2004 or later"
    );
    let previous = generation.load(Ordering::SeqCst);
    let thread_generation = generation.clone();
    let (activate, activated) = std::sync::mpsc::channel();
    let (ready, result) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new().name("computer-indicator".into()).spawn(move || unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let point = POINT {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN) + x as i32,
            y: GetSystemMetrics(SM_YVIRTUALSCREEN) + y as i32,
        };
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if monitor.is_null() || GetMonitorInfoW(monitor, &mut info) == 0 {
            let _ = ready.send(Err(anyhow::anyhow!("Computer indicator monitor unavailable")));
            return;
        }
        let class: Vec<u16> = "gpui-agent-indicator-class\0".encode_utf16().collect();
        let title: Vec<u16> = "gpui-agent computer use indicator\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(paint),
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let mut r = info.rcMonitor;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        );
        let mut affinity = 0;
        if !hwnd.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &point as *const POINT as isize);
        }
        if hwnd.is_null()
            || SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) == 0
            || GetWindowDisplayAffinity(hwnd, &mut affinity) == 0
            || affinity != WDA_EXCLUDEFROMCAPTURE
            || SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) == 0
        {
            if !hwnd.is_null() {
                DestroyWindow(hwnd);
            }
            let _ = ready.send(Err(anyhow::anyhow!("Cannot create capture-excluded computer indicator")));
            return;
        }
        if !shape(hwnd, r, point, true) {
            DestroyWindow(hwnd);
            let _ = ready.send(Err(anyhow::anyhow!("Cannot shape computer indicator")));
            return;
        }
        let _ = ready.send(Ok(()));
        let id = loop {
            match activated.recv_timeout(std::time::Duration::from_millis(100)) {
                | Ok(id) => break id,
                | Err(std::sync::mpsc::RecvTimeoutError::Timeout) if thread_generation.load(Ordering::SeqCst) == previous && valid() => {},
                | Err(_) => {
                    DestroyWindow(hwnd);
                    return;
                },
            }
        };
        if thread_generation.load(Ordering::SeqCst) == id && valid() {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let started = std::time::Instant::now();
        let mut pulse = true;
        let mut msg = MSG::default();
        while thread_generation.load(Ordering::SeqCst) == id && valid() {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let mut current = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let current_monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONULL);
            if current_monitor.is_null() || GetMonitorInfoW(current_monitor, &mut current) == 0 {
                break;
            }
            let next = current.rcMonitor;
            let changed = next.left != r.left || next.top != r.top || next.right != r.right || next.bottom != r.bottom;
            let end_pulse = pulse && started.elapsed() >= std::time::Duration::from_millis(400);
            if changed || end_pulse {
                r = next;
                pulse &= !end_pulse;
                if !shape(hwnd, r, point, pulse) || SetWindowPos(hwnd, HWND_TOPMOST, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE) == 0 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        DestroyWindow(hwnd);
    })?;
    result
        .recv_timeout(std::time::Duration::from_secs(3))
        .context("Computer indicator did not start")??;
    Ok(Prepared {
        generation,
        previous,
        activate,
    })
}
