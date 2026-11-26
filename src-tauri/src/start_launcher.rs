use crate::{
    apps::{launch_file, shell_item_icon},
    desktop,
    dock_config::DockStore,
    system_panel::{close_spacebar_start, SystemPanel},
    system_widgets::{press_keys, shell_worker, Com},
};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    ptr::copy_nonoverlapping,
    slice,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use windows::{
    core::{Interface, PCSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Com::{CoTaskMemAlloc, CoTaskMemFree, COINIT_APARTMENTTHREADED},
        UI::Shell::{
            BHID_EnumItems, BHID_SFUIObject, DefSubclassProc, FOLDERID_AppsFolder,
            GetWindowSubclass, IContextMenu, IContextMenu2, IContextMenu3, IEnumShellItems,
            ILGetSize, IShellItem, RemoveWindowSubclass, SHCreateItemFromIDList,
            SHGetIDListFromObject, SHGetKnownFolderItem, SetWindowSubclass, CMF_NORMAL,
            CMINVOKECOMMANDINFO, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY,
        },
        UI::WindowsAndMessaging::{
            CreatePopupMenu, DestroyMenu, GetMenuItemCount, TrackPopupMenuEx, HMENU, TPM_RETURNCMD,
            TPM_RIGHTBUTTON, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
        },
    },
};
use windows_sys::Win32::{
    System::{SystemInformation::GetSystemDirectoryW, Threading::CREATE_NO_WINDOW},
    UI::{
        Input::KeyboardAndMouse::VK_LWIN,
        Shell::{
            ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_IDLIST, SEE_MASK_NOASYNC,
            SHELLEXECUTEINFOW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

#[derive(Clone, Serialize)]
pub(crate) struct StartApp {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip)]
    target: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct StartApps(Mutex<Option<(Instant, Vec<StartApp>)>>);

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum StartAppLayout {
    #[default]
    List,
    Grid,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct StartPreferences {
    app_layout: StartAppLayout,
}

fn start_is_open(source: &WebviewWindow) -> Result<(), String> {
    if source.label() == "system" && source.state::<SystemPanel>().start_active() {
        Ok(())
    } else {
        Err("Open Spacebar Start to use this action.".into())
    }
}

#[tauri::command]
pub fn read_start_preferences(
    source: WebviewWindow,
    store: State<'_, DockStore>,
) -> Result<StartPreferences, String> {
    start_is_open(&source)?;
    store.start_preferences()
}

#[tauri::command]
pub fn save_start_preferences(
    source: WebviewWindow,
    store: State<'_, DockStore>,
    preferences: StartPreferences,
) -> Result<StartPreferences, String> {
    start_is_open(&source)?;
    store.save_start_preferences(preferences)
}

fn shell_name(item: &IShellItem) -> Option<String> {
    let text = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()? };
    let value = unsafe { text.to_string() };
    unsafe { CoTaskMemFree(Some(text.0.cast())) };
    value.ok().filter(|text| !text.trim().is_empty())
}

fn shell_app(item: &IShellItem, name: String) -> Option<StartApp> {
    unsafe {
        let pidl = SHGetIDListFromObject(item).ok()?;
        if pidl.is_null() {
            return None;
        }
        let size = ILGetSize(Some(pidl)) as usize;
        let app = (size > 2).then(|| {
            let target = slice::from_raw_parts(pidl.cast(), size).to_vec();
            StartApp {
                id: STANDARD_NO_PAD.encode(&target),
                name,
                icon: shell_item_icon(pidl.cast()),
                target,
            }
        });
        CoTaskMemFree(Some(pidl.cast()));
        app
    }
}

fn installed_apps() -> Result<Vec<StartApp>, String> {
    unsafe {
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
            .map_err(|error| error.to_string())?;
        let entries: IEnumShellItems = folder
            .BindToHandler(None, &BHID_EnumItems)
            .map_err(|error| error.to_string())?;
        let mut apps = Vec::new();
        let mut seen = HashSet::new();
        loop {
            let mut next = [None];
            entries
                .Next(&mut next, None)
                .map_err(|error| error.to_string())?;
            let [Some(item)] = next else { break };
            if let Some(app) = shell_name(&item).and_then(|name| shell_app(&item, name)) {
                if seen.insert(app.id.clone()) {
                    apps.push(app);
                }
            }
        }
        Ok(apps)
    }
}

#[tauri::command]
pub async fn read_start_apps(
    source: WebviewWindow,
    state: State<'_, StartApps>,
) -> Result<Vec<StartApp>, String> {
    start_is_open(&source)?;
    if let Some((_, apps)) = state
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .filter(|(read, _)| read.elapsed() < Duration::from_secs(30))
    {
        return Ok(apps.clone());
    }
    let apps = shell_worker(installed_apps).await?;
    *state.0.lock().map_err(|error| error.to_string())? = Some((Instant::now(), apps.clone()));
    Ok(apps)
}

fn launch_shell_app(target: &[u8]) -> Result<(), String> {
    unsafe {
        let item = CoTaskMemAlloc(target.len());
        if item.is_null() {
            return Err("Couldn’t allocate the Windows app identifier.".into());
        }
        copy_nonoverlapping(target.as_ptr(), item.cast(), target.len());
        let mut command = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpIDList: item,
            nShow: SW_SHOWNORMAL,
            ..Default::default()
        };
        let launched = ShellExecuteExW(&mut command) != 0;
        CoTaskMemFree(Some(item));
        launched
            .then_some(())
            .ok_or_else(|| "Windows couldn’t open this app.".into())
    }
}

#[tauri::command]
pub async fn launch_start_app(
    app: AppHandle,
    source: WebviewWindow,
    state: State<'_, StartApps>,
    id: String,
) -> Result<(), String> {
    start_is_open(&source)?;
    let target = start_target(&state, &id)?;
    shell_worker(move || launch_shell_app(&target)).await?;
    close_spacebar_start(&app);
    Ok(())
}

fn start_target(state: &StartApps, id: &str) -> Result<Vec<u8>, String> {
    state
        .0
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .and_then(|(_, apps)| apps.iter().find(|app| app.id == id))
        .map(|entry| entry.target.clone())
        .ok_or_else(|| "This app is no longer in the Start menu.".into())
}

struct ShellMenu(HMENU);

impl Drop for ShellMenu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.0) };
    }
}

