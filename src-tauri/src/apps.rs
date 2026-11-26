use crate::{
    dock_config::DockStore,
    media::MediaIdentity,
    notifications::{current_filetime, AppNotification, MessagingApp, Notifications},
    settings::save_json,
    taskbar::{TaskbarOrder, TaskbarSnapshot},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use windows::{
    core::{Interface, GUID, PCWSTR},
    Win32::{
        Foundation::PROPERTYKEY,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
            StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
        },
        UI::{
            Controls::{IImageList, ILD_TRANSPARENT},
            Shell::{
                IShellLinkW,
                PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
                SHGetImageList, ShellLink,
            },
        },
    },
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM},
    Graphics::{
        Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
        Gdi::*,
    },
    System::Threading::{
        GetProcessTimes, IsProcessCritical, OpenProcess, QueryFullProcessImageNameW,
        TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    },
    UI::{
        Shell::{
            SHDefExtractIconW, SHGetFileInfoW, ShellExecuteW, SHFILEINFOW, SHGFI_PIDL,
            SHGFI_SYSICONINDEX, SHIL_EXTRALARGE, SHIL_JUMBO,
        },
        WindowsAndMessaging::*,
    },
};

pub(crate) const SETTINGS_APP_ID: &str = "spacebar:settings";
const STEAM_CLIENT_MODEL_ID: &str = "valve.steam.client";

#[derive(Clone, Serialize)]
pub(crate) struct WindowInfo {
    pub(crate) id: String,
    pub(crate) title: String,
    #[serde(skip)]
    pid: u32,
    #[serde(skip)]
    process_start: Option<u64>,
}

impl WindowInfo {
    pub(crate) fn handle(&self) -> Option<HWND> {
        let hwnd = self.id.parse::<usize>().ok()? as HWND;
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            (pid != 0 && pid == self.pid && IsWindowVisible(hwnd) != 0).then_some(hwnd)
        }
    }
}

#[derive(Clone)]
pub struct AppWindowActions {
    id: String,
    name: String,
    windows: Vec<WindowInfo>,
}

impl AppWindowActions {
    pub fn running(&self) -> bool {
        !self.windows.is_empty()
    }

    pub fn can_end(&self, catalog: &AppCatalog) -> bool {
        self.processes(catalog).is_ok()
    }

    pub fn close(&self) -> Result<(), String> {
        let mut windows = self
            .windows
            .iter()
            .filter_map(WindowInfo::handle)
            .peekable();
        if windows.peek().is_none() {
            return Err(format!("{} has no open windows.", self.name));
        }
        let failures = windows.filter(|&hwnd| !request_close_window(hwnd)).count();
        if failures == 0 {
            Ok(())
        } else {
            Err(format!("Windows couldn’t close all {} windows.", self.name))
        }
    }

    pub fn end(&self, catalog: &AppCatalog) -> Result<(), String> {
        let failures = self
            .processes(catalog)?
            .into_iter()
            .filter(|process| unsafe { TerminateProcess(process.0, 1) } == 0)
            .count();
        if failures == 0 {
            Ok(())
        } else {
            Err(format!("Windows couldn’t end all {} processes.", self.name))
        }
    }

    fn processes(&self, catalog: &AppCatalog) -> Result<Vec<Process>, String> {
        let data = catalog.0.lock().map_err(|error| error.to_string())?;
        let mut seen = HashSet::new();
        let mut processes = Vec::new();
        for window in self
            .windows
            .iter()
            .filter(|window| window.handle().is_some())
        {
            if !seen.insert(window.pid) {
                continue;
            }
            if window.pid == std::process::id()
                || data.apps.iter().any(|app| {
                    app.id != self.id
                        && app
                            .windows
                            .iter()
                            .any(|other| other.pid == window.pid && other.handle().is_some())
                })
            {
                return Err(format!(
                    "{} shares a process with another window or Spacebar.",
                    self.name
                ));
            }
            // Hold the process handle through termination and verify its creation time against the catalog.
            let process = Process::open(
                window.pid,
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            )
            .filter(|process| {
                window.process_start.is_some()
                    && process.started() == window.process_start
                    && process.can_end()
            })
            .ok_or_else(|| format!("Windows won’t let Spacebar end {}.", self.name))?;
            processes.push(process);
        }
        if processes.is_empty() {
            return Err(format!("{} has no open windows.", self.name));
        }
        Ok(processes)
    }
}

#[derive(Clone, Serialize)]
pub struct DockApp {
    id: String,
    name: String,
    icon: Option<String>,
    windows: Vec<WindowInfo>,
    active: bool,
    pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<AppNotification>,
    #[serde(rename = "launchPath", skip_serializing_if = "Option::is_none")]
    launch_path: Option<PathBuf>,
    #[serde(skip)]
    shortcut: Option<PathBuf>,
    #[serde(skip)]
    model_id: Option<String>,
    #[serde(skip)]
    shortcut_exe: Option<String>,
}

#[derive(Default)]
struct Catalog {
    pins: Vec<DockApp>,
    scanned_pins: Option<Instant>,
    icons: HashMap<String, String>,
    apps: Vec<DockApp>,
    toast_seen_through: HashMap<MessagingApp, i64>,
    preferences: PinPreferences,
}

pub struct AppCatalog(Mutex<Catalog>, PathBuf);

