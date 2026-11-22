use crate::apps;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::{
    collections::HashMap,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows::{
    core::{w, GUID, PCWSTR},
    Win32::{
        Foundation::HWND,
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationInvokePattern, TreeScope_Descendants,
                UIA_InvokePatternId,
            },
            Shell::{SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY},
        },
    },
};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_ITEMS, HWND as RawWindow, LPARAM, RECT},
    System::Registry::*,
    UI::{
        Shell::{
            ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, Shell_NotifyIconGetRect,
            NOTIFYICONIDENTIFIER,
        },
        WindowsAndMessaging::*,
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayIcon {
    id: String,
    label: String,
    icon: Option<String>,
    is_spacebar: bool,
    can_reveal: bool,
    #[serde(skip)]
    pid: u32,
    #[serde(skip)]
    path: PathBuf,
}

impl TrayIcon {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn own_process(&self) -> bool {
        self.is_spacebar
    }

    pub(crate) fn reveal(&self) -> Result<(), String> {
        if self.can_reveal {
            let path: Vec<_> = self.path.as_os_str().encode_wide().chain([0]).collect();
            unsafe {
                let item = ILCreateFromPathW(path.as_ptr());
                if !item.is_null() {
                    let status = SHOpenFolderAndSelectItems(item, 0, null(), 0);
                    ILFree(item);
                    if status >= 0 {
                        return Ok(());
                    }
                }
            }
        }
        Err("Couldn’t find this app in File Explorer.".into())
    }
}

struct RegistryKey(HKEY);

impl RegistryKey {
    fn open(parent: HKEY, name: *const u16) -> Option<Self> {
        let mut key = null_mut();
        (unsafe { RegOpenKeyExW(parent, name, 0, KEY_READ, &mut key) } == 0).then_some(Self(key))
    }

    fn value(&self, name: PCWSTR, kind: u32) -> Option<Vec<u8>> {
        let mut size = 0;
        let read = |data: *mut u8, size: &mut u32| unsafe {
            RegGetValueW(self.0, null(), name.0, kind, null_mut(), data.cast(), size)
        };
        if read(null_mut(), &mut size) != 0 || size > 192 * 1024 {
            return None;
        }
        let mut bytes = vec![0; size as usize];
        if read(bytes.as_mut_ptr(), &mut size) != 0 {
            return None;
        }
        bytes.truncate(size as usize);
        Some(bytes)
    }

    fn string(&self, name: PCWSTR) -> Option<String> {
        let data = self.value(name, RRF_RT_REG_SZ)?;
        let units: Vec<_> = data
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .take_while(|value| *value != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    fn dword(&self, name: PCWSTR) -> Option<u32> {
        self.value(name, RRF_RT_REG_DWORD)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { RegCloseKey(self.0) };
        }
    }
}

fn executable_path(value: &str) -> Option<PathBuf> {
    if !value.starts_with('{') {
        return Some(PathBuf::from(value));
    }
    let (folder, file) = value[1..].split_once("}\\")?;
    let guid = GUID::try_from(folder).ok()?;
    unsafe {
        let directory = SHGetKnownFolderPath(&guid, KF_FLAG_DONT_VERIFY, None).ok()?;
        let path = directory
            .to_string()
            .ok()
            .map(|parent| Path::new(&parent).join(file));
        CoTaskMemFree(Some(directory.0.cast()));
        path
    }
}

fn owner_windows() -> HashMap<String, Vec<(RawWindow, u32)>> {
    let mut windows = Vec::<(RawWindow, u32)>::new();
    unsafe extern "system" fn collect(hwnd: RawWindow, data: LPARAM) -> i32 {
        let mut pid = 0;
        if GetWindowThreadProcessId(hwnd, &mut pid) != 0 {
            (&mut *(data as *mut Vec<(RawWindow, u32)>)).push((hwnd, pid));
        }
        1
    }
    unsafe {
        let data = (&mut windows as *mut Vec<_>) as LPARAM;
        EnumWindows(Some(collect), data);
        let mut last = null_mut();
        loop {
            last = FindWindowExW(HWND_MESSAGE, last, null(), null());
            if last.is_null() {
                break;
            }
            collect(last, data);
        }
    }
    let mut paths = HashMap::new();
    let mut owned = HashMap::<String, Vec<_>>::new();
    for (hwnd, pid) in windows {
        if let Some(path) = paths.entry(pid).or_insert_with(|| {
            apps::process_path(pid).map(|path| path.to_string_lossy().to_lowercase())
        }) {
            owned.entry(path.clone()).or_default().push((hwnd, pid));
        }
    }
    owned
}

fn is_registered(window: RawWindow, uid: u32, guid: GUID) -> bool {
    let data = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: window,
        uID: uid,
        guidItem: windows_sys::core::GUID {
            data1: guid.data1,
            data2: guid.data2,
            data3: guid.data3,
            data4: guid.data4,
        },
    };
    unsafe { Shell_NotifyIconGetRect(&data, &mut RECT::default()) == 0 }
}

pub fn list() -> Result<Vec<TrayIcon>, String> {
    let root = RegistryKey::open(HKEY_CURRENT_USER, w!("Control Panel\\NotifyIconSettings").0)
        .ok_or("Explorer’s tray icon cache is unavailable.")?;
    let windows = owner_windows();
    let mut icons = Vec::new();
    // Explorer caches old entries too. Keep only identities the live shell recognizes.
    for index in 0..4096 {
        let mut name = [0u16; 128];
        let mut len = name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                root.0,
                index,
                name.as_mut_ptr(),
                &mut len,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != 0 {
            continue;
        }
        let Some(key) = RegistryKey::open(root.0, name.as_ptr()) else {
            continue;
        };
        let Some(path) = key
            .string(w!("ExecutablePath"))
            .and_then(|value| executable_path(&value))
        else {
            continue;
        };
        let path_key = path.to_string_lossy().to_lowercase();
        let candidates = windows
            .get(&path_key)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let pid = if let Some(guid) = key
            .string(w!("IconGuid"))
            .and_then(|value| GUID::try_from(value.trim_matches(['{', '}'])).ok())
        {
            if !is_registered(null_mut(), 0, guid) {
                continue;
            }
            candidates.first().map_or(0, |&(_, pid)| pid)
        } else {
            let Some(uid) = key.dword(w!("UID")) else {
                continue;
            };
            let Some((_, pid)) = candidates
                .iter()
                .copied()
                .find(|&(hwnd, _)| is_registered(hwnd, uid, GUID::zeroed()))
            else {
                continue;
            };
            pid
        };
        let label = key
            .string(w!("InitialTooltip"))
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| apps::app_name(&path));
        let can_reveal = path.is_absolute() && path.is_file();
        let icon = key
            .value(w!("IconSnapshot"), RRF_RT_REG_BINARY)
            .filter(|data| data.starts_with(b"\x89PNG\r\n\x1a\n"))
            .map(|data| format!("data:image/png;base64,{}", STANDARD.encode(data)))
            .or_else(|| can_reveal.then(|| apps::shell_icon(&path)).flatten());
        icons.push(TrayIcon {
            id: String::from_utf16_lossy(&name[..len as usize]),
            label,
            icon,
            is_spacebar: pid == std::process::id(),
            can_reveal,
            pid,
            path,
        });
    }
    icons.sort_by_cached_key(|entry| entry.label.to_lowercase());
    Ok(icons)
}

