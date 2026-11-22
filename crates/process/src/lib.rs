#![cfg_attr(not(test), deny(clippy::unwrap_used))]
//! Owned process trees shared by native runtimes and manual browser control.
use anyhow::{Context,
             Result};
use tokio::process::{Child,
                     Command};

pub struct Process {
    child: Child,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl std::ops::Deref for Process {
    type Target = Child;

    fn deref(&self) -> &Child { &self.child }
}
impl std::ops::DerefMut for Process {
    fn deref_mut(&mut self) -> &mut Child { &mut self.child }
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(id) = self.child.id() {
            // The process group was created specifically for this owned
            // runtime.
            unsafe {
                libc::kill(-(id as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

/// Starts an owned process tree. Dropping the guard terminates its descendants.
pub fn spawn(command: &mut Command) -> Result<Process> {
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    let job = {
        use std::os::windows::io::{AsRawHandle,
                                   FromRawHandle};
        use windows_sys::Win32::System::JobObjects::*;
        // Closing our job handle terminates the runtime and its
        // unpacker/harness descendants.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let job = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        job
    };
    let child = command.spawn().context("could not start native runtime; install it or set --executable")?;
    #[cfg(windows)]
    let mut child = child;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let process_handle = match child.raw_handle() {
            | Some(handle) => handle,
            | None => {
                let _ = child.start_kill();
                return Err(anyhow::anyhow!("Native runtime process handle is unavailable"));
            },
        };
        if unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(job.as_raw_handle(), process_handle) } == 0 {
            let _ = child.start_kill();
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(Process {
        child,
        #[cfg(windows)]
        _job: job,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt,
                    BufReader};

    #[test]
    #[ignore = "subprocess fixture invoked by drop_terminates_owned_child"]
    fn child_fixture() {
        use std::io::Write;
        println!("ready");
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }

    #[tokio::test]
    async fn drop_terminates_owned_child() -> Result<()> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["--ignored", "--exact", "tests::child_fixture", "--nocapture"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut process = spawn(&mut command)?;
        let mut output = BufReader::new(process.stdout.take().unwrap()).lines();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(line) = output.next_line().await? {
                if line == "ready" {
                    return Ok::<_, anyhow::Error>(());
                }
            }
            anyhow::bail!("child exited before readiness");
        })
        .await??;

        #[cfg(windows)]
        let handle = {
            use std::os::windows::io::BorrowedHandle;
            unsafe { BorrowedHandle::borrow_raw(process.raw_handle().unwrap()) }.try_clone_to_owned()?
        };
        #[cfg(unix)]
        let pid = process.id().unwrap() as i32;
        drop(process);

        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            assert_eq!(
                unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(handle.as_raw_handle(), 5000) },
                0,
                "owned child must exit when its guard is dropped"
            );
        }
        #[cfg(unix)]
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let status = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
                if status == pid || (status == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        Ok(())
    }
}
