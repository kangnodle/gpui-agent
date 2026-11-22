use anyhow::{Result,
             bail};
use gpui_agent_contracts::{ControlCommand,
                           ControlFrame};

#[cfg(windows)]
fn process_identity(process_id: u32) -> Option<gpui_agent_contracts::ComputerForeground> {
    use windows_sys::Win32::{Foundation::*,
                             System::Threading::*};
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if process.is_null() {
            return None;
        }
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(process);
        (ok != 0).then_some(gpui_agent_contracts::ComputerForeground {
            process_id,
            started_at: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        })
    }
}

pub(super) fn foreground_identity() -> Option<gpui_agent_contracts::ComputerForeground> {
    #[cfg(windows)]
    {
        process_identity(std::process::id())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Call from the foreground client when the user approves native computer
/// input. Failure leaves the host's normal focus check in place.
pub fn allow_computer_foreground(identity: &gpui_agent_contracts::ComputerForeground) -> bool {
    #[cfg(windows)]
    {
        if identity.started_at == 0 || !process_identity(identity.process_id).is_some_and(|local| local.started_at == identity.started_at) {
            return false;
        }
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(identity.process_id) != 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = identity;
        false
    }
}

#[cfg(all(test, windows))]
mod foreground_tests {
    #[test]
    fn given_a_remote_or_reused_pid_when_focus_is_delegated_then_it_is_rejected() {
        let mut identity = super::foreground_identity().unwrap();
        identity.started_at += 1;
        assert!(!super::allow_computer_foreground(&identity));
        identity.process_id = 0;
        assert!(!super::allow_computer_foreground(&identity));
    }
}

pub(super) fn key_code(key: &str) -> Result<u16> {
    Ok(match key {
        | "Enter" => 13,
        | "Tab" => 9,
        | "Escape" => 27,
        | "Backspace" => 8,
        | "ArrowUp" => 38,
        | "ArrowDown" => 40,
        | "ArrowLeft" => 37,
        | "ArrowRight" => 39,
        | "Home" => 36,
        | "End" => 35,
        | "Delete" => 46,
        | _ => bail!("Unsupported key"),
    })
}

#[cfg(not(windows))]
pub(super) fn execute(_: ControlCommand, _: &std::sync::atomic::AtomicIsize, _: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
    bail!("Desktop control requires an interactive Windows session")
}

#[cfg(windows)]
pub(super) fn execute(command: ControlCommand, target: &std::sync::atomic::AtomicIsize, allowed: &dyn Fn() -> bool) -> Result<Option<ControlFrame>> {
    use anyhow::ensure;
    use windows_sys::Win32::{Graphics::Gdi::*,
                             UI::{HiDpi::*,
                                  Input::KeyboardAndMouse::*,
                                  WindowsAndMessaging::*}};
    struct Dpi(isize);
    impl Drop for Dpi {
        fn drop(&mut self) {
            unsafe {
                SetThreadDpiAwarenessContext(self.0 as _);
            }
        }
    }
    unsafe {
        ensure!(allowed(), "Desktop authorization expired");
        let _dpi = Dpi(SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) as isize);
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        ensure!(
            width > 0 && height > 0 && width <= 32768 && height <= 32768,
            "No capturable interactive desktop"
        );
        if matches!(
            command,
            ControlCommand::Text { .. } | ControlCommand::Key { .. } | ControlCommand::Scroll { .. }
        ) {
            let window = target.load(std::sync::atomic::Ordering::SeqCst) as windows_sys::Win32::Foundation::HWND;
            ensure!(
                !window.is_null() && IsWindow(window) != 0,
                "Click the intended target window in the captured screen first"
            );
            if GetForegroundWindow() != window {
                ensure!(allowed(), "Desktop authorization expired before focus");
                ensure!(
                    SetForegroundWindow(window) != 0,
                    "Windows prevented target focus; click the target window again"
                );
            }
            for key in [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN] {
                ensure!(GetAsyncKeyState(key as i32) >= 0, "Release modifier keys before sending input");
            }
        }
        let clicked = matches!(command, ControlCommand::Click { .. });
        let mut inputs = vec![];
        let keyboard = |vk, scan, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        };
        let mouse = |flags, data| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: flags,
                    mouseData: data,
                    ..Default::default()
                },
            },
        };
        match command {
            | ControlCommand::Capture {} => {},
            | ControlCommand::Click {
                x,
                y,
            } => {
                ensure!(x < width as u32 && y < height as u32, "Click is outside the Windows desktop");
                // A single input batch avoids clicking at an old pointer
                // position.
                inputs.push(INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: ((x as u64 * 65535) / (width - 1).max(1) as u64) as i32,
                            dy: ((y as u64 * 65535) / (height - 1).max(1) as u64) as i32,
                            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                            ..Default::default()
                        },
                    },
                });
                inputs.extend([mouse(MOUSEEVENTF_LEFTDOWN, 0), mouse(MOUSEEVENTF_LEFTUP, 0)]);
            },
            | ControlCommand::Text {
                text,
            } =>
                for unit in text.encode_utf16() {
                    if unit == 10 {
                        inputs.extend([keyboard(VK_RETURN, 0, 0), keyboard(VK_RETURN, 0, KEYEVENTF_KEYUP)]);
                    } else {
                        inputs.extend([keyboard(0, unit, KEYEVENTF_UNICODE), keyboard(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]);
                    }
                },
            | ControlCommand::Key {
                key,
            } => {
                let vk = key_code(&key)?;
                let flags = if matches!(vk, 35 ..= 40 | 46) { KEYEVENTF_EXTENDEDKEY } else { 0 };
                inputs.extend([keyboard(vk, 0, flags), keyboard(vk, 0, flags | KEYEVENTF_KEYUP)]);
            },
            | ControlCommand::Scroll {
                delta,
            } => {
                inputs.push(mouse(MOUSEEVENTF_WHEEL, (-delta) as u32));
            },
            | _ => bail!("Unsupported Windows desktop command"),
        }
        if !inputs.is_empty() {
            ensure!(allowed(), "Desktop authorization expired before input");
            ensure!(
                SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) == inputs.len() as u32,
                "Windows blocked input (secure desktop, elevation/UIPI or inactive session)"
            );
            // Give the foreground app a chance to paint before the snapshot.
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
        if clicked {
            target.store(GetForegroundWindow() as isize, std::sync::atomic::Ordering::SeqCst);
        }
        let scale = (1920.0 / width as f64).min(1080.0 / height as f64).min(1.0);
        let w = (width as f64 * scale).round().max(1.0) as i32;
        let h = (height as f64 * scale).round().max(1.0) as i32;
        struct Surface {
            screen: HDC,
            dc: HDC,
            bitmap: HBITMAP,
            old: HGDIOBJ,
        }
        impl Drop for Surface {
            fn drop(&mut self) {
                unsafe {
                    if !self.old.is_null() && self.old as isize != -1 {
                        SelectObject(self.dc, self.old);
                    }
                    if !self.bitmap.is_null() {
                        DeleteObject(self.bitmap);
                    }
                    if !self.dc.is_null() {
                        DeleteDC(self.dc);
                    }
                    if !self.screen.is_null() {
                        ReleaseDC(std::ptr::null_mut(), self.screen);
                    }
                }
            }
        }
        let screen = GetDC(std::ptr::null_mut());
        ensure!(!screen.is_null(), "Cannot open Windows desktop");
        let mut surface = Surface {
            screen,
            dc: CreateCompatibleDC(screen),
            bitmap: std::ptr::null_mut(),
            old: std::ptr::null_mut(),
        };
        ensure!(!surface.dc.is_null(), "Cannot create capture surface");
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        surface.bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        ensure!(!surface.bitmap.is_null() && !bits.is_null(), "Cannot allocate capture surface");
        surface.old = SelectObject(surface.dc, surface.bitmap);
        ensure!(!surface.old.is_null() && surface.old as isize != -1, "Cannot select capture surface");
        SetStretchBltMode(surface.dc, HALFTONE);
        ensure!(
            StretchBlt(surface.dc, 0, 0, w, h, screen, left, top, width, height, SRCCOPY | CAPTUREBLT) != 0,
            "Windows screen capture failed"
        );
        GdiFlush();
        let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
        let rgb: Vec<u8> = pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
        let mut bytes = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80).encode(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)?;
        let image = gpui_agent_contracts::Attachment::from_bytes("windows-capture.jpg".into(), &bytes).map_err(anyhow::Error::msg)?;
        ensure!(allowed(), "Desktop authorization expired during capture");
        Ok(Some(ControlFrame {
            width: width as u32,
            height: height as u32,
            url: None,
            image,
        }))
    }
}
