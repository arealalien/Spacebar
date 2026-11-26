use crate::{
    desktop::{
        center, dock_popup_anchor, handle, monitor_info, popup_bounds, popup_space, screen_anchor,
        window, PanelTrigger, PopupAnchor,
    },
    dock_config::DockStore,
    popups, previews,
    settings::DockEdge,
    start_backdrop,
    system_widgets::{com_worker, shell_worker},
    tray_icons::{self, TrayIcon},
    usb_drives::{self, UsbDrive},
};
use serde::{Deserialize, Serialize};
use std::{
    ptr::null_mut,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn},
    },
    UI::WindowsAndMessaging::{GetForegroundWindow, SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE},
};

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Panel {
    Tray,
    QuickSettings,
    AudioOutput,
    Microphone,
    Notifications,
    QuickNote,
    DrawingPad,
    Agenda,
    QuickTasks,
    ColorPicker,
    SpacebarStart,
}

struct PanelOrigin {
    edge: DockEdge,
    folder: bool,
    scale: f64,
    anchor: RECT,
    trigger: Option<PanelTrigger>,
}

impl PanelOrigin {
    fn start(app: &AppHandle, folder: bool, trigger: Option<PanelTrigger>) -> Result<Self, String> {
        let (anchor, edge, scale) = dock_popup_anchor(app)?;
        Ok(Self {
            edge,
            folder,
            scale,
            anchor,
            trigger,
        })
    }
}

struct OpenPanel {
    panel: Panel,
    folder: bool,
    trigger: Option<PanelTrigger>,
    backdrop: Option<String>,
}

#[derive(Default)]
pub struct SystemPanel(Mutex<Option<OpenPanel>>);

static START_WINDOW: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn start_window() -> HWND {
    START_WINDOW.load(Ordering::Acquire) as HWND
}

impl SystemPanel {
    pub(crate) fn panel(&self) -> Option<Panel> {
        self.0.lock().ok()?.as_ref().map(|open| open.panel)
    }

    pub fn active(&self) -> bool {
        self.panel().is_some()
    }

    pub(crate) fn start_active(&self) -> bool {
        self.panel() == Some(Panel::SpacebarStart)
    }

    pub fn from_folder(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|state| state.as_ref().is_some_and(|open| open.folder))
    }

    pub fn points_at_trigger(&self, cursor: POINT, root: HWND) -> bool {
        self.0.lock().is_ok_and(|state| {
            state
                .as_ref()
                .and_then(|open| open.trigger)
                .is_some_and(|trigger| trigger.hit(cursor, root))
        })
    }
}

#[tauri::command]
pub fn read_system_panel(state: State<'_, SystemPanel>) -> Option<Panel> {
    state.panel()
}

#[tauri::command]
pub fn read_start_backdrop(source: WebviewWindow, state: State<'_, SystemPanel>) -> Option<String> {
    if source.label() != "system" {
        return None;
    }
    state.0.lock().ok()?.as_ref()?.backdrop.clone()
}

#[tauri::command]
pub fn close_system_panel(app: AppHandle, state: State<'_, SystemPanel>) {
    let Some(open) = state.0.lock().ok().and_then(|mut state| state.take()) else {
        return;
    };
    if open.panel == Panel::SpacebarStart {
        START_WINDOW.store(0, Ordering::Release);
    }
    if let Ok(surface) = window(&app, "system") {
        let restore_folder = open.folder
            && app.state::<crate::widgets::OpenFolder>().active()
            && handle(&surface).is_ok_and(|hwnd| unsafe { GetForegroundWindow() == hwnd });
        let _ = surface.hide();
        if restore_folder {
            if let Ok(folder) = window(&app, "folder") {
                let _ = folder.set_focus();
            }
        }
    }
    let _ = app.emit_to("system", "system-panel-changed", Option::<Panel>::None);
    if open.backdrop.is_some() {
        let _ = app.emit_to("system", "start-backdrop-changed", Option::<String>::None);
    }
    app.state::<previews::Previews>().wake_watcher();
}

pub(crate) fn close_spacebar_start(app: &AppHandle) {
    let state = app.state::<SystemPanel>();
    if state.start_active() {
        close_system_panel(app.clone(), state);
    }
}

pub(crate) fn toggle_start_from_keyboard(app: &AppHandle) -> Result<(), String> {
    set_start_visible(app, !app.state::<SystemPanel>().start_active())
}