pub(crate) struct PreviewSource {
    pub(crate) windows: Vec<(WindowInfo, HWND)>,
    pub(crate) media: MediaIdentity,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct PinPreferences {
    hidden: HashSet<String>,
    added: Vec<SavedPin>,
}

#[derive(Clone, Deserialize, Serialize)]
struct SavedPin {
    id: String,
    name: String,
    path: PathBuf,
    model_id: Option<String>,
}

impl DockApp {
    fn messaging_app(&self) -> Option<MessagingApp> {
        self.model_id
            .as_deref()
            .and_then(MessagingApp::from_identity)
            .or_else(|| {
                self.launch_path
                    .as_deref()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .and_then(MessagingApp::from_executable)
            })
            .or_else(|| {
                self.shortcut_exe
                    .as_deref()
                    .and_then(MessagingApp::from_executable)
            })
    }

    fn order_id(&self) -> String {
        if let Some(shortcut) = &self.shortcut {
            return path_id(shortcut);
        }
        self.model_id.clone().unwrap_or_else(|| self.id.clone())
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn path_id(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn dock_model_id(model_id: Option<String>, executable: Option<&Path>) -> Option<String> {
    model_id.or_else(|| {
        // Keep Steam's saved slot when a window has no explicit AppUserModelID.
        executable
            .and_then(Path::file_name)
            .filter(|name| name.eq_ignore_ascii_case("steam.exe"))
            .map(|_| STEAM_CLIENT_MODEL_ID.to_owned())
    })
}

fn window_text(hwnd: HWND) -> String {
    let mut title = [0; 512];
    let size =
        unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) }.max(0) as usize;
    String::from_utf16_lossy(&title[..size])
}

struct Process(HANDLE);

impl Process {
    fn open(pid: u32, access: u32) -> Option<Self> {
        let handle = unsafe { OpenProcess(access, 0, pid) };
        (!handle.is_null()).then(|| Self(handle))
    }

    fn path(&self) -> Option<PathBuf> {
        let mut name = vec![0; 32768];
        let mut size = name.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(self.0, 0, name.as_mut_ptr(), &mut size) };
        (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&name[..size as usize])))
    }

    fn started(&self) -> Option<u64> {
        let (mut started, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let ok =
            unsafe { GetProcessTimes(self.0, &mut started, &mut exited, &mut kernel, &mut user) };
        (ok != 0)
            .then(|| (u64::from(started.dwHighDateTime) << 32) | u64::from(started.dwLowDateTime))
    }

    fn can_end(&self) -> bool {
        let mut critical = 0;
        if unsafe { IsProcessCritical(self.0, &mut critical) } == 0 || critical != 0 {
            return false;
        }
        self.path()
            .as_deref()
            .and_then(Path::file_name)
            .is_some_and(|exe| {
                !matches!(
                    exe.to_ascii_lowercase().to_str(),
                    Some(
                        "explorer.exe"
                            | "applicationframehost.exe"
                            | "dllhost.exe"
                            | "dwm.exe"
                            | "rundll32.exe"
                            | "runtimebroker.exe"
                            | "searchhost.exe"
                            | "shellexperiencehost.exe"
                            | "shellhost.exe"
                            | "sihost.exe"
                            | "startmenuexperiencehost.exe"
                            | "svchost.exe"
                            | "taskhostw.exe"
                    )
                )
            })
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(crate) fn process_path(pid: u32) -> Option<PathBuf> {
    Process::open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?.path()
}

pub(crate) fn app_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("App");
    match stem.to_ascii_lowercase().as_str() {
        "explorer" => "File Explorer",
        "chrome" => "Google Chrome",
        "msedge" => "Microsoft Edge",
        "windowsterminal" => "Windows Terminal",
        "code" => "Visual Studio Code",
        "systemsettings" => "Settings",
        "taskmgr" => "Task Manager",
        "firefox" => "Firefox",
        "notepad" => "Notepad",
        "spotify" => "Spotify",
        "discord" => "Discord",
        "webstorm64" => "WebStorm",
        "powershell" => "Windows PowerShell",
        _ => stem,
    }
    .to_owned()
}

const LARGE_ICON: i32 = 256;
const FALLBACK_ICON: i32 = 48;

fn icon_data(icon: HICON, size: i32, check_jumbo: bool) -> Option<String> {
    if icon.is_null() {
        return None;
    }
    let bytes = (size * size * 4) as usize;
    // Draw on black and white to recover alpha for both modern icons and legacy masks.
    let pixels = unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut address = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut address, null_mut(), 0);
        if bitmap.is_null() || address.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        let buffer = std::slice::from_raw_parts_mut(address.cast::<u8>(), bytes);
        buffer.fill(0);
        let black_ok = DrawIconEx(dc, 0, 0, icon, size, size, 0, null_mut(), DI_NORMAL);
        let black = buffer.to_vec();
        buffer.fill(255);
        let white_ok = DrawIconEx(dc, 0, 0, icon, size, size, 0, null_mut(), DI_NORMAL);
        let rgba = black
            .chunks_exact(4)
            .zip(buffer.chunks_exact(4))
            .flat_map(|(black, white)| {
                let alpha = 255 - white[1].saturating_sub(black[1]);
                let channel = |index: usize| {
                    if alpha == 0 {
                        0
                    } else {
                        (u32::from(black[index]) * 255 / u32::from(alpha)).min(255) as u8
                    }
                };
                [channel(2), channel(1), channel(0), alpha]
            })
            .collect::<Vec<_>>();
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        if black_ok == 0 || white_ok == 0 {
            return None;
        }
        rgba
    };
    if check_jumbo {
        // Windows can place a small legacy icon inside a mostly transparent jumbo slot.
        let mut left = size;
        let mut right = -1;
        let mut top = size;
        let mut bottom = -1;
        for (index, _) in pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[3] > 24)
        {
            let x = index as i32 % size;
            let y = index as i32 / size;
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
        }
        if (right - left).max(bottom - top) < size / 2 {
            return None;
        }
    }
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, size as u32, size as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .ok()?
        .write_image_data(&pixels)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(encoded)
    ))
}

