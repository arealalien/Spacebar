use crate::{
    desktop::{
        animations_enabled, center, follow_popup_bounds, handle, monitor_info, move_popup,
        popup_bounds, popup_space, screen_anchor, window, PopupAnchor,
    },
    dock_config::DockStore,
    popups::PopupState,
    previews::{self, Previews},
    settings::DockEdge,
    system_panel::{set_panel_region, SystemPanel},
    widgets::{OpenFolder, WidgetDrag},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::WindowsAndMessaging::*,
};

// Fall back to hiding the native window if the WebView cannot acknowledge the exit.
const EXIT_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetPopupKind {
    Spotify,
    Weather,
    WeatherColorful,
    #[serde(rename = "weather-3d")]
    Weather3d,
    WeatherAmbient,
}

impl WidgetPopupKind {
    fn is_weather(self) -> bool {
        self != Self::Spotify
    }
}

#[derive(Clone, Copy, Serialize)]
pub struct WidgetPopupDetails {
    id: u32,
    kind: WidgetPopupKind,
    edge: DockEdge,
    keyboard: bool,
    animated: bool,
    closing: bool,
}

#[derive(Clone, Serialize)]
pub struct WidgetPopupAnchor {
    id: u32,
    widget: String,
}

struct OpenPopup {
    widget: String,
    folder: bool,
    hwnd: usize,
    anchor: RECT,
    bounds: RECT,
    area: RECT,
    scale: f64,
    gap: i32,
    outside: Option<Instant>,
    exit_deadline: Option<Instant>,
    popup: WidgetPopupDetails,
}

impl OpenPopup {
    fn origin(&self) -> &'static str {
        if self.folder {
            "folder"
        } else {
            "dock"
        }
    }

    fn source_anchor(&self) -> WidgetPopupAnchor {
        WidgetPopupAnchor {
            id: self.popup.id,
            widget: self.widget.clone(),
        }
    }

    fn focused(&self) -> bool {
        // A click inside a mouse-opened forecast can activate the WebView.
        // Only popups opened with explicit focus stay when the cursor leaves.
        self.popup.keyboard && unsafe { GetForegroundWindow() == self.hwnd as HWND }
    }
}

#[derive(Default)]
pub struct WidgetPopup {
    current: Mutex<Option<OpenPopup>>,
    next_id: AtomicU32,
}

impl WidgetPopup {
    pub fn active(&self) -> bool {
        self.current.lock().is_ok_and(|open| open.is_some())
    }

    pub(crate) fn weather_open(&self) -> bool {
        self.current.lock().is_ok_and(|open| {
            open.as_ref().is_some_and(|open| {
                open.popup.kind.is_weather() && !open.popup.closing
            })
        })
    }

    pub fn from_folder(&self) -> bool {
        self.current
            .lock()
            .is_ok_and(|open| open.as_ref().is_some_and(|open| open.folder))
    }

    pub fn close(&self, app: &AppHandle) {
        let (popup, origin) = {
            let Ok(mut current) = self.current.lock() else {
                return;
            };
            let Some(open) = current.as_mut().filter(|open| open.exit_deadline.is_none()) else {
                return;
            };
            open.exit_deadline = Some(Instant::now() + EXIT_TIMEOUT);
            open.popup.closing = true;
            (open.popup, open.origin())
        };
        let _ = app.emit_to(
            origin,
            "widget-popup-anchor-changed",
            Option::<WidgetPopupAnchor>::None,
        );
        if !popup.animated || !animations_enabled() {
            self.finish_exit(app, popup.id);
            return;
        }
        let _ = app.emit_to("widget-popup", "widget-popup-changed", popup);
    }

    fn take_closing(&self, id: u32) -> Option<OpenPopup> {
        self.current
            .lock()
            .ok()?
            .take_if(|open| open.popup.id == id && open.exit_deadline.is_some())
    }

    fn finish_exit(&self, app: &AppHandle, id: u32) {
        let Some(open) = self.take_closing(id) else {
            return;
        };
        let hwnd = open.hwnd as HWND;
        let restore_folder = open.folder
            && app.state::<OpenFolder>().active()
            && unsafe { GetForegroundWindow() == hwnd };
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        if restore_folder {
            if let Ok(folder) = window(app, "folder") {
                let _ = folder.set_focus();
            }
        }
        let _ = app.emit_to(
            "widget-popup",
            "widget-popup-changed",
            Option::<WidgetPopupDetails>::None,
        );
    }

