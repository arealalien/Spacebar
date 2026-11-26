use crate::{
    dock_config::DockStore,
    notifications::{self, BadgeSignal, Notifications},
    settings::NotificationBadges,
};
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    ptr::null_mut,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use windows::{
    core::{w, Interface, PCWSTR},
    Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationElement6, TreeScope_Descendants,
                UIA_ButtonControlTypeId,
            },
            WindowsAndMessaging::FindWindowW,
        },
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY},
    UI::WindowsAndMessaging::{EnumChildWindows, GetClassNameW},
};

#[derive(Clone)]
pub struct TaskbarButton {
    pub name: String,
    pub identity: String,
    pub notification: Option<BadgeSignal>,
}

#[derive(Clone, Default)]
pub struct TaskbarSnapshot {
    pub buttons: Vec<TaskbarButton>,
    pub notifications: Notifications,
    favorites: Arc<[u8]>,
}

pub struct TaskbarOrder(Mutex<TaskbarSnapshot>);

impl Default for TaskbarOrder {
    fn default() -> Self {
        Self(Mutex::new(TaskbarSnapshot {
            favorites: read_favorites().into(),
            ..Default::default()
        }))
    }
}

impl TaskbarOrder {
    pub fn snapshot(&self) -> TaskbarSnapshot {
        self.0
            .lock()
            .map(|buttons| buttons.clone())
            .unwrap_or_default()
    }
}

fn read_favorites() -> Vec<u8> {
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Taskband");
    let value = w!("Favorites");
    let mut size = 0;
    unsafe {
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key.0,
            value.0,
            RRF_RT_REG_BINARY,
            null_mut(),
            null_mut(),
            &mut size,
        );
        if status != 0 || size == 0 || size > 1024 * 1024 {
            return Vec::new();
        }
        let mut bytes = vec![0; size as usize];
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key.0,
            value.0,
            RRF_RT_REG_BINARY,
            null_mut(),
            bytes.as_mut_ptr().cast(),
            &mut size,
        );
        if status != 0 {
            return Vec::new();
        }
        bytes.truncate(size as usize);
        bytes
    }
}

impl TaskbarSnapshot {
    pub fn favorite_rank(&self, file_name: &OsStr) -> Option<usize> {
        if self.favorites.is_empty() {
            return None;
        }
        // Search the saved sequence for null-terminated filenames of pins we already know.
        // Taskband data may contain narrow strings or UTF-16 strings at either alignment.
        let wide: Vec<u8> = file_name
            .encode_wide()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        let wide_rank = self
            .favorites
            .windows(wide.len())
            .position(|bytes| bytes.eq_ignore_ascii_case(&wide));
        let narrow_rank = file_name
            .to_str()
            .filter(|name| name.is_ascii())
            .and_then(|name| {
                self.favorites.windows(name.len() + 1).position(|bytes| {
                    bytes.last() == Some(&0)
                        && bytes[..name.len()].eq_ignore_ascii_case(name.as_bytes())
                })
            });
        wide_rank.into_iter().chain(narrow_rank).min()
    }
}

fn read_buttons(
    automation: &IUIAutomation,
    badges: bool,
) -> windows::core::Result<Vec<TaskbarButton>> {
    // Explorer's live task buttons carry names, application IDs, and screen coordinates.
    // Read them on a dedicated COM worker to avoid blocking the dock while Explorer is busy.
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())?;
        if let Ok(entries) = read_surface(automation, taskbar, false, badges) {
            if !entries.is_empty() {
                return Ok(entries);
            }
        }

        let mut hosts: Vec<(HWND, bool)> = Vec::new();
        EnumChildWindows(
            taskbar.0,
            Some(collect_task_hosts),
            (&mut hosts as *mut Vec<(HWND, bool)>) as LPARAM,
        );
        for (host, legacy) in hosts {
            if let Ok(entries) = read_surface(
                automation,
                windows::Win32::Foundation::HWND(host),
                legacy,
                badges,
            ) {
                if !entries.is_empty() {
                    return Ok(entries);
                }
            }
        }
        Ok(Vec::new())
    }
}