fn resource_icon(path: &Path, index: i32) -> Option<String> {
    let path = wide(path);
    unsafe {
        let mut icon = null_mut();
        SHDefExtractIconW(
            path.as_ptr(),
            index,
            0,
            &mut icon,
            null_mut(),
            LARGE_ICON as u32,
        );
        owned_icon_data(icon, LARGE_ICON, false)
    }
}

unsafe fn owned_icon_data(icon: HICON, size: i32, check_jumbo: bool) -> Option<String> {
    if icon.is_null() {
        return None;
    }
    let data = icon_data(icon, size, check_jumbo);
    DestroyIcon(icon);
    data
}

pub(crate) fn shell_icon(path: &Path) -> Option<String> {
    let path = wide(path);
    unsafe { indexed_icon(path.as_ptr(), 0, true) }
}

pub(crate) unsafe fn shell_item_icon(item: *const u16) -> Option<String> {
    indexed_icon(item, SHGFI_PIDL, false)
}

unsafe fn indexed_icon(item: *const u16, flags: u32, prefer_jumbo: bool) -> Option<String> {
    // The base icon index has no Explorer shortcut overlay.
    let mut info = SHFILEINFOW::default();
    if SHGetFileInfoW(
        item,
        0,
        &mut info,
        std::mem::size_of::<SHFILEINFOW>() as u32,
        flags | SHGFI_SYSICONINDEX,
    ) == 0
    {
        return None;
    }
    let sizes = [(SHIL_JUMBO, LARGE_ICON), (SHIL_EXTRALARGE, FALLBACK_ICON)];
    let sizes = if prefer_jumbo { &sizes[..] } else { &sizes[1..] };
    for &(list, size) in sizes {
        if let Ok(icon) = SHGetImageList::<IImageList>(list as i32)
            .and_then(|images| images.GetIcon(info.iIcon, ILD_TRANSPARENT.0))
        {
            if let Some(data) = owned_icon_data(icon.0, size, list == SHIL_JUMBO) {
                return Some(data);
            }
        }
    }
    None
}

fn window_icon(hwnd: HWND) -> Option<String> {
    unsafe {
        for kind in [ICON_BIG, ICON_SMALL2, ICON_SMALL] {
            let mut icon = 0;
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                kind as usize,
                0,
                SMTO_ABORTIFHUNG,
                35,
                &mut icon,
            );
            if icon != 0 {
                return icon_data(icon as HICON, FALLBACK_ICON, false);
            }
        }
        icon_data(
            GetClassLongPtrW(hwnd, GCLP_HICON) as HICON,
            FALLBACK_ICON,
            false,
        )
    }
}

fn path_from_buffer(buffer: &[u16]) -> Option<PathBuf> {
    let end = buffer
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(buffer.len());
    (end > 0).then(|| PathBuf::from(std::ffi::OsString::from_wide(&buffer[..end])))
}

fn app_model_id(store: &IPropertyStore) -> Option<String> {
    const KEY: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };
    unsafe {
        let mut value = store.GetValue(&KEY).ok()?;
        let result = PropVariantToStringAlloc(&value).ok().and_then(|text| {
            let string = text.to_string().ok();
            CoTaskMemFree(Some(text.0.cast()));
            string
                .filter(|id| !id.is_empty())
                .map(|id| id.to_lowercase())
        });
        let _ = PropVariantClear(&mut value);
        result
    }
}

#[derive(Default)]
struct ShortcutDetails {
    target: Option<PathBuf>,
    icon: Option<String>,
    model_id: Option<String>,
    shortcut_exe: Option<String>,
}

fn squirrel_exe(arguments: &str) -> Option<String> {
    let mut parts = arguments.split_ascii_whitespace();
    parts.find(|part| part.eq_ignore_ascii_case("--processStart"))?;
    parts
        .next()
        .map(|part| part.trim_matches('"'))
        .filter(|part| part.to_ascii_lowercase().ends_with(".exe"))
        .map(|part| part.to_lowercase())
}

