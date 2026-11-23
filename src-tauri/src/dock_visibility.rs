use crate::{
    apps,
    desktop::{handle, monitor_info},
    dock_config::DockStore,
    system_widgets::shell_worker,
};
use serde::Serialize;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use windows::{
    core::{w, HRESULT},
    Win32::{
        Foundation::HWND as WindowHandle,
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST,
            FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
        },
    },
};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, HWND, POINT, RECT},
    Graphics::{
        Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        Gdi::{
            MonitorFromPoint, MonitorFromWindow, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
        },
    },
    UI::{
        Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN},
        WindowsAndMessaging::{
            GetAncestor, GetClassNameW, GetShellWindow, GetWindowLongPtrW, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, GA_ROOT, GWL_STYLE,
            WS_CAPTION, WS_THICKFRAME,
        },
    },
};

#[derive(Serialize)]
pub(crate) struct ExcludedApp {
    name: String,
    path: String,
}

fn display_path(path: &str) -> Cow<'_, str> {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(format!(r"\\{unc}"));
    }
    Cow::Borrowed(path.strip_prefix(r"\\?\").unwrap_or(path))
}

pub(crate) fn same_executable(left: &Path, right: &Path) -> bool {
    let left = left.to_string_lossy();
    let right = right.to_string_lossy();
    let left = display_path(&left);
    let right = display_path(&right);
    left.eq_ignore_ascii_case(&right)
        || ((!left.is_ascii() || !right.is_ascii()) && left.to_lowercase() == right.to_lowercase())
}

fn excluded_apps(paths: Vec<PathBuf>) -> Vec<ExcludedApp> {
    paths
        .into_iter()
        .map(|path| ExcludedApp {
            name: apps::app_name(&path),
            path: path.to_string_lossy().into_owned(),
        })
        .collect()
}

fn require_settings(source: &WebviewWindow) -> Result<(), String> {
    if source.label() == "settings" {
        Ok(())
    } else {
        Err("Open Spacebar Settings to manage excluded apps.".into())
    }
}

#[tauri::command]
pub(crate) fn read_dock_exclusions(
    source: WebviewWindow,
    store: State<'_, DockStore>,
) -> Result<Vec<ExcludedApp>, String> {
    require_settings(&source)?;
    store.dock_exclusions().map(excluded_apps)
}

#[tauri::command]
pub(crate) async fn add_dock_exclusion(
    app: AppHandle,
    source: WebviewWindow,
) -> Result<Vec<ExcludedApp>, String> {
    require_settings(&source)?;
    let owner = handle(&source)? as usize;
    let path =
        shell_worker(move || choose_executable(owner).map_err(|error| error.to_string())).await?;
    let store = app.state::<DockStore>();
    let Some(path) = path else {
        return store.dock_exclusions().map(excluded_apps);
    };
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err("Choose the program’s .exe file.".into());
    }
    let resolved = path.canonicalize().unwrap_or(path);
    let path = PathBuf::from(display_path(&resolved.to_string_lossy()).as_ref());
    store.edit_dock_exclusion(path, true).map(excluded_apps)
}

#[tauri::command]
pub(crate) fn remove_dock_exclusion(
    source: WebviewWindow,
    store: State<'_, DockStore>,
    path: PathBuf,
) -> Result<Vec<ExcludedApp>, String> {
    require_settings(&source)?;
    store.edit_dock_exclusion(path, false).map(excluded_apps)
}

fn choose_executable(owner: usize) -> windows::core::Result<Option<PathBuf>> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(w!("Choose a program to exclude"))?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: w!("Programs (*.exe)"),
            pszSpec: w!("*.exe"),
        }])?;
        match dialog.Show(Some(WindowHandle(owner as HWND))) {
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED) => return Ok(None),
            result => result?,
        }
        let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0.cast()));
        Ok(Some(PathBuf::from(path?)))
    }
}