    pub fn track(&self, app: &AppHandle, cursor: POINT) -> bool {
        let expired_exit = {
            let Ok(mut state) = self.current.lock() else {
                return false;
            };
            let Some(open) = state.as_mut() else {
                return false;
            };
            if let Some(deadline) = open.exit_deadline {
                if Instant::now() < deadline {
                    return true;
                }
                Some(open.popup.id)
            } else {
                if open.focused()
                    || previews::popup_hovered(open.popup.edge, open.bounds, open.anchor, cursor)
                {
                    open.outside = None;
                    return true;
                }
                if open.outside.get_or_insert_with(Instant::now).elapsed()
                    < Duration::from_millis(115)
                {
                    return true;
                }
                None
            }
        };
        if let Some(id) = expired_exit {
            self.finish_exit(app, id);
        } else {
            self.close(app);
        }
        self.active()
    }
}

#[tauri::command]
pub fn read_widget_popup(state: State<'_, WidgetPopup>) -> Option<WidgetPopupDetails> {
    state.current.lock().ok()?.as_ref().map(|open| open.popup)
}

#[tauri::command]
pub fn read_widget_popup_anchor(
    webview: WebviewWindow,
    state: State<'_, WidgetPopup>,
) -> Option<WidgetPopupAnchor> {
    let state = state.current.lock().ok()?;
    let open = state
        .as_ref()
        .filter(|open| !open.popup.closing && open.origin() == webview.label())?;
    Some(open.source_anchor())
}

fn outside_anchor(mut anchor: RECT, perimeter: RECT, edge: DockEdge) -> RECT {
    match edge {
        DockEdge::Bottom => anchor.top = anchor.top.min(perimeter.top),
        DockEdge::Top => anchor.bottom = anchor.bottom.max(perimeter.bottom),
        DockEdge::Left => anchor.right = anchor.right.max(perimeter.right),
        DockEdge::Right => anchor.left = anchor.left.min(perimeter.left),
    }
    anchor
}

#[tauri::command]
pub fn move_widget_popup(
    webview: WebviewWindow,
    state: State<'_, WidgetPopup>,
    id: u32,
    anchor: PopupAnchor,
    perimeter: PopupAnchor,
) -> Result<bool, String> {
    let mut state = state.current.lock().map_err(|error| error.to_string())?;
    let Some(open) = state.as_mut().filter(|open| {
        open.popup.id == id && !open.popup.closing && open.origin() == webview.label()
    }) else {
        return Ok(false);
    };
    let anchor = screen_anchor(&webview, anchor, open.scale)?;
    let perimeter = screen_anchor(&webview, perimeter, open.scale)?;
    let edge = open.popup.edge;
    let space = popup_space(
        open.area,
        outside_anchor(anchor, perimeter, edge),
        edge,
        open.gap,
    );
    let bounds = follow_popup_bounds(space, anchor, edge, open.bounds);
    if bounds.left != open.bounds.left || bounds.top != open.bounds.top {
        if !move_popup(open.hwnd as HWND, bounds) {
            return Err("Windows couldn’t move the widget panel.".into());
        }
        open.bounds = bounds;
    }
    open.anchor = anchor;
    Ok(true)
}

#[tauri::command]
pub fn close_widget_popup(app: AppHandle, state: State<'_, WidgetPopup>) {
    state.close(&app);
}

#[tauri::command]
pub fn finish_widget_popup_exit(app: AppHandle, state: State<'_, WidgetPopup>, id: u32) {
    state.finish_exit(&app, id);
}