unsafe extern "system" fn collect_task_hosts(hwnd: HWND, state: LPARAM) -> i32 {
    let mut buffer = [0_u16; 128];
    let length = GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
    let class = String::from_utf16_lossy(&buffer[..length]);
    let legacy = class == "MSTaskListWClass";
    if legacy
        || class == "Windows.UI.Composition.DesktopWindowContentBridge"
        || class == "XamlExplorerHostIslandWindow"
    {
        let hosts = &mut *(state as *mut Vec<(HWND, bool)>);
        hosts.push((hwnd, legacy));
    }
    1
}

fn read_surface(
    automation: &IUIAutomation,
    hwnd: windows::Win32::Foundation::HWND,
    legacy: bool,
    badges: bool,
) -> windows::core::Result<Vec<TaskbarButton>> {
    unsafe {
        let root = automation.ElementFromHandle(hwnd)?;
        let elements = root.FindAll(TreeScope_Descendants, &automation.CreateTrueCondition()?)?;
        let mut entries = Vec::new();
        for index in 0..elements.Length()?.min(2048) {
            let Ok(element) = elements.GetElement(index) else {
                continue;
            };
            let identity = element
                .CurrentAutomationId()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let class = element
                .CurrentClassName()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let identity = identity.trim();
            let app_id = identity
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("appid:"));
            let legacy_button = legacy
                && element
                    .CurrentControlType()
                    .is_ok_and(|kind| kind == UIA_ButtonControlTypeId);
            if !app_id && !legacy_button && !class.to_ascii_lowercase().contains("tasklistbutton") {
                continue;
            }
            let name = element
                .CurrentName()
                .map(|value| value.to_string())
                .unwrap_or_default();
            if name.is_empty() && identity.is_empty() {
                continue;
            }
            let bounds = element.CurrentBoundingRectangle().unwrap_or_default();
            let notification = if badges {
                [
                    element.CurrentItemStatus(),
                    element
                        .cast::<IUIAutomationElement6>()
                        .and_then(|element| element.CurrentFullDescription()),
                    element.CurrentHelpText(),
                ]
                .into_iter()
                .filter_map(Result::ok)
                .find_map(|value| BadgeSignal::from_description(&value.to_string()))
                .or_else(|| {
                    // Some Explorer versions put the badge in the accessible button name.
                    let (_, status) = name.split_once([',', '(', '–', '—'])?;
                    BadgeSignal::from_description(status)
                })
            } else {
                None
            };
            entries.push((
                (bounds.right > bounds.left).then_some(bounds.left),
                TaskbarButton {
                    name: name.to_lowercase(),
                    identity: if app_id {
                        identity[6..].trim().to_lowercase()
                    } else if identity.contains(['!', '\\']) {
                        identity.to_lowercase()
                    } else {
                        String::new()
                    },
                    notification,
                },
            ));
        }
        if entries.iter().all(|(x, _)| x.is_some()) {
            entries.sort_by_key(|(x, _)| *x);
        }
        let mut seen = std::collections::HashSet::new();
        Ok(entries
            .into_iter()
            .map(|(_, button)| button)
            .filter(|button| button.identity.is_empty() || seen.insert(button.identity.clone()))
            .collect())
    }
}