fn shortcut_details(path: &Path) -> ShortcutDetails {
    let filename = wide(path);
    let details = unsafe {
        (|| -> windows::core::Result<_> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.cast::<IPersistFile>()?
                .Load(PCWSTR(filename.as_ptr()), STGM_READ)?;
            let mut buffer = vec![0; 32768];
            let target = link
                .GetPath(&mut buffer, null_mut(), 0)
                .ok()
                .and_then(|()| path_from_buffer(&buffer));
            buffer.fill(0);
            let mut icon_index = 0;
            let icon = link
                .GetIconLocation(&mut buffer, &mut icon_index)
                .ok()
                .and_then(|()| path_from_buffer(&buffer))
                .and_then(|resource| resource_icon(&resource, icon_index))
                .or_else(|| {
                    target
                        .as_deref()
                        .and_then(|path| resource_icon(path, 0).or_else(|| shell_icon(path)))
                })
                .or_else(|| {
                    let item = link.GetIDList().ok()?;
                    if item.is_null() {
                        return None;
                    }
                    let image = indexed_icon(item.cast(), SHGFI_PIDL, true);
                    CoTaskMemFree(Some(item.cast()));
                    image
                });
            let model_id = link
                .cast::<IPropertyStore>()
                .ok()
                .and_then(|store| app_model_id(&store));
            buffer.fill(0);
            let shortcut_exe = link.GetArguments(&mut buffer).ok().and_then(|()| {
                let end = buffer
                    .iter()
                    .position(|&character| character == 0)
                    .unwrap_or(buffer.len());
                squirrel_exe(&String::from_utf16_lossy(&buffer[..end]))
            });
            Ok(ShortcutDetails {
                target,
                icon,
                model_id,
                shortcut_exe,
            })
        })()
    }
    .unwrap_or_default();
    ShortcutDetails {
        icon: details.icon.or_else(|| shell_icon(path)),
        ..details
    }
}

struct ShellApartment(bool);

impl ShellApartment {
    fn enter() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for ShellApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn pinned_apps() -> Vec<DockApp> {
    let Some(root) = env::var_os("APPDATA") else {
        return Vec::new();
    };
    let folder =
        PathBuf::from(root).join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut links: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("lnk"))
        })
        .collect();
    links.sort_by_key(|path| path_id(path));
    links
        .into_iter()
        .map(|path| {
            let ShortcutDetails {
                target,
                icon,
                model_id,
                shortcut_exe,
            } = shortcut_details(&path);
            let model_id = dock_model_id(model_id, target.as_deref());
            DockApp {
                id: path_id(target.as_deref().unwrap_or(&path)),
                name: path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                icon,
                windows: Vec::new(),
                active: false,
                pinned: true,
                notification: None,
                launch_path: target,
                shortcut: Some(path),
                model_id,
                shortcut_exe,
            }
        })
        .collect()
}

unsafe extern "system" fn collect_window(hwnd: HWND, state: LPARAM) -> i32 {
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    let styles = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if styles & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 && styles & WS_EX_APPWINDOW == 0 {
        return 1;
    }
    if !GetWindow(hwnd, GW_OWNER).is_null() && styles & WS_EX_APPWINDOW == 0 {
        return 1;
    }
    if hwnd == GetShellWindow() {
        return 1;
    }
    let mut cloaked: u32 = 0;
    DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED as u32,
        (&mut cloaked as *mut u32).cast(),
        4,
    );
    if cloaked != 0 {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 || pid == std::process::id() {
        return 1;
    }
    let title = window_text(hwnd);
    if title.trim().is_empty() || title == "Program Manager" {
        return 1;
    }
    let windows = &mut *(state as *mut Vec<(HWND, u32, String)>);
    windows.push((hwnd, pid, title));
    1
}

impl AppCatalog {
    pub fn load(app: &AppHandle) -> tauri::Result<Self> {
        let path = app.path().app_config_dir()?.join("pins.json");
        let preferences = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Ok(Self(
            Mutex::new(Catalog {
                preferences,
                ..Default::default()
            }),
            path,
        ))
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0
            .lock()
            .is_ok_and(|catalog| catalog.apps.iter().any(|app| app.id == id))
    }