pub(crate) fn set_start_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    if !visible {
        close_spacebar_start(app);
        return Ok(());
    }
    if app.state::<SystemPanel>().start_active() {
        return Ok(());
    }
    toggle_panel(
        app,
        app.state(),
        Panel::SpacebarStart,
        PanelOrigin::start(app, false, None)?,
    )
}

#[tauri::command]
pub fn open_system_panel(
    app: AppHandle,
    webview: WebviewWindow,
    state: State<'_, SystemPanel>,
    panel: Panel,
    anchor: PopupAnchor,
) -> Result<(), String> {
    let (edge, folder) = match webview.label() {
        "dock" => (app.state::<DockStore>().current().edge, false),
        "folder" if app.state::<crate::widgets::OpenFolder>().active() => (DockEdge::Bottom, true),
        _ => return Err("The system panel is unavailable here.".into()),
    };
    let scale = webview.scale_factor().map_err(|error| error.to_string())?;
    let anchor = screen_anchor(&webview, anchor, scale)?;
    let trigger = Some(PanelTrigger::new(handle(&webview)?, anchor));
    let origin = if panel == Panel::SpacebarStart {
        PanelOrigin::start(&app, folder, trigger)?
    } else {
        PanelOrigin {
            edge,
            folder,
            scale,
            anchor,
            trigger,
        }
    };
    toggle_panel(&app, state, panel, origin)
}

fn toggle_panel(
    app: &AppHandle,
    state: State<'_, SystemPanel>,
    panel: Panel,
    origin: PanelOrigin,
) -> Result<(), String> {
    let PanelOrigin {
        edge,
        folder,
        scale,
        anchor,
        trigger,
    } = origin;
    if state.panel() == Some(panel)
        && (panel == Panel::SpacebarStart || state.from_folder() == folder)
    {
        close_system_panel(app.clone(), state);
        return Ok(());
    }
    let start = panel == Panel::SpacebarStart;
    let glass = matches!(
        panel,
        Panel::SpacebarStart
            | Panel::QuickSettings
            | Panel::AudioOutput
            | Panel::Microphone
            | Panel::Tray
    );
    let rounded = glass || matches!(panel, Panel::QuickTasks | Panel::ColorPicker);
    let monitor = monitor_info(center(anchor))?;
    let area = if glass && !app.state::<DockStore>().current().replace_taskbar {
        monitor.rcWork
    } else {
        monitor.rcMonitor
    };
    let bounds = panel_bounds(panel, area, anchor, edge, scale);
    let surface = window(app, "system")?;
    let hwnd = handle(&surface)?;
    if state.active() {
        surface.hide().map_err(|error| error.to_string())?;
        if glass {
            unsafe { DwmFlush() };
        }
    }
    if unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOACTIVATE,
        )
    } == 0
    {
        close_system_panel(app.clone(), state);
        return Err("Couldn’t position the system panel.".into());
    }
    set_panel_region(
        hwnd,
        bounds,
        ((if glass { 52.0 } else { 40.0 }) * scale).round() as i32,
        rounded,
    );
    previews::close_all(app);
    app.state::<crate::widget_popup::WidgetPopup>().close(app);
    popups::close_context_menu(app.clone(), app.state());
    let backdrop = if glass {
        start_backdrop::capture(bounds)
    } else {
        None
    };
    *state.0.lock().map_err(|error| error.to_string())? = Some(OpenPanel {
        panel,
        folder,
        trigger,
        backdrop: backdrop.clone(),
    });
    START_WINDOW.store(if start { hwnd as usize } else { 0 }, Ordering::Release);
    let _ = app.emit_to("system", "start-backdrop-changed", backdrop);
    let _ = app.emit_to("system", "system-panel-changed", panel);
    if let Err(error) = surface.show().and_then(|()| surface.set_focus()) {
        close_system_panel(app.clone(), state);
        return Err(error.to_string());
    }
    app.state::<previews::Previews>().wake_watcher();
    Ok(())
}