#[derive(Default)]
pub(crate) struct ForegroundVisibility {
    identity: Option<(usize, u32)>,
    executable: Option<PathBuf>,
    last_check: Option<(usize, Instant)>,
    suppressed: bool,
}

impl ForegroundVisibility {
    pub(crate) fn sample(&mut self, hwnd: HWND, store: &DockStore, automatic: bool) -> bool {
        let now = Instant::now();
        if hwnd.is_null() {
            // Foreground transitions can briefly leave Windows without an active window.
            self.suppressed &= self.last_check.is_some_and(|(_, checked)| {
                now.duration_since(checked) < Duration::from_millis(160)
            });
            return self.suppressed;
        }
        if self.last_check.is_some_and(|(window, checked)| {
            window == hwnd as usize && now.duration_since(checked) < Duration::from_millis(80)
        }) {
            return self.suppressed;
        }
        self.last_check = Some((hwnd as usize, now));
        self.suppressed = self.check(hwnd, store, automatic);
        self.suppressed
    }

    fn check(&mut self, hwnd: HWND, store: &DockStore, automatic: bool) -> bool {
        let hwnd = unsafe { GetAncestor(hwnd, GA_ROOT) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 || pid == std::process::id() || !application_window(hwnd) {
            return false;
        }
        let identity = Some((hwnd as usize, pid));
        if self.identity != identity {
            self.identity = identity;
            self.executable = apps::process_path(pid);
        }
        if self
            .executable
            .as_deref()
            .is_some_and(|path| store.excludes_from_dock(path))
        {
            return true;
        }
        automatic && fullscreen_on_dock_display(hwnd)
    }
}

fn application_window(hwnd: HWND) -> bool {
    unsafe {
        if hwnd == GetShellWindow() || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return false;
        }
        let mut class = [0u16; 32];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        ![
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ]
        .into_iter()
        .any(|name| class[..len].iter().copied().eq(name.encode_utf16()))
    }
}

fn fullscreen_on_dock_display(hwnd: HWND) -> bool {
    let Ok(monitor) = monitor_info(POINT::default()) else {
        return false;
    };
    let mut bounds = RECT::default();
    unsafe {
        let framed = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let framed = framed & WS_CAPTION == WS_CAPTION || framed & WS_THICKFRAME != 0;
        let measured = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut bounds as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        ) >= 0
            || GetWindowRect(hwnd, &mut bounds) != 0;
        if measured && fills_display(bounds, monitor.rcMonitor, framed && IsZoomed(hwnd) != 0) {
            return true;
        }
        // Windows can report exclusive Direct3D mode when the composited bounds are unavailable.
        let primary = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        if MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) != primary {
            return false;
        }
        let mut state = 0;
        SHQueryUserNotificationState(&mut state) >= 0 && state == QUNS_RUNNING_D3D_FULL_SCREEN
    }
}

fn fills_display(bounds: RECT, display: RECT, framed_maximized: bool) -> bool {
    !framed_maximized
        && bounds.left <= display.left + 1
        && bounds.top <= display.top + 1
        && bounds.right >= display.right - 1
        && bounds.bottom >= display.bottom - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fullscreen_and_ignores_framed_maximized_and_other_displays() {
        let display = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(fills_display(display, display, false));
        assert!(!fills_display(display, display, true));
        assert!(!fills_display(
            RECT {
                bottom: 1032,
                ..display
            },
            display,
            false
        ));
        assert!(!fills_display(
            RECT {
                left: -1920,
                right: 0,
                ..display
            },
            display,
            false
        ));
        assert!(fills_display(
            RECT {
                right: 1919,
                ..display
            },
            display,
            false
        ));
    }

    #[test]
    fn matches_the_executable_path_case_insensitively() {
        let chosen = Path::new(r"\\?\C:\Games\Example\Game.exe");
        assert!(same_executable(
            chosen,
            Path::new(r"c:\games\example\game.exe")
        ));
        assert!(!same_executable(chosen, Path::new(r"c:\other\game.exe")));
    }
}
