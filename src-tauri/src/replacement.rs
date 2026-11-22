use std::{
    collections::HashMap,
    io::Read,
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    ptr::null,
    sync::{mpsc, Mutex},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_ABANDONED, WAIT_OBJECT_0},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject, CREATE_NO_WINDOW},
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, ShowWindowAsync,
        SW_HIDE, SW_SHOWNOACTIVATE,
    },
};

pub const GUARD_ARGUMENT: &str = "--taskbar-guardian";

#[derive(Default)]
pub struct Replacement(Mutex<Option<Child>>);

impl Replacement {
    pub fn update(&self, enabled: bool) -> Result<(), String> {
        let mut child = self.0.lock().map_err(|error| error.to_string())?;
        if !enabled {
            stop(&mut child);
        } else if !child
            .as_mut()
            .is_some_and(|process| matches!(process.try_wait(), Ok(None)))
        {
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            *child = Some(
                Command::new(exe)
                    .arg(GUARD_ARGUMENT)
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("Couldn't replace the Windows taskbar. {error}"))?,
            );
        }
        Ok(())
    }
}

impl Drop for Replacement {
    fn drop(&mut self) {
        stop(self.0.get_mut().unwrap_or_else(|error| error.into_inner()));
    }
}

fn stop(child: &mut Option<Child>) {
    if let Some(mut process) = child.take() {
        // EOF also reaches the guardian if Windows terminates the dock process.
        drop(process.stdin.take());
        let _ = process.wait();
    }
}

struct GuardLock(HANDLE);

impl GuardLock {
    fn acquire() -> Option<Self> {
        let name: Vec<u16> = "Local\\Spacebar.TaskbarGuardian\0".encode_utf16().collect();
        let lock = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if lock.is_null() {
            return None;
        }
        if matches!(
            unsafe { WaitForSingleObject(lock, 0) },
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            Some(Self(lock))
        } else {
            unsafe { CloseHandle(lock) };
            None
        }
    }
}

impl Drop for GuardLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

fn shell_bar(hwnd: HWND) -> Option<(u32, bool)> {
    let mut class = [0_u16; 32];
    let length = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    let name = String::from_utf16_lossy(&class[..length.max(0) as usize]);
    if name != "Shell_TrayWnd" && name != "Shell_SecondaryTrayWnd" {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    Some((pid, unsafe { IsWindowVisible(hwnd) != 0 }))
}

#[derive(Default)]
struct Taskbars {
    originals: HashMap<HWND, (u32, bool)>,
    visible: bool,
}

impl Taskbars {
    fn show(&mut self, visible: bool) {
        self.visible = visible;
        unsafe {
            EnumWindows(Some(apply_visibility), (self as *mut Self) as LPARAM);
        }
    }

    fn restore(&self) {
        for (&hwnd, &(pid, visible)) in &self.originals {
            if shell_bar(hwnd).is_some_and(|(current, _)| current == pid) {
                unsafe {
                    ShowWindowAsync(hwnd, show_mode(visible));
                }
            }
        }
    }
}

fn show_mode(visible: bool) -> i32 {
    if visible {
        SW_SHOWNOACTIVATE
    } else {
        SW_HIDE
    }
}

unsafe extern "system" fn apply_visibility(hwnd: HWND, data: LPARAM) -> i32 {
    let bars = &mut *(data as *mut Taskbars);
    if let Some((pid, visible)) = shell_bar(hwnd) {
        if visible != bars.visible && ShowWindowAsync(hwnd, show_mode(bars.visible)) != 0 {
            let original = bars.originals.entry(hwnd).or_insert((pid, visible));
            if original.0 != pid {
                *original = (pid, visible);
            }
        }
    }
    1
}

pub fn guard() {
    let Some(_lock) = GuardLock::acquire() else {
        return;
    };
    let (writer, pipe) = mpsc::channel::<()>();
    thread::spawn(move || {
        let _writer = writer;
        let mut input = std::io::stdin().lock();
        let mut buffer = [0u8];
        while matches!(input.read(&mut buffer), Ok(1)) {}
    });
    let mut bars = Taskbars::default();
    while let Err(mpsc::RecvTimeoutError::Timeout) = pipe.recv_timeout(Duration::from_millis(100)) {
        bars.show(false);
    }
    bars.restore();
}