fn panel_bounds(panel: Panel, area: RECT, anchor: RECT, edge: DockEdge, scale: f64) -> RECT {
    let px = |value: f64| (value * scale).round() as i32;
    let (width, height) = match panel {
        Panel::Tray => (370.0, 342.0),
        Panel::QuickSettings | Panel::AudioOutput | Panel::Microphone => (440.0, 426.0),
        Panel::Notifications => (292.0, 298.0),
        Panel::QuickNote => (360.0, 282.0),
        Panel::DrawingPad => (704.0, 484.0),
        Panel::Agenda => (382.0, 404.0),
        Panel::QuickTasks => (360.0, 346.0),
        Panel::ColorPicker => (332.0, 370.0),
        Panel::SpacebarStart => (868.0, 714.0),
    };
    let space = popup_space(area, anchor, edge, px(9.0));
    popup_bounds(space, anchor, edge, px(width), px(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_centered_over_the_dock_body() {
        let area = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let dock = RECT {
            left: 120,
            top: 942,
            right: 1180,
            bottom: 1024,
        };
        let bounds = panel_bounds(Panel::SpacebarStart, area, dock, DockEdge::Bottom, 1.0);
        assert_eq!(center(bounds).x, center(dock).x);
        assert_eq!(bounds.bottom, dock.top - 9);
        assert_eq!(bounds.bottom - bounds.top, 714);
    }

    #[test]
    fn start_fits_above_the_dock_on_a_small_scaled_secondary_monitor() {
        let area = RECT {
            left: -1080,
            top: 120,
            right: 0,
            bottom: 1320,
        };
        let dock = RECT {
            left: -690,
            top: 1100,
            right: -390,
            bottom: 1260,
        };
        let bounds = panel_bounds(Panel::SpacebarStart, area, dock, DockEdge::Bottom, 1.5);
        assert_eq!(center(bounds).x, center(dock).x);
        assert_eq!((bounds.left, bounds.right), (-1066, -14));
        assert_eq!((bounds.top, bounds.bottom), (134, 1086));
    }
}

pub(crate) fn set_panel_region(hwnd: HWND, bounds: RECT, corner: i32, enabled: bool) {
    let rounded = enabled
        && unsafe {
            let region = CreateRoundRectRgn(
                0,
                0,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                corner,
                corner,
            );
            if region.is_null() {
                false
            } else if SetWindowRgn(hwnd, region, 1) != 0 {
                true
            } else {
                DeleteObject(region);
                false
            }
        };
    if !rounded {
        unsafe {
            SetWindowRgn(hwnd, null_mut(), 1);
        }
    }
}

#[tauri::command]
pub async fn read_tray_icons(webview: WebviewWindow) -> Result<Vec<TrayIcon>, String> {
    if webview.label() != "system" {
        return Err("The system tray is unavailable here.".into());
    }
    com_worker(tray_icons::list).await
}

#[tauri::command]
pub async fn activate_tray_icon(
    app: AppHandle,
    webview: WebviewWindow,
    id: String,
) -> Result<(), String> {
    if webview.label() != "system" {
        return Err("The system tray is unavailable here.".into());
    }
    let own = com_worker(move || {
        let entry = tray_entry(&id)?;
        if entry.own_process() {
            return Ok(true);
        }
        tray_icons::activate(&entry)?;
        Ok(false)
    })
    .await?;
    if own {
        crate::desktop::open_settings(app, None)?;
    }
    Ok(())
}

fn tray_entry(id: &str) -> Result<TrayIcon, String> {
    tray_icons::list()?
        .into_iter()
        .find(|entry| entry.id() == id)
        .ok_or_else(|| "This tray icon has closed.".into())
}

fn require_tray(webview: &WebviewWindow, state: &SystemPanel) -> Result<(), String> {
    if webview.label() == "system" && state.panel() == Some(Panel::Tray) {
        Ok(())
    } else {
        Err("Open the System Tray widget first.".into())
    }
}

#[tauri::command]
pub async fn reveal_tray_app(
    app: AppHandle,
    webview: WebviewWindow,
    id: String,
) -> Result<(), String> {
    require_tray(&webview, &app.state::<SystemPanel>())?;
    shell_worker(move || tray_entry(&id)?.reveal()).await
}

#[tauri::command]
pub async fn read_tray_usb_drives(
    app: AppHandle,
    webview: WebviewWindow,
) -> Result<Vec<UsbDrive>, String> {
    require_tray(&webview, &app.state::<SystemPanel>())?;
    com_worker(usb_drives::list).await
}

#[tauri::command]
pub async fn eject_tray_usb_drive(
    app: AppHandle,
    webview: WebviewWindow,
    id: String,
) -> Result<(), String> {
    require_tray(&webview, &app.state::<SystemPanel>())?;
    com_worker(move || usb_drives::eject(&id)).await
}