    pub(crate) fn preview_source(&self, app_id: &str) -> Result<PreviewSource, String> {
        let catalog = self.0.lock().map_err(|error| error.to_string())?;
        let app = catalog
            .apps
            .iter()
            .find(|app| app.id == app_id)
            .ok_or("This app is no longer in the dock.")?;
        let windows = app
            .windows
            .iter()
            .filter_map(|window| {
                let hwnd = window.handle()?;
                Some((window.clone(), hwnd))
            })
            .collect();
        let executable = app.launch_path.as_ref().filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
                && !catalog.apps.iter().any(|other| {
                    other.id != app.id
                        && !other.windows.is_empty()
                        && other
                            .launch_path
                            .as_deref()
                            .and_then(Path::file_name)
                            .zip(path.file_name())
                            .is_some_and(|(other, name)| other.eq_ignore_ascii_case(name))
                })
        });
        Ok(PreviewSource {
            windows,
            media: MediaIdentity {
                model_id: app.model_id.clone(),
                executable: executable.cloned(),
            },
        })
    }

    pub(crate) fn window_actions(&self, id: &str) -> Option<AppWindowActions> {
        let mut app = self.get_app(id).ok()?;
        app.windows.retain(|window| window.handle().is_some());
        Some(AppWindowActions {
            id: app.id,
            name: app.name,
            windows: app.windows,
        })
    }

    fn refresh(
        &self,
        taskbar: &TaskbarSnapshot,
        settings: Option<HWND>,
        app_order: &[String],
    ) -> Result<Vec<DockApp>, String> {
        let _apartment = ShellApartment::enter();
        let mut catalog = self.0.lock().map_err(|error| error.to_string())?;
        if catalog
            .scanned_pins
            .is_none_or(|time| time.elapsed() > Duration::from_secs(10))
        {
            catalog.pins = pinned_apps();
            catalog.scanned_pins = Some(Instant::now());
        }
        let mut apps = catalog.pins.clone();
        let mut ids = HashSet::new();
        apps.retain(|app| ids.insert(app.id.clone()));
        let custom = catalog.preferences.added.clone();
        for pin in custom.iter().filter(|pin| ids.insert(pin.id.clone())) {
            let icon = catalog
                .icons
                .get(&pin.id)
                .cloned()
                .or_else(|| resource_icon(&pin.path, 0).or_else(|| shell_icon(&pin.path)));
            if let Some(icon) = &icon {
                catalog.icons.insert(pin.id.clone(), icon.clone());
            }
            apps.push(DockApp {
                id: pin.id.clone(),
                name: pin.name.clone(),
                icon,
                windows: Vec::new(),
                active: false,
                pinned: true,
                notification: None,
                launch_path: Some(pin.path.clone()),
                shortcut: None,
                model_id: dock_model_id(pin.model_id.clone(), Some(&pin.path)),
                shortcut_exe: None,
            });
        }
        let pinned = apps.len();
        let mut windows: Vec<(HWND, u32, String)> = Vec::new();
        let foreground = unsafe {
            EnumWindows(
                Some(collect_window),
                (&mut windows as *mut Vec<(HWND, u32, String)>) as LPARAM,
            );
            GetForegroundWindow()
        };
        let mut processes = HashMap::new();
        for (hwnd, pid, title) in windows {
            let (path, started) = processes.entry(pid).or_insert_with(|| {
                Process::open(pid, PROCESS_QUERY_LIMITED_INFORMATION)
                    .map_or((None, None), |process| (process.path(), process.started()))
            });
            let path = path.as_deref();
            let host = path
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("ApplicationFrameHost.exe"));
            let id = if host {
                format!("frame:{}", hwnd as usize)
            } else {
                path.map(path_id)
                    .unwrap_or_else(|| format!("process:{pid}"))
            };
            let model_id = unsafe {
                SHGetPropertyStoreForWindow::<IPropertyStore>(windows::Win32::Foundation::HWND(
                    hwnd,
                ))
            }
            .ok()
            .and_then(|store| app_model_id(&store));
            let model_id = dock_model_id(model_id, path);
            let name = if host {
                title.clone()
            } else {
                path.map(app_name).unwrap_or_else(|| title.clone())
            };
            let exe = path
                .and_then(Path::file_name)
                .and_then(|name| name.to_str());
            let index = model_id
                .as_ref()
                .and_then(|identity| {
                    apps.iter()
                        .position(|app| app.model_id.as_ref() == Some(identity))
                })
                .or_else(|| apps.iter().position(|app| app.id == id))
                .or_else(|| {
                    apps[..pinned].iter().position(|pin| {
                        pin.shortcut_exe
                            .as_deref()
                            .zip(exe)
                            .is_some_and(|(target, exe)| target.eq_ignore_ascii_case(exe))
                            || (!(pin.model_id.is_some() && model_id.is_some())
                                && pin.name.eq_ignore_ascii_case(&name))
                    })
                })
                .unwrap_or_else(|| {
                    let icon = catalog
                        .icons
                        .get(&id)
                        .cloned()
                        .or_else(|| {
                            path.filter(|_| !host).and_then(|path| {
                                resource_icon(path, 0).or_else(|| shell_icon(path))
                            })
                        })
                        .or_else(|| window_icon(hwnd));
                    if let Some(icon) = &icon {
                        catalog.icons.insert(id.clone(), icon.clone());
                    }
                    apps.push(DockApp {
                        id,
                        name,
                        icon,
                        windows: Vec::new(),
                        active: false,
                        pinned: false,
                        notification: None,
                        launch_path: path.map(Path::to_path_buf),
                        shortcut: None,
                        model_id: model_id.clone(),
                        shortcut_exe: None,
                    });
                    apps.len() - 1
                });
            let app = &mut apps[index];
            if app.model_id.is_none() {
                app.model_id = model_id;
            }
            if let Some(path) = path.filter(|_| !host) {
                app.launch_path = Some(path.to_path_buf());
            }
            let window = WindowInfo {
                id: (hwnd as usize).to_string(),
                title,
                pid,
                process_start: *started,
            };
            if hwnd == foreground {
                app.windows.insert(0, window);
                app.active = true;
            } else {
                app.windows.push(window);
            }
        }
        if let Some(hwnd) = settings.filter(|hwnd| unsafe { IsWindowVisible(*hwnd) } != 0) {
            let settings_window = WindowInfo {
                id: (hwnd as usize).to_string(),
                title: "Spacebar Settings".into(),
                pid: std::process::id(),
                process_start: None,
            };
            let own_id = env::current_exe().ok().map(|path| path_id(&path));
            let entry = own_id
                .as_ref()
                .and_then(|id| apps.iter_mut().find(|app| app.pinned && app.id == *id));
            if let Some(entry) = entry {
                entry.windows.push(settings_window);
                entry.active = hwnd == foreground;
            } else {
                let icon = catalog
                    .icons
                    .entry(SETTINGS_APP_ID.into())
                    .or_insert_with(|| {
                        format!(
                            "data:image/png;base64,{}",
                            STANDARD.encode(include_bytes!("../icons/icon.png"))
                        )
                    })
                    .clone();
                apps.push(DockApp {
                    id: SETTINGS_APP_ID.into(),
                    name: "Spacebar Settings".into(),
                    icon: Some(icon),
                    windows: vec![settings_window],
                    active: hwnd == foreground,
                    pinned: false,
                    notification: None,
                    launch_path: None,
                    shortcut: None,
                    model_id: None,
                    shortcut_exe: None,
                });
            }
        }
        let previous_order: HashMap<_, _> = catalog
            .apps
            .iter()
            .enumerate()
            .map(|(index, app)| (app.id.as_str(), index))
            .collect();
        apps[pinned..].sort_by_cached_key(|app| {
            if let Some(&index) = previous_order.get(app.id.as_str()) {
                return (0, index as u64);
            }
            // Before Explorer exposes an order, foreground/Z-order isn't a stable launch order.
            let started = app
                .windows
                .iter()
                .filter_map(|window| window.process_start)
                .min();
            (1, started.unwrap_or(u64::MAX))
        });
        apps.sort_by_cached_key(|app| {
            let favorite = app
                .shortcut
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|name| taskbar.favorite_rank(name));
            if let Some(rank) = favorite {
                return (0, rank);
            }
            if app.shortcut.is_some() {
                return (1, 0);
            }
            if let Some(rank) = custom.iter().position(|pin| pin.id == app.id) {
                return (2, rank);
            }
            (3, 0)
        });
        if let Some(kind) = apps
            .iter()
            .find(|app| app.active)
            .and_then(DockApp::messaging_app)
        {
            catalog.toast_seen_through.insert(kind, current_filetime());
        }
        for app in &mut apps {
            let messaging_app = app.messaging_app();
            let toast = messaging_app.filter(|_| !app.active).and_then(|kind| {
                let seen_through = catalog
                    .toast_seen_through
                    .get(&kind)
                    .copied()
                    .unwrap_or_default();
                taskbar.notifications.toast_badge(kind, seen_through)
            });
            app.notification =
                notification_for_app(app, messaging_app, &taskbar.notifications).or(toast);
        }
        order_by_live_buttons(&mut apps, taskbar);
        order_by_saved_slots(&mut apps, app_order);
        apps.retain_mut(|app| {
            if catalog.preferences.hidden.contains(&app.id) {
                app.pinned = false;
                return !app.windows.is_empty();
            }
            true
        });
        catalog
            .icons
            .retain(|id, _| apps.iter().any(|app| &app.id == id));
        catalog.apps = apps.clone();
        Ok(apps)
    }
}

