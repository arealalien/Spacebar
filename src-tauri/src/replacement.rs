use std::{
    collections::HashMap,
    io::{Read, Write},
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
    UI::{
        Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA},
        WindowsAndMessaging::{
            EnumWindows, FindWindowW, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
            ShowWindowAsync, SW_HIDE, SW_SHOWNOACTIVATE,
        },
    },
};

pub const GUARD_ARGUMENT: &str = "--taskbar-guardian";

#[derive(Default)]
pub struct Replacement(Mutex<Option<Guardian>>);

struct Guardian {
    process: Child,
    auto_hide: Option<bool>,
}

impl Replacement {
    pub fn update(&self, enabled: bool, auto_hide: bool) -> Result<(), String> {
        let mut child = self.0.lock().map_err(|error| error.to_string())?;
        if !enabled {
            stop(&mut child);
            return Ok(());
        }
        if !child
            .as_mut()
            .is_some_and(|guard| matches!(guard.process.try_wait(), Ok(None)))
        {
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            *child = Some(Guardian {
                process: Command::new(exe)
                    .arg(GUARD_ARGUMENT)
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("Couldn't replace the Windows taskbar. {error}"))?,
                auto_hide: None,
            });
        }
        if let Some(guard) = child
            .as_mut()
            .filter(|guard| guard.auto_hide != Some(auto_hide))
        {
            guard
                .process
                .stdin
                .as_mut()
                .ok_or("Taskbar guardian is unavailable.")?
                .write_all(&[u8::from(auto_hide)])
                .map_err(|error| format!("Couldn't sync Windows taskbar auto-hide. {error}"))?;
            guard.auto_hide = Some(auto_hide);
        }
        Ok(())
    }
}

impl Drop for Replacement {
    fn drop(&mut self) {
        stop(self.0.get_mut().unwrap_or_else(|error| error.into_inner()));
    }
}

fn stop(child: &mut Option<Guardian>) {
    if let Some(Guardian { mut process, .. }) = child.take() {
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
    original_auto_hide: Option<bool>,
    synced: Option<(HWND, u32, bool)>,
}

impl Taskbars {
    fn show(&mut self, visible: bool) {
        self.visible = visible;
        unsafe {
            EnumWindows(Some(apply_visibility), (self as *mut Self) as LPARAM);
        }
    }

    fn restore(&self) {
        if let (Some(auto_hide), Some((hwnd, _))) = (self.original_auto_hide, primary_taskbar()) {
            sync_auto_hide(hwnd, auto_hide);
        }
        for (&hwnd, &(pid, visible)) in &self.originals {
            if shell_bar(hwnd).is_some_and(|(current, _)| current == pid) {
                unsafe {
                    ShowWindowAsync(hwnd, show_mode(visible));
                }
            }
        }
    }

    fn sync(&mut self, auto_hide: bool) {
        self.show(false);
        if let Some((hwnd, pid)) = primary_taskbar() {
            if self.synced != Some((hwnd, pid, auto_hide)) {
                let original = sync_auto_hide(hwnd, auto_hide);
                self.original_auto_hide.get_or_insert(original);
                self.synced = Some((hwnd, pid, auto_hide));
            }
        } else {
            self.synced = None;
        }
    }
}

fn primary_taskbar() -> Option<(HWND, u32)> {
    let hwnd = unsafe { FindWindowW(windows::core::w!("Shell_TrayWnd").0, null()) };
    let mut pid = 0;
    if !hwnd.is_null() {
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    }
    (pid != 0).then_some((hwnd, pid))
}

fn sync_auto_hide(hwnd: HWND, enabled: bool) -> bool {
    let mut data = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    };
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) } as isize;
    let flag = ABS_AUTOHIDE as isize;
    let previous = state & flag != 0;
    if previous != enabled {
        data.lParam = if enabled { state | flag } else { state & !flag };
        unsafe { SHAppBarMessage(ABM_SETSTATE, &mut data) };
    }
    previous
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
        let original = bars.originals.entry(hwnd).or_insert((pid, visible));
        if original.0 != pid {
            *original = (pid, visible);
        }
        if visible != bars.visible {
            ShowWindowAsync(hwnd, show_mode(bars.visible));
        }
    }
    1
}

pub fn guard() {
    let Some(_lock) = GuardLock::acquire() else {
        return;
    };
    let (writer, pipe) = mpsc::channel();
    thread::spawn(move || {
        let mut input = std::io::stdin().lock();
        let mut buffer = [0u8];
        while matches!(input.read(&mut buffer), Ok(1)) {
            if writer.send(buffer[0] != 0).is_err() {
                break;
            }
        }
    });
    let mut bars = Taskbars::default();
    let mut auto_hide = None;
    loop {
        match pipe.recv_timeout(Duration::from_millis(100)) {
            Ok(value) => auto_hide = Some(value),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Some(enabled) = auto_hide {
            bars.sync(enabled);
        }
    }
    bars.restore();
}