fn invoke_accessible_icon(label: &str) -> windows::core::Result<bool> {
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        for name in [
            w!("NotifyIconOverflowWindow"),
            w!("TopLevelWindowForOverflowXamlIsland"),
            w!("Shell_TrayWnd"),
        ] {
            let hwnd = FindWindowW(name.0, null());
            if hwnd.is_null() {
                continue;
            }
            let Ok(root) = automation.ElementFromHandle(HWND(hwnd)) else {
                continue;
            };
            let Ok(elements) =
                root.FindAll(TreeScope_Descendants, &automation.CreateTrueCondition()?)
            else {
                continue;
            };
            for i in 0..elements.Length()?.min(512) {
                let Ok(element) = elements.GetElement(i) else {
                    continue;
                };
                let name = element.CurrentName().unwrap_or_default().to_string();
                if !name.trim().eq_ignore_ascii_case(label) {
                    continue;
                }
                let class = element.CurrentClassName().unwrap_or_default().to_string();
                let automation_id = element
                    .CurrentAutomationId()
                    .unwrap_or_default()
                    .to_string();
                if class.starts_with("SystemTray.")
                    || class.contains("NotifyIcon")
                    || automation_id == "SystemTrayIcon"
                {
                    if let Ok(pattern) = element
                        .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    {
                        if pattern.Invoke().is_ok() {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}

pub fn activate(entry: &TrayIcon) -> Result<(), String> {
    if invoke_accessible_icon(&entry.label).unwrap_or(false) {
        return Ok(());
    }
    struct Search {
        pid: u32,
        window: RawWindow,
    }
    unsafe extern "system" fn find(hwnd: RawWindow, data: LPARAM) -> i32 {
        let search = &mut *(data as *mut Search);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == search.pid
            && IsWindowVisible(hwnd) != 0
            && GetWindowTextLengthW(hwnd) > 0
            && GetWindow(hwnd, GW_OWNER).is_null()
            && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize == 0
        {
            search.window = hwnd;
            0
        } else {
            1
        }
    }
    if entry.pid != 0 {
        let mut search = Search {
            pid: entry.pid,
            window: null_mut(),
        };
        unsafe { EnumWindows(Some(find), (&mut search as *mut Search) as LPARAM) };
        if !search.window.is_null() && apps::focus_window(search.window) {
            return Ok(());
        }
    }
    Err("Windows didn’t expose an action for this tray icon.".into())
}