fn order_by_saved_slots(apps: &mut [DockApp], app_order: &[String]) {
    if app_order.is_empty() {
        return;
    }
    let ranks: HashMap<_, _> = app_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    apps.sort_by_cached_key(|app| {
        ranks
            .get(app.order_id().as_str())
            .or_else(|| ranks.get(app.id.as_str()))
            .or_else(|| app.model_id.as_deref().and_then(|id| ranks.get(id)))
            .copied()
            .unwrap_or(usize::MAX)
    });
}

fn notification_for_app(
    app: &DockApp,
    messaging_app: Option<MessagingApp>,
    notifications: &Notifications,
) -> Option<AppNotification> {
    [app.model_id.as_deref(), Some(app.id.as_str())]
        .into_iter()
        .flatten()
        .find_map(|identity| notifications.badges.get(identity).copied())
        .or_else(|| messaging_app.and_then(|app| notifications.messaging_badge(app)))
}

fn order_by_live_buttons(apps: &mut [DockApp], taskbar: &TaskbarSnapshot) {
    let mut matches = Vec::new();
    for (button_index, button) in taskbar.buttons.iter().enumerate() {
        let messaging_app = MessagingApp::from_identity(&button.identity);
        for (app_index, app) in apps.iter().enumerate() {
            let score = if !button.identity.is_empty()
                && app.model_id.as_deref() == Some(button.identity.as_str())
            {
                9000
            } else if messaging_app.is_some() && app.messaging_app() == messaging_app {
                8500
            } else {
                button.score(
                    &app.name,
                    &app.id,
                    app.windows.iter().map(|window| window.title.as_str()),
                )
            };
            if score > 0 {
                matches.push((std::cmp::Reverse(score), button_index, app_index));
            }
        }
    }
    matches.sort_unstable();
    let mut used_buttons = HashSet::new();
    let mut used_apps = HashSet::new();
    let mut assigned = Vec::new();
    for (_, button, app) in matches {
        if !used_buttons.contains(&button) && used_apps.insert(app) {
            used_buttons.insert(button);
            let task = &taskbar.buttons[button];
            apps[app].notification = match task.notification {
                Some(signal) => signal.notification(),
                None => taskbar
                    .notifications
                    .badges
                    .get(&task.identity)
                    .copied()
                    .or(apps[app].notification),
            };
            assigned.push((button, app));
        }
    }
    // Keep fallback positions for entries Explorer didn't expose, including local pins.
    assigned.sort_unstable();
    let slots = (0..apps.len()).filter(|index| used_apps.contains(index));
    let mut positions: HashMap<_, _> = apps
        .iter()
        .enumerate()
        .map(|(index, app)| (app.id.clone(), index))
        .collect();
    for (slot, (_, app)) in slots.zip(assigned) {
        positions.insert(apps[app].id.clone(), slot);
    }
    apps.sort_by_cached_key(|app| positions[&app.id]);
}