pub fn watch(app: AppHandle) {
    thread::spawn(move || unsafe {
        let automation = CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .ok()
            .and_then(|()| {
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .ok()
            });
        let mut badges_enabled_at = None;
        loop {
            let badges =
                app.state::<DockStore>().current().notification_badges != NotificationBadges::None;
            let notifications = if badges {
                let since = *badges_enabled_at.get_or_insert_with(notifications::current_filetime);
                notifications::read(since).unwrap_or_default()
            } else {
                badges_enabled_at = None;
                Notifications::default()
            };
            let buttons = automation
                .as_ref()
                .and_then(|automation| read_buttons(automation, badges).ok())
                .unwrap_or_default();
            let favorites = read_favorites();
            let order = app.state::<TaskbarOrder>();
            if let Ok(mut snapshot) = order.0.lock() {
                snapshot.notifications = notifications;
                if buttons.is_empty() {
                    for button in &mut snapshot.buttons {
                        button.notification = None;
                    }
                } else {
                    snapshot.buttons = buttons;
                }
                if !favorites.is_empty() {
                    snapshot.favorites = favorites.into();
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

fn label_score(label: &str, expected: &str) -> usize {
    if expected.is_empty() {
        return 0;
    }
    if label == expected {
        return 2000 + expected.len();
    }
    let boundary = |character: char| !character.is_alphanumeric();
    if label
        .strip_prefix(expected)
        .and_then(|suffix| suffix.chars().next())
        .is_some_and(boundary)
        || label
            .strip_suffix(expected)
            .and_then(|prefix| prefix.chars().next_back())
            .is_some_and(boundary)
    {
        return 1000 + expected.len();
    }
    0
}

impl TaskbarButton {
    pub fn score<'a>(&self, name: &str, id: &str, titles: impl Iterator<Item = &'a str>) -> usize {
        if !self.identity.is_empty() && self.identity == id {
            return 8000;
        }
        let stem = std::path::Path::new(id)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let identity_has_stem = stem.len() > 3
            && self
                .identity
                .split(|c: char| !c.is_alphanumeric())
                .any(|part| part == stem);
        let title_score = titles
            .map(|title| label_score(&self.name, &title.to_lowercase()))
            .max()
            .unwrap_or_default();
        let app_score = label_score(&self.name, &name.to_lowercase());
        if identity_has_stem {
            return 6000 + app_score;
        }
        app_score.max(title_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_application_identity_and_task_labels() {
        let button = TaskbarButton {
            name: "visual studio code (2 windows)".into(),
            identity: "microsoft.visualstudiocode.code".into(),
            notification: None,
        };
        let code = button.score(
            "Visual Studio Code",
            r"c:\apps\code.exe",
            std::iter::empty(),
        );
        let other = button.score("Visual Studio", r"c:\apps\devenv.exe", std::iter::empty());
        assert!(code > other);
        assert_eq!(
            button.score("Visual", r"c:\apps\other.exe", std::iter::empty()),
            1006
        );
        assert_eq!(
            button.score("Visualist", r"c:\apps\other.exe", std::iter::empty()),
            0
        );
    }

    #[test]
    #[ignore = "requires a Windows desktop with a configured Explorer taskbar"]
    fn reads_windows_taskbar_order() -> windows::core::Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let result = (|| {
                let automation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
                read_buttons(&automation, true)
            })();
            windows::Win32::System::Com::CoUninitialize();
            let buttons = result.unwrap_or_default();
            let favorites = read_favorites();
            let snapshot = TaskbarSnapshot {
                buttons,
                favorites: favorites.into(),
                ..Default::default()
            };
            let appdata = std::env::var_os("APPDATA").unwrap();
            let folder = std::path::PathBuf::from(appdata)
                .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
            let matched = std::fs::read_dir(folder)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| snapshot.favorite_rank(&entry.file_name()).is_some())
                .count();
            assert!(!snapshot.buttons.is_empty() || matched > 0);
            println!(
                "Read {} live task buttons and recovered the order of {} classic pins",
                snapshot.buttons.len(),
                matched
            );
        }
        Ok(())
    }

    #[test]
    fn reads_narrow_and_unaligned_utf16_pin_names() {
        let mut favorites = b"\0Browser.lnk\0\xaa".to_vec();
        favorites.extend("Editor.lnk\0".encode_utf16().flat_map(u16::to_le_bytes));
        let order = TaskbarSnapshot {
            favorites: favorites.into(),
            ..Default::default()
        };
        assert_eq!(order.favorite_rank(OsStr::new("BROWSER.lnk")), Some(1));
        assert!(order.favorite_rank(OsStr::new("Editor.lnk")) > Some(1));
        assert_eq!(order.favorite_rank(OsStr::new("Missing.lnk")), None);
    }
}