#[tauri::command]
pub fn show_widget_popup(
    app: AppHandle,
    webview: WebviewWindow,
    state: State<'_, WidgetPopup>,
    kind: WidgetPopupKind,
    widget: String,
    anchor: PopupAnchor,
    perimeter: PopupAnchor,
    toggle: bool,
    keyboard: bool,
) -> Result<(), String> {
    let folder = match webview.label() {
        "dock" => false,
        "folder" if app.state::<OpenFolder>().active() => true,
        _ => return Ok(()),
    };
    let popups = app.state::<PopupState>();
    if popups.menu_open()
        || popups.editing()
        || app.state::<WidgetDrag>().active()
        || app.state::<SystemPanel>().active()
    {
        return Ok(());
    }
    let (same_widget, focused) = {
        let open = state.current.lock().map_err(|error| error.to_string())?;
        open.as_ref()
            .filter(|open| open.exit_deadline.is_none())
            .map_or((false, false), |open| {
                (
                    open.popup.kind == kind && open.widget == widget && open.folder == folder,
                    open.focused(),
                )
            })
    };
    if toggle && same_widget && (!keyboard || focused) {
        state.close(&app);
        return Ok(());
    }
    if !toggle && !keyboard && (same_widget || focused) {
        return Ok(());
    }

    let settings = app.state::<DockStore>().current();
    let edge = if folder && kind.is_weather() {
        DockEdge::Bottom
    } else {
        settings.edge
    };
    let scale = webview.scale_factor().map_err(|error| error.to_string())?;
    let px = |value: f64| (value * scale).round() as i32;
    let anchor = screen_anchor(&webview, anchor, scale)?;
    let perimeter = screen_anchor(&webview, perimeter, scale)?;
    let monitor = monitor_info(center(anchor))?;
    let area = if settings.replace_taskbar {
        monitor.rcMonitor
    } else {
        monitor.rcWork
    };
    let gap = px(9.0);
    let space = popup_space(area, outside_anchor(anchor, perimeter, edge), edge, gap);
    let (width, height) = match kind {
        WidgetPopupKind::Spotify => (352.0, 282.0),
        WidgetPopupKind::Weather => (382.0, 374.0),
        WidgetPopupKind::WeatherColorful => (448.0, 496.0),
        WidgetPopupKind::Weather3d => (348.0, 600.0),
        WidgetPopupKind::WeatherAmbient => (416.0, 196.0),
    };
    let bounds = popup_bounds(space, anchor, edge, px(width), px(height));
    let surface = window(&app, "widget-popup")?;
    let hwnd = handle(&surface)?;
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
        return Err("Couldn’t position the widget panel.".into());
    }
    set_panel_region(hwnd, bounds, px(40.0), kind.is_weather());
    previews::close_all(&app);
    let popup = WidgetPopupDetails {
        id: state.next_id.fetch_add(1, Ordering::Relaxed),
        kind,
        edge,
        keyboard,
        animated: animations_enabled(),
        closing: false,
    };
    let open = OpenPopup {
        widget,
        folder,
        hwnd: hwnd as usize,
        anchor,
        bounds,
        area,
        scale,
        gap,
        outside: None,
        exit_deadline: None,
        popup,
    };
    let source_anchor = open.source_anchor();
    let previous = state
        .current
        .lock()
        .map_err(|error| error.to_string())?
        .replace(open);
    if let Some(previous) = previous.filter(|previous| previous.origin() != webview.label()) {
        let _ = app.emit_to(
            previous.origin(),
            "widget-popup-anchor-changed",
            Option::<WidgetPopupAnchor>::None,
        );
    }
    let _ = app.emit_to(
        webview.label(),
        "widget-popup-anchor-changed",
        source_anchor,
    );
    let _ = app.emit_to("widget-popup", "widget-popup-changed", popup);
    unsafe {
        ShowWindow(hwnd, if keyboard { SW_SHOW } else { SW_SHOWNOACTIVATE });
    }
    app.state::<Previews>().wake_watcher();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_designs_can_be_saved_and_opened() {
        let designs = ["weather", "weather-colorful", "weather-3d", "weather-ambient"];
        let items: Vec<_> = designs
            .iter()
            .map(|kind| serde_json::json!({ "id": kind, "kind": kind }))
            .collect();
        let layout: crate::widgets::Layout =
            serde_json::from_value(serde_json::json!({ "revision": 1, "items": items })).unwrap();
        assert!(crate::widgets::valid_layout(&layout.items));
        for kind in designs {
            let value = serde_json::json!(kind);
            let popup: WidgetPopupKind = serde_json::from_value(value.clone()).unwrap();
            assert!(popup.is_weather());
            assert_eq!(serde_json::to_value(popup).unwrap(), value);
        }
    }

    #[test]
    fn an_exit_only_hides_the_matching_closing_panel() {
        let panel = WidgetPopup::default();
        let set_popup = |id, closing: bool| {
            *panel.current.lock().unwrap() = Some(OpenPopup {
                widget: "widget-popup".into(),
                folder: false,
                hwnd: 0,
                anchor: RECT::default(),
                bounds: RECT::default(),
                area: RECT::default(),
                scale: 1.0,
                gap: 9,
                outside: None,
                exit_deadline: closing.then(Instant::now),
                popup: WidgetPopupDetails {
                    id,
                    kind: WidgetPopupKind::Spotify,
                    edge: DockEdge::Bottom,
                    keyboard: false,
                    animated: true,
                    closing,
                },
            });
        };
        set_popup(1, true);
        set_popup(2, false);
        assert!(panel.take_closing(1).is_none());
        assert!(panel.take_closing(2).is_none());
        set_popup(2, true);
        assert!(panel.take_closing(1).is_none());
        assert!(panel.take_closing(2).is_some());
        assert!(!panel.active());
    }
}