const MENU_SUBCLASS: usize = 0x53424150;

struct ShellMenuMessages {
    extended: Option<IContextMenu3>,
    legacy: Option<IContextMenu2>,
}

unsafe extern "system" fn forward_shell_menu(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    if matches!(
        message,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        let handler = &*(data as *const ShellMenuMessages);
        let mut result = LRESULT(isize::from(matches!(message, WM_DRAWITEM | WM_MEASUREITEM)));
        if let Some(menu) = &handler.extended {
            if menu
                .HandleMenuMsg2(message, wparam, lparam, Some(&mut result))
                .is_ok()
            {
                return result;
            }
        } else if message != WM_MENUCHAR {
            if let Some(menu) = &handler.legacy {
                if menu.HandleMenuMsg(message, wparam, lparam).is_ok() {
                    return result;
                }
            }
        }
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

fn show_shell_menu(target: &[u8], owner: HWND, x: i32, y: i32) -> windows::core::Result<bool> {
    let _com = Com::enter(COINIT_APARTMENTTHREADED)?;
    unsafe {
        let mut existing = 0;
        if GetWindowSubclass(
            owner,
            Some(forward_shell_menu),
            MENU_SUBCLASS,
            Some(&mut existing),
        )
        .as_bool()
        {
            return Ok(false);
        }
        let commands = shell_commands(target)?;
        let menu = ShellMenu(CreatePopupMenu()?);
        let first = 1;
        commands
            .QueryContextMenu(menu.0, 0, first, 0x7fff, CMF_NORMAL)
            .ok()?;
        if GetMenuItemCount(Some(menu.0)) <= 0 {
            return Ok(false);
        }
        let messages = ShellMenuMessages {
            extended: commands.cast().ok(),
            legacy: commands.cast().ok(),
        };
        if !SetWindowSubclass(
            owner,
            Some(forward_shell_menu),
            MENU_SUBCLASS,
            (&messages as *const ShellMenuMessages) as usize,
        )
        .as_bool()
        {
            return Err(windows::core::Error::from_win32());
        }
        let choice = TrackPopupMenuEx(
            menu.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            x,
            y,
            owner,
            None,
        )
        .0 as u32;
        let _ = RemoveWindowSubclass(owner, Some(forward_shell_menu), MENU_SUBCLASS);
        if choice < first {
            return Ok(false);
        }
        commands.InvokeCommand(&CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: owner,
            lpVerb: PCSTR((choice - first) as usize as *const u8),
            nShow: SW_SHOWNORMAL,
            ..Default::default()
        })?;
        Ok(true)
    }
}

fn shell_commands(target: &[u8]) -> windows::core::Result<IContextMenu> {
    unsafe {
        let item: IShellItem = SHCreateItemFromIDList(target.as_ptr().cast())?;
        item.BindToHandler(None, &BHID_SFUIObject)
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires Windows Explorer and a populated AppsFolder"]
fn windows_provides_context_commands_for_start_apps() -> Result<(), String> {
    let _com = Com::enter(COINIT_APARTMENTTHREADED).map_err(|error| error.to_string())?;
    let apps = installed_apps()?;
    let with_commands = apps
        .iter()
        .filter(|app| unsafe {
            let Ok(commands) = shell_commands(&app.target) else {
                return false;
            };
            let Ok(menu) = CreatePopupMenu().map(ShellMenu) else {
                return false;
            };
            commands
                .QueryContextMenu(menu.0, 0, 1, 0x7fff, CMF_NORMAL)
                .is_ok()
                && GetMenuItemCount(Some(menu.0)) > 1
        })
        .count();
    assert!(
        with_commands > 0,
        "Windows supplied no app menus for {} Start apps",
        apps.len()
    );
    println!(
        "Windows supplied context commands for {with_commands} of {} Start apps",
        apps.len()
    );
    Ok(())
}

#[tauri::command]
pub async fn open_start_app_menu(
    app: AppHandle,
    source: WebviewWindow,
    state: State<'_, StartApps>,
    id: String,
    anchor: desktop::PopupAnchor,
) -> Result<(), String> {
    start_is_open(&source)?;
    let target = start_target(&state, &id)?;
    let (send, mut result) = tauri::async_runtime::channel(1);
    app.run_on_main_thread(move || {
        let open = || -> Result<bool, String> {
            start_is_open(&source)?;
            let scale = source.scale_factor().map_err(|error| error.to_string())?;
            let bounds = desktop::screen_anchor(&source, anchor, scale)?;
            let hwnd = HWND(desktop::handle(&source)?);
            show_shell_menu(&target, hwnd, bounds.left, bounds.bottom)
                .map_err(|error| format!("Couldn't open the Windows app menu. {error}"))
        };
        let _ = send.try_send(open());
    })
    .map_err(|error| error.to_string())?;
    if result
        .recv()
        .await
        .ok_or("Windows closed the app menu.")??
    {
        close_spacebar_start(&app);
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StartAction {
    WindowsSettings,
    SpacebarSettings,
    Files,
    Lock,
    Shutdown,
    Restart,
}

fn power_action(restart: bool) -> Result<(), String> {
    let mut system = [0u16; 260];
    let len = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if len == 0 || len >= system.len() {
        return Err("Windows couldn’t locate the system directory.".into());
    }
    let shutdown = PathBuf::from(OsString::from_wide(&system[..len])).join("shutdown.exe");
    let status = Command::new(shutdown)
        .args([if restart { "/r" } else { "/s" }, "/t", "0"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Windows couldn’t run this power action.".into())
}

#[tauri::command]
pub async fn start_action(
    app: AppHandle,
    source: WebviewWindow,
    action: StartAction,
    confirmed: bool,
) -> Result<(), String> {
    start_is_open(&source)?;
    match action {
        StartAction::WindowsSettings | StartAction::Files => {
            let target = if matches!(action, StartAction::Files) {
                "explorer.exe"
            } else {
                "ms-settings:"
            };
            if !launch_file(Path::new(target), None) {
                return Err("Windows couldn’t open this shortcut.".into());
            }
        }
        StartAction::SpacebarSettings => desktop::open_settings(app.clone(), None)?,
        StartAction::Lock => press_keys(&[VK_LWIN, b'L' as u16])?,
        StartAction::Shutdown | StartAction::Restart => {
            if !confirmed {
                return Err("Confirm this power action first.".into());
            }
            tauri::async_runtime::spawn_blocking(move || {
                power_action(matches!(action, StartAction::Restart))
            })
            .await
            .map_err(|error| error.to_string())??;
        }
    }
    close_spacebar_start(&app);
    Ok(())
}