#[cfg(test)]
#[test]
fn steam_keeps_its_saved_slot_with_or_without_a_window_id_or_pin() {
    let executable = Path::new(r"C:\Program Files (x86)\Steam\Steam.exe");
    let mut apps: Vec<_> = ["browser", "editor", "steam"]
        .into_iter()
        .map(|id| DockApp {
            id: id.into(),
            name: id.into(),
            icon: None,
            windows: Vec::new(),
            active: false,
            pinned: false,
            notification: None,
            launch_path: None,
            shortcut: None,
            model_id: None,
            shortcut_exe: None,
        })
        .collect();
    let saved = ["browser", STEAM_CLIENT_MODEL_ID, "editor"].map(String::from);
    for reported in [None, Some(STEAM_CLIENT_MODEL_ID.to_owned())] {
        let steam = apps.iter_mut().find(|app| app.name == "steam").unwrap();
        steam.id = path_id(executable);
        steam.model_id = dock_model_id(reported, Some(executable));
        order_by_saved_slots(&mut apps, &saved);
        assert_eq!(apps[1].name, "steam");
        apps[1].shortcut = Some(PathBuf::from(r"C:\TaskBar\Steam.lnk"));
        apps[1].pinned = true;
        order_by_saved_slots(&mut apps, &saved);
        assert_eq!(apps[1].name, "steam");
        apps[1].shortcut = None;
        apps[1].pinned = false;
    }
    assert_eq!(
        dock_model_id(Some("other.app".into()), Some(executable)).as_deref(),
        Some("other.app")
    );
    assert_eq!(dock_model_id(None, Some(Path::new("steamwebhelper.exe"))), None);
}

#[cfg(test)]
#[test]
fn live_taskbar_order_takes_priority_over_saved_pin_positions() {
    let mut apps: Vec<_> = [
        ("Browser", "browser"),
        ("Local pin", "local"),
        ("Editor", "editor"),
    ]
    .into_iter()
    .map(|(name, id)| DockApp {
        id: id.into(),
        name: name.into(),
        icon: None,
        windows: Vec::new(),
        active: false,
        pinned: true,
        notification: None,
        launch_path: None,
        shortcut: None,
        model_id: Some(id.into()),
        shortcut_exe: None,
    })
    .collect();
    let mut taskbar = TaskbarSnapshot::default();
    taskbar.buttons = [("Editor", "editor"), ("Browser", "browser")]
        .into_iter()
        .map(|(name, identity)| crate::taskbar::TaskbarButton {
            name: name.into(),
            identity: identity.into(),
            notification: None,
        })
        .collect();
    order_by_live_buttons(&mut apps, &taskbar);
    assert_eq!(
        apps.iter().map(|app| app.id.as_str()).collect::<Vec<_>>(),
        ["editor", "local", "browser"]
    );
}

#[tauri::command]
pub async fn list_dock_apps(app: AppHandle) -> Result<Vec<DockApp>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let taskbar = app.state::<TaskbarOrder>().snapshot();
        let settings = crate::desktop::window(&app, "settings")
            .and_then(|window| crate::desktop::handle(&window))
            .ok();
        let app_order = app.state::<DockStore>().app_order();
        app.state::<AppCatalog>()
            .refresh(&taskbar, settings, &app_order)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn move_dock_app(
    app: AppHandle,
    catalog: State<'_, AppCatalog>,
    store: State<'_, DockStore>,
    id: String,
    anchor: String,
    before: bool,
) -> Result<(), String> {
    let data = catalog.0.lock().map_err(|error| error.to_string())?;
    let order_id = |id: &str| {
        data.apps
            .iter()
            .find(|app| app.id == id)
            .map(DockApp::order_id)
            .ok_or("That app is no longer on the dock.")
    };
    let source = order_id(&id)?;
    let anchor = order_id(&anchor)?;
    let visible: Vec<_> = data.apps.iter().map(DockApp::order_id).collect();
    drop(data);
    store.move_app(&visible, &source, &anchor, before)?;
    let _ = app.emit("dock-apps-changed", ());
    Ok(())
}

#[tauri::command]
pub fn set_app_pinned(
    app: AppHandle,
    catalog: State<'_, AppCatalog>,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    let mut data = catalog.0.lock().map_err(|error| error.to_string())?;
    let item = data
        .apps
        .iter()
        .find(|item| item.id == id)
        .ok_or("This app is no longer in the dock.")?;
    if item.pinned == pinned {
        return Ok(());
    }
    let mut next = data.preferences.clone();
    if pinned {
        next.hidden.remove(&id);
        if item.shortcut.is_none() {
            let path = item
                .launch_path
                .clone()
                .ok_or("This app has no available launcher.")?;
            next.added.push(SavedPin {
                id: id.clone(),
                name: item.name.clone(),
                path,
                model_id: item.model_id.clone(),
            });
        }
    } else if item.shortcut.is_some() {
        next.hidden.insert(id.clone());
    } else {
        next.added.retain(|pin| pin.id != id);
    }
    save_json(&catalog.1, &next)?;
    data.preferences = next;
    if let Some(item) = data.apps.iter_mut().find(|item| item.id == id) {
        item.pinned = pinned;
    }
    let _ = app.emit("dock-apps-changed", ());
    Ok(())
}

pub fn focus_window(hwnd: HWND) -> bool {
    unsafe {
        let restoring = IsIconic(hwnd) != 0 && ShowWindowAsync(hwnd, SW_RESTORE) != 0;
        SetForegroundWindow(hwnd) != 0 || restoring
    }
}

pub(crate) fn request_close_window(hwnd: HWND) -> bool {
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 }
}

pub(crate) fn launch_file(path: &Path, arguments: Option<&str>) -> bool {
    let _apartment = ShellApartment::enter();
    let file = wide(path);
    let arguments = arguments.map(|value| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            null(),
            file.as_ptr(),
            arguments.as_ref().map_or(null(), |value| value.as_ptr()),
            null(),
            SW_SHOWNORMAL,
        )
    };
    result as isize > 32
}

fn window_launch_arguments(exe: &str) -> Option<&'static str> {
    match exe {
        "explorer.exe" => Some("/n"),
        "chrome.exe" | "msedge.exe" | "brave.exe" | "vivaldi.exe" | "chromium.exe" => {
            Some("--new-window")
        }
        "firefox.exe" | "zen.exe" | "librewolf.exe" | "waterfox.exe" => {
            Some("--new-window about:home")
        }
        _ => None,
    }
}

impl AppCatalog {
    fn get_app(&self, id: &str) -> Result<DockApp, String> {
        self.0
            .lock()
            .map_err(|error| error.to_string())?
            .apps
            .iter()
            .find(|app| app.id == id)
            .cloned()
            .ok_or_else(|| "This app is no longer in the dock.".into())
    }
}

#[tauri::command]
pub fn activate_app(
    handle: AppHandle,
    catalog: State<'_, AppCatalog>,
    id: String,
    toggle: Option<bool>,
) -> Result<(), String> {
    crate::system_panel::close_spacebar_start(&handle);
    let foreground = if toggle == Some(true) {
        crate::desktop::foreground_for_dock(&handle)
    } else {
        unsafe { GetForegroundWindow() }
    };
    let owner = unsafe { GetAncestor(foreground, GA_ROOTOWNER) };
    crate::previews::close_all(&handle);
    let app = match catalog.get_app(&id) {
        Err(_) if id == SETTINGS_APP_ID => return crate::desktop::open_settings(handle, None),
        result => result?,
    };
    let windows: Vec<_> = app.windows.iter().filter_map(WindowInfo::handle).collect();
    if toggle == Some(true) {
        if let Some(&active) = windows
            .iter()
            .find(|&&hwnd| hwnd == foreground || hwnd == owner)
        {
            if unsafe { IsIconic(active) == 0 } {
                return if unsafe { ShowWindowAsync(active, SW_MINIMIZE) } != 0 {
                    Ok(())
                } else {
                    Err(format!("Windows couldn’t minimize {}.", app.name))
                };
            }
        }
    }
    if let Some(&hwnd) = windows.first() {
        return if focus_window(hwnd) {
            Ok(())
        } else {
            Err(format!("Windows couldn’t focus {}.", app.name))
        };
    }
    if id == SETTINGS_APP_ID
        || app
            .windows
            .iter()
            .any(|window| window.pid == std::process::id())
    {
        return crate::desktop::open_settings(handle, None);
    }
    if app
        .shortcut
        .as_deref()
        .or(app.launch_path.as_deref())
        .is_some_and(|path| launch_file(path, None))
    {
        return Ok(());
    }
    Err(format!("Couldn’t open {}.", app.name))
}

#[tauri::command]
pub fn launch_app(
    handle: AppHandle,
    catalog: State<'_, AppCatalog>,
    id: String,
) -> Result<(), String> {
    crate::system_panel::close_spacebar_start(&handle);
    crate::previews::close_all(&handle);
    if id == SETTINGS_APP_ID {
        return crate::desktop::open_settings(handle, None);
    }
    let app = catalog.get_app(&id)?;
    if app
        .windows
        .iter()
        .any(|window| window.pid == std::process::id())
    {
        return crate::desktop::open_settings(handle, None);
    }
    let existing = app.windows.iter().find_map(WindowInfo::handle);
    let exe = app
        .launch_path
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_ascii_lowercase());
    if existing.is_some() {
        if let Some((path, arguments)) = app
            .launch_path
            .as_deref()
            .zip(exe.as_deref().and_then(window_launch_arguments))
        {
            if launch_file(path, Some(arguments)) {
                return Ok(());
            }
        }
    }
    let single_instance = exe.as_deref().is_some_and(|name| {
        name == "spotify.exe"
            || name == "discord.exe"
            || name == "discordptb.exe"
            || name == "discordcanary.exe"
    });
    if !(single_instance && existing.is_some())
        && app
            .shortcut
            .as_deref()
            .or(app.launch_path.as_deref())
            .is_some_and(|path| launch_file(path, None))
    {
        return Ok(());
    }
    if existing.is_some_and(focus_window) {
        return Ok(());
    }
    Err(format!("Couldn’t open {}.", app.name))
}
