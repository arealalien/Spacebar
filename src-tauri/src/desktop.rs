use crate::{
    apps::{self, AppCatalog},
    connectivity,
    dock_config::{self, DockStore},
    media::{self, MediaState},
    popups::{self, PopupState},
    previews::{self, Previews},
    replacement::Replacement,
    screen_color,
    settings::DockEdge,
    widget_popup::{self, WidgetPopup},
    start_launcher::{self, StartApps},
    start_menu::{self, Panel, StartMenu},
    startup,
    system_panel::{self, SystemPanel},
    system_widgets,
    taskbar::{self, TaskbarOrder},
    telemetry::{self, Telemetry},
    widget_content::{self, WidgetContentStore},
    widgets::{self, OpenFolder, WidgetDrag},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, State, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Dwm::*,
        Gdi::{
            ClientToScreen, CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW,
            MonitorFromPoint, SetWindowRgn, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        },
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, Default, PartialEq)]
struct Bounds {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct DockLayout {
    edge: DockEdge,
    window: Bounds,
    body: Bounds,
    monitor: Bounds,
    has_content: bool,
    scale: f64,
    corner: i32,
    margin: i32,
    top_margin: i32,
    snow_headroom: i32,
}

#[derive(Default)]
struct DockPlacement {
    layout: Option<DockLayout>,
    behind_start: bool,
    start_revealed: bool,
    expanded: bool,
    revealed: bool,
    idle_since: Option<Instant>,
    hide_at: Option<Instant>,
    effects: Option<DockEffectScene>,
    effects_position: Option<(Bounds, bool)>,
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DockEffectScene {
    opacity: f64,
    offset_x: f64,
    offset_y: f64,
}

#[derive(Default)]
struct DockEffectsWindow(Mutex<()>);

fn overlaps_start(layout: DockLayout, panel: Option<Panel>) -> bool {
    let Some(panel) = panel else { return false };
    let body = layout.body;
    let panel = panel.bounds;
    layout.has_content
        && panel.left < body.left + body.width
        && panel.right > body.left
        && panel.top < body.top + body.height
        && panel.bottom > body.top
}

#[derive(Default)]
struct Placement(Mutex<DockPlacement>);

#[derive(Default)]
struct PreviousForeground(AtomicUsize);

pub(crate) fn foreground_for_dock(app: &AppHandle) -> HWND {
    let foreground = unsafe { GetForegroundWindow() };
    let dock = window(app, "dock").and_then(|dock| handle(&dock)).ok();
    if dock.is_some_and(|dock| foreground == dock) {
        return app.state::<PreviousForeground>().0.load(Ordering::Relaxed) as HWND;
    }
    foreground
}

pub(crate) fn window(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    app.get_webview_window(label)
        .ok_or_else(|| format!("Spacebar’s {label} window is unavailable."))
}

pub(crate) fn handle(window: &WebviewWindow) -> Result<HWND, String> {
    window
        .hwnd()
        .map(|handle| handle.0 as HWND)
        .map_err(|error| error.to_string())
}

pub(crate) fn monitor_info(point: POINT) -> Result<MONITORINFO, String> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err("Windows couldn’t report the monitor usable area.".into());
        }
    }
    Ok(info)
}

pub(crate) fn dock_popup_anchor(app: &AppHandle) -> Result<(RECT, DockEdge, f64), String> {
    let layout = app
        .state::<Placement>()
        .0
        .lock()
        .ok()
        .and_then(|state| state.layout);
    if let Some(layout) = layout {
        let body = layout.body;
        let anchor = RECT {
            left: body.left,
            top: body.top,
            right: body.left + body.width,
            bottom: body.top + body.height,
        };
        return Ok((anchor, layout.edge, layout.scale));
    }

    // A shortcut can arrive before the dock has reported its first layout.
    let settings = app.state::<DockStore>().current();
    let monitor = monitor_info(POINT::default())?;
    let area = if settings.replace_taskbar {
        monitor.rcMonitor
    } else {
        monitor.rcWork
    };
    let scale = app
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .map_or(1.0, |monitor| monitor.scale_factor());
    let inset = (f64::from(settings.offset + settings.icon_size + 26) * scale).round() as i32;
    let mut point = center(area);
    match settings.edge {
        DockEdge::Bottom => point.y = area.bottom - inset,
        DockEdge::Top => point.y = area.top + inset,
        DockEdge::Left => point.x = area.left + inset,
        DockEdge::Right => point.x = area.right - inset,
    }
    let anchor = RECT {
        left: point.x,
        top: point.y,
        right: point.x,
        bottom: point.y,
    };
    Ok((anchor, settings.edge, scale))
}

#[derive(Deserialize)]
pub struct PopupAnchor {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

pub(crate) fn screen_anchor(
    source: &WebviewWindow,
    anchor: PopupAnchor,
    scale: f64,
) -> Result<RECT, String> {
    let mut origin = POINT::default();
    if unsafe { ClientToScreen(handle(source)?, &mut origin) } == 0 {
        return Err("Couldn’t locate the dock.".into());
    }
    let px = |value: f64| (value * scale).round() as i32;
    Ok(RECT {
        left: origin.x + px(anchor.left),
        top: origin.y + px(anchor.top),
        right: origin.x + px(anchor.right),
        bottom: origin.y + px(anchor.bottom),
    })
}

#[derive(Clone, Copy)]
pub(crate) struct PanelTrigger {
    owner: usize,
    bounds: RECT,
}

impl PanelTrigger {
    pub fn new(owner: HWND, bounds: RECT) -> Self {
        Self {
            owner: owner as usize,
            bounds,
        }
    }

    pub fn hit(self, cursor: POINT, root: HWND) -> bool {
        self.owner == root as usize
            && (self.bounds.left..self.bounds.right).contains(&cursor.x)
            && (self.bounds.top..self.bounds.bottom).contains(&cursor.y)
    }
}

pub(crate) fn center(rect: RECT) -> POINT {
    POINT {
        x: rect.left + (rect.right - rect.left) / 2,
        y: rect.top + (rect.bottom - rect.top) / 2,
    }
}

pub(crate) fn popup_space(area: RECT, anchor: RECT, edge: DockEdge, gap: i32) -> RECT {
    let mut space = RECT {
        left: area.left + gap,
        top: area.top + gap,
        right: area.right - gap,
        bottom: area.bottom - gap,
    };
    match edge {
        DockEdge::Bottom => space.bottom = space.bottom.min(anchor.top - gap).max(space.top + 1),
        DockEdge::Top => space.top = space.top.max(anchor.bottom + gap).min(space.bottom - 1),
        DockEdge::Left => space.left = space.left.max(anchor.right + gap).min(space.right - 1),
        DockEdge::Right => space.right = space.right.min(anchor.left - gap).max(space.left + 1),
    }
    space
}

pub(crate) fn popup_bounds(
    space: RECT,
    anchor: RECT,
    edge: DockEdge,
    width: i32,
    height: i32,
) -> RECT {
    let center = center(anchor);
    let width = width.min(space.right - space.left).max(1);
    let height = height.min(space.bottom - space.top).max(1);
    let left = match edge {
        DockEdge::Left => space.left,
        DockEdge::Right => space.right - width,
        _ => (center.x - width / 2).clamp(space.left, space.right - width),
    };
    let top = match edge {
        DockEdge::Top => space.top,
        DockEdge::Bottom => space.bottom - height,
        _ => (center.y - height / 2).clamp(space.top, space.bottom - height),
    };
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

pub(crate) fn follow_popup_bounds(
    mut space: RECT,
    anchor: RECT,
    edge: DockEdge,
    panel: RECT,
) -> RECT {
    let width = panel.right - panel.left;
    let height = panel.bottom - panel.top;
    // Keep the existing content inside the monitor as its anchor moves.
    match edge {
        DockEdge::Bottom => space.bottom = space.bottom.max(space.top + height),
        DockEdge::Top => space.top = space.top.min(space.bottom - height),
        DockEdge::Left => space.left = space.left.min(space.right - width),
        DockEdge::Right => space.right = space.right.max(space.left + width),
    }
    popup_bounds(space, anchor, edge, width, height)
}

pub(crate) fn move_popup(hwnd: HWND, bounds: RECT) -> bool {
    unsafe {
        SetWindowPos(
            hwnd,
            0 as HWND,
            bounds.left,
            bounds.top,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
        ) != 0
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;

    #[test]
    fn dock_effects_leave_room_for_snow_and_stop_at_the_monitor_edges() {
        let monitor = Bounds {
            left: -1000,
            top: -20,
            width: 1000,
            height: 800,
        };
        let layout = DockLayout {
            edge: DockEdge::Left,
            window: Bounds {
                left: -984,
                top: -12,
                width: 160,
                height: 790,
            },
            monitor,
            scale: 1.5,
            ..DockLayout::default()
        };
        let effects = dock_effects_bounds(layout);
        assert_eq!(effects.left, monitor.left);
        assert_eq!(effects.left + effects.width, -716);
        assert_eq!(effects.top, monitor.top);
        assert_eq!(effects.top + effects.height, monitor.top + monitor.height);

        let layout = DockLayout {
            edge: DockEdge::Top,
            window: Bounds {
                left: -1000,
                top: 0,
                width: 1000,
                height: 260,
            },
            ..layout
        };
        let effects = dock_effects_bounds(layout);
        assert_eq!(effects.top, monitor.top);
        assert_eq!(effects.top + effects.height, 368);
        assert_eq!(effects.width, monitor.width);

        let layout = DockLayout {
            edge: DockEdge::Bottom,
            window: Bounds {
                left: -750,
                top: 360,
                width: 500,
                height: 260,
            },
            ..layout
        };
        let effects = dock_effects_bounds(layout);
        assert_eq!(effects.left, layout.window.left - 108);
        assert_eq!(effects.width, layout.window.width + 216);
        assert_eq!(effects.top + effects.height, 728);
    }

    #[test]
    fn panel_trigger_matches_its_widget_after_the_panel_loses_focus() {
        let dock = 1usize as HWND;
        let other = 2usize as HWND;
        let trigger = PanelTrigger::new(
            dock,
            RECT {
                left: 100,
                top: 140,
                right: 148,
                bottom: 190,
            },
        );
        assert!(trigger.hit(POINT { x: 124, y: 162 }, dock));
        assert!(!trigger.hit(POINT { x: 124, y: 162 }, other));
        assert!(!trigger.hit(POINT { x: 149, y: 162 }, dock));
    }

    #[test]
    fn panels_stay_inside_the_monitor_and_open_inward() {
        let area = RECT {
            left: -800,
            top: 40,
            right: 400,
            bottom: 840,
        };
        let anchors = [
            (
                DockEdge::Top,
                RECT {
                    left: -790,
                    top: 40,
                    right: -734,
                    bottom: 96,
                },
            ),
            (
                DockEdge::Bottom,
                RECT {
                    left: 286,
                    top: 772,
                    right: 342,
                    bottom: 828,
                },
            ),
            (
                DockEdge::Left,
                RECT {
                    left: -800,
                    top: 750,
                    right: -744,
                    bottom: 806,
                },
            ),
            (
                DockEdge::Right,
                RECT {
                    left: 344,
                    top: 40,
                    right: 400,
                    bottom: 96,
                },
            ),
        ];
        for (edge, anchor) in anchors {
            let bounds = popup_bounds(popup_space(area, anchor, edge, 10), anchor, edge, 260, 186);
            assert!(bounds.left >= area.left && bounds.right <= area.right);
            assert!(bounds.top >= area.top && bounds.bottom <= area.bottom);
            match edge {
                DockEdge::Top => assert_eq!(bounds.top, anchor.bottom + 10),
                DockEdge::Bottom => assert_eq!(bounds.bottom, anchor.top - 10),
                DockEdge::Left => assert_eq!(bounds.left, anchor.right + 10),
                DockEdge::Right => assert_eq!(bounds.right, anchor.left - 10),
            }
        }
    }
}

const FRAME_STYLE: u32 = WS_CAPTION | WS_THICKFRAME;

fn suppress_frame(hwnd: HWND, no_mouse_activate: bool) {
    let policy = DWMNCRP_DISABLED;
    let border = 0xffff_fffe_u32; // DWMWA_COLOR_NONE
    let flags = 1_i32;
    unsafe {
        SetWindowSubclass(
            hwnd,
            Some(frameless_surface_proc),
            1,
            no_mouse_activate as usize,
        );
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let frameless = style & !(FRAME_STYLE as isize);
        if frameless != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, frameless);
            SetWindowPos(
                hwnd,
                0 as HWND,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            (&policy as *const i32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            (&border as *const u32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            (&flags as *const i32).cast(),
            4,
        );
    }
}

unsafe extern "system" fn frameless_surface_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass: usize,
    no_mouse_activate: usize,
) -> LRESULT {
    match message {
        // Deliver clicks to the WebView without activating its top-level tool window.
        WM_MOUSEACTIVATE if no_mouse_activate != 0 => return MA_NOACTIVATE as LRESULT,
        WM_STYLECHANGING if wparam as i32 == GWL_STYLE && lparam != 0 => {
            // The runtime can restore its window styles when showing a hidden surface.
            (*(lparam as *mut STYLESTRUCT)).styleNew &= !FRAME_STYLE;
            return 0;
        }
        WM_NCCALCSIZE | WM_NCPAINT => return 0,
        WM_NCACTIVATE => return 1,
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(frameless_surface_proc), subclass);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

pub(crate) fn animations_enabled() -> bool {
    let mut enabled = 1_i32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        );
    }
    enabled != 0
}

#[derive(Default)]
struct SettingsWindow {
    widgets: AtomicBool,
    opening: AtomicBool,
}

#[tauri::command]
fn read_settings_page(settings: State<'_, SettingsWindow>) -> bool {
    settings.widgets.load(Ordering::Relaxed)
}

fn show_settings(app: &AppHandle, settings: &WebviewWindow) -> Result<(), String> {
    settings
        .show()
        .and_then(|()| settings.unminimize())
        .and_then(|()| settings.set_focus())
        .map_err(|error| error.to_string())?;
    let _ = app.emit("dock-apps-changed", ());
    Ok(())
}

#[tauri::command]
pub(crate) fn open_settings(app: AppHandle, widgets: Option<bool>) -> Result<(), String> {
    let state = app.state::<SettingsWindow>();
    let widgets = widgets.unwrap_or(false);
    state.widgets.store(widgets, Ordering::Relaxed);
    let _ = app.emit_to("settings", "settings-page", widgets);
    if let Some(settings) = app.get_webview_window("settings") {
        return show_settings(&app, &settings);
    }
    if !state.opening.swap(true, Ordering::AcqRel) {
        tauri::async_runtime::spawn_blocking(move || {
            let result = (|| {
                let config = app.config().app.windows.iter().find(|window| window.label == "settings")
                    .ok_or("Spacebar’s settings window is unavailable.")?;
                let settings = WebviewWindowBuilder::from_config(&app, config)
                    .and_then(|builder| builder.visible(false).build())
                    .map_err(|error| error.to_string())?;
                show_settings(&app, &settings)
            })();
            app.state::<SettingsWindow>().opening.store(false, Ordering::Release);
            if let Err(error) = result {
                eprintln!("Couldn’t open Spacebar Settings. {error}");
            }
        });
    }
    Ok(())
}

#[tauri::command]
fn quit_spacebar(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn place_dock(
    app: AppHandle,
    store: State<'_, DockStore>,
    placement: State<'_, Placement>,
    count: usize,
    layout_width: Option<f64>,
    layout_height: Option<f64>,
    layout_icon_size: Option<f64>,
) -> Result<(), String> {
    let dock = window(&app, "dock")?;
    let hwnd = handle(&dock)?;
    let monitor = monitor_info(POINT::default())?;
    let settings = store.current();
    let replacement = app.state::<Replacement>();
    if !settings.replace_taskbar {
        replacement.update(false)?;
    }
    let area = if settings.replace_taskbar {
        monitor.rcMonitor
    } else {
        monitor.rcWork
    };
    let scale = dock
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .map_or(1.0, |monitor| monitor.scale_factor());
    let px = |logical: f64| (logical * scale).round() as i32;
    let size = layout_icon_size
        .filter(|value| value.is_finite())
        .map_or(settings.icon_size as f64, |value| value.clamp(12.0, 112.0));
    let per_app = size + settings.spacing as f64;
    let padding = 2.0 * settings.padding as f64;
    let legacy_width = if settings.hide_settings_icon {
        per_app * count.min(4096) as f64 + padding
            - if count > 0 {
                settings.spacing as f64
            } else {
                0.0
            }
    } else {
        per_app * count.min(4096) as f64
            + padding + 31.0
            + if count > 0 {
                1.0 + settings.spacing as f64
            } else {
                0.0
            }
    };
    let has_content = layout_width.map_or(count > 0 || !settings.hide_settings_icon, |width| {
        width > 0.0
    });
    let base_width = layout_width
        .filter(|width| width.is_finite() && *width > 0.0)
        .unwrap_or(if has_content { legacy_width } else { 170.0 })
        .clamp(20.0, 32768.0);
    let spread = if settings.magnify && count > 0 {
        size * (settings.zoom - 1.0) * (settings.falloff + 1.2)
    } else {
        0.0
    };
    let vertical = settings.edge.vertical();
    let along = if vertical {
        area.bottom - area.top
    } else {
        area.right - area.left
    };
    let across = if vertical {
        area.right - area.left
    } else {
        area.bottom - area.top
    };
    let frame_length = if settings.full_span {
        along
    } else {
        px((base_width + (spread * 2.0).max(130.0)).max(184.0)).min(along)
    };
    let body_length = if settings.full_span {
        frame_length
    } else {
        px(base_width).min(frame_length)
    };
    let strip_height = layout_height
        .filter(|height| height.is_finite())
        .map_or(size + 7.0 + padding, |height| height.clamp(10.0, 260.0));
    let zoom = if settings.magnify { settings.zoom } else { 1.0 };
    let frame_depth = px((size * zoom + 75.0)
        .max(strip_height + size * (zoom - 1.0) + 34.0)
        .max(206.0))
    .min(across);
    let body_depth = px(strip_height + 4.0).min(frame_depth);
    let offset = px(settings.offset as f64);
    let snow_headroom = if settings.dock_snow { px(24.0) } else { 0 };
    let axis_start = if vertical { area.top } else { area.left }
        + settings.alignment.offset(along - frame_length);
    let body_start = axis_start + settings.alignment.offset(frame_length - body_length);
    let (window, body) = match settings.edge {
        DockEdge::Bottom => (
            Bounds {
                left: axis_start,
                top: area.bottom - offset - frame_depth,
                width: frame_length,
                height: frame_depth,
            },
            Bounds {
                left: body_start,
                top: area.bottom - offset - body_depth,
                width: body_length,
                height: body_depth,
            },
        ),
        DockEdge::Top => {
            // The WebView starts above the strip when the offset leaves room for snow.
            let headroom = offset.min(snow_headroom);
            (
                Bounds {
                    left: axis_start,
                    top: area.top + offset - headroom,
                    width: frame_length,
                    height: frame_depth + headroom,
                },
                Bounds {
                    left: body_start,
                    top: area.top + offset,
                    width: body_length,
                    height: body_depth,
                },
            )
        }
        DockEdge::Left => (
            Bounds {
                left: area.left + offset,
                top: axis_start,
                width: frame_depth,
                height: frame_length,
            },
            Bounds {
                left: area.left + offset,
                top: body_start,
                width: body_depth,
                height: body_length,
            },
        ),
        DockEdge::Right => (
            Bounds {
                left: area.right - offset - frame_depth,
                top: axis_start,
                width: frame_depth,
                height: frame_length,
            },
            Bounds {
                left: area.right - offset - body_depth,
                top: body_start,
                width: body_depth,
                height: body_length,
            },
        ),
    };
    let new_layout = DockLayout {
        edge: settings.edge,
        window,
        body,
        monitor: Bounds {
            left: monitor.rcMonitor.left,
            top: monitor.rcMonitor.top,
            width: monitor.rcMonitor.right - monitor.rcMonitor.left,
            height: monitor.rcMonitor.bottom - monitor.rcMonitor.top,
        },
        has_content,
        scale,
        corner: px(34.0),
        margin: px(8.0),
        top_margin: px(if settings.auto_hide { 26.0 } else { 8.0 }),
        snow_headroom,
    };
    let mut dock_placement = placement.0.lock().map_err(|error| error.to_string())?;
    if dock_placement.layout == Some(new_layout) {
        return replacement.update(settings.replace_taskbar && has_content);
    }
    let behind_start = overlaps_start(new_layout, app.state::<StartMenu>().panel());
    let success = unsafe {
        SetWindowPos(
            hwnd,
            if behind_start {
                HWND_NOTOPMOST
            } else {
                HWND_TOPMOST
            },
            new_layout.window.left,
            new_layout.window.top,
            new_layout.window.width,
            new_layout.window.height,
            SWP_NOACTIVATE,
        )
    };
    if success == 0 {
        return Err("Windows couldn’t position the dock.".into());
    }
    apply_dock_region(hwnd, new_layout, dock_placement.expanded)?;
    dock_placement.layout = Some(new_layout);
    dock_placement.behind_start = behind_start;
    dock_placement.effects_position = None;
    if !new_layout.has_content {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        hide_dock_effects(&app, &mut dock_placement);
        dock_placement.hide_at = None;
        dock_placement.revealed = false;
        let _ = app.emit_to("dock", "dock-revealed", false);
    } else if !settings.auto_hide {
        set_revealed(
            &app,
            hwnd,
            &mut dock_placement,
            true,
            settings.reveal_duration,
        );
    }
    replacement.update(settings.replace_taskbar && has_content)
}

fn apply_dock_region(hwnd: HWND, layout: DockLayout, expanded: bool) -> Result<(), String> {
    // Keep an explicit region even when expanded so DWM never restores a rectangular frame.
    unsafe {
        let region = if expanded {
            CreateRectRgn(0, 0, layout.window.width, layout.window.height)
        } else {
            let x = layout.body.left - layout.window.left;
            let y = layout.body.top - layout.window.top;
            let (left, top, right, bottom) = match layout.edge {
                DockEdge::Bottom => (layout.margin, layout.top_margin, layout.margin, 0),
                DockEdge::Top => (layout.margin, 0, layout.margin, layout.top_margin),
                DockEdge::Left => (0, layout.margin, layout.top_margin, layout.margin),
                DockEdge::Right => (layout.top_margin, layout.margin, 0, layout.margin),
            };
            CreateRoundRectRgn(
                (x - left).max(0),
                (y - top.max(layout.snow_headroom)).max(0),
                (x + layout.body.width + right).min(layout.window.width),
                (y + layout.body.height + bottom).min(layout.window.height),
                layout.corner,
                layout.corner,
            )
        };
        if region.is_null() {
            return Err("Windows couldn’t create the dock input area.".into());
        }
        if SetWindowRgn(hwnd, region, 1) == 0 {
            DeleteObject(region);
            return Err("Windows couldn’t update the dock input area.".into());
        }
    }
    Ok(())
}

fn dock_effects_bounds(layout: DockLayout) -> Bounds {
    let frame = layout.window;
    let monitor = layout.monitor;
    let reach = (72.0 * layout.scale).round() as i32;
    let left = (frame.left - reach).max(monitor.left);
    let right = (frame.left + frame.width + reach).min(monitor.left + monitor.width);
    let top = (frame.top - reach).max(monitor.top);
    let bottom = (frame.top + frame.height + reach).min(monitor.top + monitor.height);
    Bounds {
        left,
        top,
        width: (right - left).max(1),
        height: (bottom - top).max(1),
    }
}

#[tauri::command]
async fn prepare_dock_effects(app: AppHandle, source: WebviewWindow) -> Result<(), String> {
    if source.label() != "dock" {
        return Err("Only the dock can prepare its effects.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let effects = app.state::<DockEffectsWindow>();
        let _guard = effects.0.lock().map_err(|error| error.to_string())?;
        let surface = match app.get_webview_window("dock-effects") {
            Some(surface) => surface,
            None => {
                let config = app
                    .config()
                    .app
                    .windows
                    .iter()
                    .find(|window| window.label == "dock-effects")
                    .ok_or("The dock effects window is unavailable.")?;
                WebviewWindowBuilder::from_config(&app, config)
                    .and_then(|builder| builder.build())
                    .map_err(|error| error.to_string())?
            }
        };
        let hwnd = handle(&surface)? as usize;
        app.run_on_main_thread(move || suppress_frame(hwnd as HWND, true))
            .map_err(|error| error.to_string())?;
        surface
            .set_ignore_cursor_events(true)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn hide_dock_effects(app: &AppHandle, state: &mut DockPlacement) {
    if state.effects.take().is_some() {
        if let Ok(hwnd) = window(app, "dock-effects").and_then(|surface| handle(&surface)) {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
        let _ = app.emit_to(
            "dock-effects",
            "dock-effects-changed",
            Option::<DockEffectScene>::None,
        );
    }
    state.effects_position = None;
}

#[tauri::command]
fn read_dock_effects(placement: State<'_, Placement>) -> Option<DockEffectScene> {
    placement.0.lock().ok()?.effects.clone()
}

#[tauri::command]
fn update_dock_effects(
    app: AppHandle,
    source: WebviewWindow,
    placement: State<'_, Placement>,
    opacity: Option<f64>,
) -> Result<(), String> {
    if source.label() != "dock" {
        return Err("Only the dock can update its effects.".into());
    }
    let settings = app.state::<DockStore>().current();
    let dock_hwnd = handle(&source)?;
    let mut state = placement.0.lock().map_err(|error| error.to_string())?;
    let Some((layout, opacity)) = state.layout.zip(opacity).filter(|(layout, _)| {
        layout.has_content && settings.dock_snow && unsafe { IsWindowVisible(dock_hwnd) } != 0
    }) else {
        hide_dock_effects(&app, &mut state);
        return Ok(());
    };
    let bounds = dock_effects_bounds(layout);
    let position = (bounds, state.behind_start);
    if state.effects_position != Some(position) {
        let effects = handle(&window(&app, "dock-effects")?)?;
        let band = if state.behind_start {
            HWND_NOTOPMOST
        } else {
            HWND_TOPMOST
        };
        let preposition = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        let ready = unsafe {
            if SetWindowPos(effects, band, 0, 0, 0, 0, preposition) == 0 {
                false
            } else {
                // Insert immediately above the dock, below any open preview windows.
                let above = GetWindow(dock_hwnd, GW_HWNDPREV);
                let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
                let (after, flags) = if above == effects {
                    (effects, flags | SWP_NOZORDER)
                } else if above.is_null() {
                    (band, flags)
                } else {
                    (above, flags)
                };
                SetWindowPos(
                    effects,
                    after,
                    bounds.left,
                    bounds.top,
                    bounds.width,
                    bounds.height,
                    flags,
                ) != 0
            }
        };
        if !ready {
            return Err("Windows couldn’t position the dock effects.".into());
        }
        state.effects_position = Some(position);
    }
    let scene = DockEffectScene {
        opacity: opacity.clamp(0.0, 1.0),
        offset_x: f64::from(layout.window.left - bounds.left) / layout.scale,
        offset_y: f64::from(layout.window.top - bounds.top) / layout.scale,
    };
    if state.effects.as_ref() == Some(&scene) {
        return Ok(());
    }
    state.effects = Some(scene.clone());
    drop(state);
    app.emit_to("dock-effects", "dock-effects-changed", scene)
        .map_err(|error| error.to_string())
}

fn set_revealed(
    app: &AppHandle,
    hwnd: HWND,
    state: &mut DockPlacement,
    reveal: bool,
    duration: u32,
) {
    if state.revealed == reveal {
        return;
    }
    if reveal {
        state.hide_at = None;
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    } else {
        state.hide_at = Some(
            Instant::now()
                + Duration::from_millis(if animations_enabled() {
                    u64::from(duration)
                } else {
                    0
                }),
        );
    }
    state.revealed = reveal;
    let _ = app.emit_to("dock", "dock-revealed", reveal);
}

fn track_dock(app: &AppHandle, cursor: POINT, popup_open: bool) {
    let placement = app.state::<Placement>();
    let Ok(mut state) = placement.0.lock() else {
        return;
    };
    let Some(layout) = state.layout else {
        return;
    };
    let Some(hwnd) = window(app, "dock").and_then(|dock| handle(&dock)).ok() else {
        return;
    };
    let panel = app.state::<StartMenu>().panel();
    let behind_start = overlaps_start(layout, panel);
    if state.behind_start != behind_start
        && unsafe {
            SetWindowPos(
                hwnd,
                if behind_start {
                    HWND_NOTOPMOST
                } else {
                    HWND_TOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOMOVE,
            )
        } != 0
    {
        state.behind_start = behind_start;
    }
    let settings = app.state::<DockStore>().current();
    let start_open = app.state::<SystemPanel>().start_active();
    let drag = app.state::<WidgetDrag>();
    let dragging = drag.active();
    if dragging {
        let bounds = layout.body;
        let inside = cursor.x >= bounds.left - 18
            && cursor.x < bounds.left + bounds.width + 18
            && cursor.y >= bounds.top - 16
            && cursor.y < bounds.top + bounds.height + 16;
        drag.track(
            app,
            cursor,
            (cursor.x - layout.window.left) as f64 / layout.scale,
            (cursor.y - layout.window.top) as f64 / layout.scale,
            inside,
        );
    }
    if !layout.has_content && !dragging {
        if state.revealed {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
            let _ = app.emit_to("dock", "dock-revealed", false);
        }
        state.revealed = false;
        state.hide_at = None;
        hide_dock_effects(app, &mut state);
        return;
    }
    if !settings.auto_hide {
        state.start_revealed = false;
        set_revealed(app, hwnd, &mut state, true, settings.reveal_duration);
        state.idle_since = None;
        return;
    }
    let monitor = layout.monitor;
    let right = monitor.left + monitor.width;
    let bottom = monitor.top + monitor.height;
    let on_monitor = cursor.x >= monitor.left
        && cursor.x < right
        && cursor.y >= monitor.top
        && cursor.y < bottom;
    let edge = on_monitor
        && match layout.edge {
            DockEdge::Bottom => cursor.y >= bottom - 3,
            DockEdge::Top => cursor.y < monitor.top + 3,
            DockEdge::Left => cursor.x < monitor.left + 3,
            DockEdge::Right => cursor.x >= right - 3,
        };
    let mut hit = if state.expanded {
        layout.window
    } else {
        layout.body
    };
    match layout.edge {
        DockEdge::Bottom => hit.height = bottom - hit.top,
        DockEdge::Top => {
            hit.height += hit.top - monitor.top;
            hit.top = monitor.top;
        }
        DockEdge::Left => {
            hit.width += hit.left - monitor.left;
            hit.left = monitor.left;
        }
        DockEdge::Right => hit.width = right - hit.left,
    }
    let in_dock_path = on_monitor
        && cursor.x >= hit.left - layout.margin
        && cursor.x < hit.left + hit.width + layout.margin
        && cursor.y >= hit.top - layout.margin
        && cursor.y < hit.top + hit.height + layout.margin;
    let now = Instant::now();
    if std::mem::replace(&mut state.start_revealed, panel.is_some()) && panel.is_none() {
        state.idle_since = Some(now - Duration::from_millis(450));
    }
    if edge
        || dragging
        || panel.is_some()
        || start_open
        || app.state::<PopupState>().editing()
        || (state.revealed && (in_dock_path || popup_open))
    {
        state.idle_since = None;
        set_revealed(app, hwnd, &mut state, true, settings.reveal_duration);
    } else if state.revealed
        && now.duration_since(*state.idle_since.get_or_insert(now)) >= Duration::from_millis(450)
    {
        set_revealed(app, hwnd, &mut state, false, settings.reveal_duration);
    }
    if state.hide_at.is_some_and(|at| now >= at) {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        state.hide_at = None;
        hide_dock_effects(app, &mut state);
    }
}

fn track_popup_focus(app: &AppHandle, cursor: POINT) {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return;
    }
    let focused = |name| {
        window(app, name)
            .and_then(|surface| handle(&surface))
            .is_ok_and(|hwnd| hwnd == foreground)
    };
    let folder = app.state::<OpenFolder>();
    let system = app.state::<SystemPanel>();
    let folder_focused = folder.active() && focused("folder");
    // The color and date pickers can give focus to a dialog owned by the panel.
    let owner = unsafe { GetAncestor(foreground, GA_ROOTOWNER) };
    let system_focused = system.active()
        && window(app, "system")
            .and_then(|surface| handle(&surface))
            .is_ok_and(|hwnd| hwnd == foreground || hwnd == owner);
    let context_focused = popups::track_menu_focus(app, foreground);
    let root = unsafe { GetAncestor(WindowFromPoint(cursor), GA_ROOT) };
    // A press can switch focus before the widget receives its click. Keep its toggle alive until then.
    if system.active()
        && !(system_focused
            || (system.from_folder() && folder_focused)
            || (!context_focused && system.points_at_trigger(cursor, root)))
    {
        if system.start_active() {
            app.state::<StartMenu>().note_spacebar_focus_loss();
        }
        system_panel::close_system_panel(app.clone(), app.state());
    }
    if folder.active()
        && !(folder_focused
            || (system_focused && system.from_folder())
            || (app.state::<WidgetPopup>().from_folder() && focused("widget-popup"))
            || context_focused
            || folder.points_at_trigger(cursor, root))
    {
        let _ = widgets::close_folder(app.clone(), folder);
    }
}

fn watch_pointer(app: AppHandle) {
    thread::spawn(move || {
        app.state::<Previews>().attach_watcher();
        let dock = window(&app, "dock").and_then(|dock| handle(&dock)).ok();
        let previous = app.state::<PreviousForeground>();
        let queued = Arc::new(AtomicBool::new(false));
        let mut sampled_foreground = None;
        loop {
            let foreground = unsafe { GetForegroundWindow() };
            if !foreground.is_null() && Some(foreground) != dock {
                previous.0.store(foreground as usize, Ordering::Relaxed);
            }
            let preview_active = app.state::<Previews>().active();
            let widget_popup_active = app.state::<WidgetPopup>().active();
            let popup_active = app.state::<SystemPanel>().active()
                || app.state::<OpenFolder>().active()
                || app.state::<PopupState>().menu_open();
            let dragging = app.state::<WidgetDrag>().active();
            let focus_changed = sampled_foreground != Some(foreground);
            let active = focus_changed
                || popup_active
                || app.state::<DockStore>().current().auto_hide
                || app.state::<StartMenu>().tracking()
                || preview_active
                || widget_popup_active
                || dragging
                || app.state::<Placement>().0.lock().is_ok_and(|state| {
                    state.behind_start
                        || state.start_revealed
                        || state.hide_at.is_some()
                        || state
                            .layout
                            .is_some_and(|layout| layout.has_content != state.revealed)
                });
            if active && !queued.swap(true, Ordering::AcqRel) {
                sampled_foreground = Some(foreground);
                let handle = app.clone();
                let outstanding = queued.clone();
                if app
                    .run_on_main_thread(move || {
                        if let Some(action) = handle.state::<StartMenu>().next_spacebar_action() {
                            let show = action == start_menu::ShortcutAction::Open;
                            if let Err(error) = system_panel::set_start_visible(&handle, show) {
                                eprintln!("Couldn’t update Spacebar Start. {error}");
                            }
                        }
                        let mut cursor = POINT::default();
                        if unsafe { GetCursorPos(&mut cursor) } != 0 {
                            if focus_changed || popup_active {
                                track_popup_focus(&handle, cursor);
                            }
                            let preview_open = handle.state::<Previews>().track(&handle, cursor);
                            let widget_popup_open =
                                handle.state::<WidgetPopup>().track(&handle, cursor);
                            track_dock(
                                &handle,
                                cursor,
                                preview_open
                                    || widget_popup_open
                                    || handle.state::<OpenFolder>().active()
                                    || handle.state::<PopupState>().menu_open()
                                    || handle.state::<SystemPanel>().active(),
                            );
                        }
                        outstanding.store(false, Ordering::Release);
                    })
                    .is_err()
                {
                    break;
                }
            }
            thread::park_timeout(Duration::from_millis(
                if preview_active || widget_popup_active || dragging {
                    16
                } else if active {
                    30
                } else {
                    80
                },
            ));
        }
    });
}

#[tauri::command]
fn set_dock_expanded(
    app: AppHandle,
    placement: State<'_, Placement>,
    expanded: bool,
) -> Result<(), String> {
    let mut state = placement.0.lock().map_err(|error| error.to_string())?;
    if state.expanded != expanded {
        if let Some(layout) = state.layout {
            apply_dock_region(handle(&window(&app, "dock")?)?, layout, expanded)?;
        }
        state.expanded = expanded;
    }
    Ok(())
}

pub fn run(dock_only: bool) {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(crate::updates::Updates::default())
        .manage(TaskbarOrder::default())
        .manage(Replacement::default())
        .manage(StartMenu::default())
        .manage(StartApps::default())
        .manage(SystemPanel::default())
        .manage(PopupState::default())
        .manage(Previews::default())
        .manage(MediaState::default())
        .manage(connectivity::BluetoothConnections::default())
        .manage(WidgetPopup::default())
        .manage(Telemetry::default())
        .manage(system_widgets::audio_spectrum::AudioSpectrum::default())
        .manage(system_widgets::spotify_meter::SpotifyMeter::default())
        .manage(WidgetDrag::default())
        .manage(OpenFolder::default())
        .manage(Placement::default())
        .manage(DockEffectsWindow::default())
        .manage(PreviousForeground::default())
        .manage(SettingsWindow::default())
        .setup(move |app| {
            app.manage(DockStore::load(app.handle())?);
            app.manage(AppCatalog::load(app.handle())?);
            app.manage(WidgetContentStore::new(app.path().app_config_dir()?));
            for config in &app.config().app.windows {
                if config.label == "dock-effects" || (dock_only && config.label == "settings") {
                    continue;
                }
                let surface = WebviewWindowBuilder::from_config(app, config)?.build()?;
                if matches!(
                    surface.label(),
                    "dock" | "preview" | "peek" | "drag" | "widget-popup" | "system"
                ) {
                    if let Ok(hwnd) = surface.hwnd() {
                        suppress_frame(hwnd.0 as HWND, surface.label() != "system");
                    }
                }
            }
            let open = MenuItem::with_id(app, "settings", "Open settings", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Spacebar", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Spacebar")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => {
                        let _ = open_settings(app.clone(), None);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        let _ = open_settings(tray.app_handle().clone(), None);
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            for label in ["peek", "drag"] {
                if let Some(surface) = app.get_webview_window(label) {
                    surface.set_ignore_cursor_events(true)?;
                }
            }
            watch_pointer(app.handle().clone());
            start_menu::watch_keyboard(app.handle().clone());
            taskbar::watch(app.handle().clone());
            crate::updates::check_on_startup(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "context" && matches!(event, WindowEvent::Focused(true)) {
                let state = window.app_handle().state::<PopupState>();
                if state.menu_open() {
                    state.note_menu_focus();
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    api.prevent_close();
                    let app = window.app_handle();
                    app.state::<PopupState>().set_editing(app, false);
                    let _ = window.hide();
                    let _ = app.emit("dock-apps-changed", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::updates::read_update,
            crate::updates::download_update,
            crate::updates::install_update,
            dock_config::read_settings,
            dock_config::write_settings,
            dock_config::read_app_appearance,
            dock_config::write_app_appearance,
            dock_config::read_saved_docks,
            dock_config::edit_saved_docks,
            apps::list_dock_apps,
            apps::activate_app,
            apps::launch_app,
            apps::set_app_pinned,
            apps::move_dock_app,
            open_settings,
            read_settings_page,
            startup::read_startup,
            startup::set_startup,
            quit_spacebar,
            place_dock,
            set_dock_expanded,
            prepare_dock_effects,
            read_dock_effects,
            update_dock_effects,
            crate::surface_snow::set_surface_snow,
            previews::show_window_previews,
            previews::move_window_previews,
            previews::hide_window_previews,
            previews::read_preview_layout,
            previews::preview_media,
            previews::activate_preview_window,
            previews::close_preview_window,
            dock_config::read_widgets,
            dock_config::save_widgets,
            widget_content::read_widget_content,
            widget_content::write_widget_content,
            widget_content::read_agenda,
            widget_content::read_quick_tasks,
            widget_content::read_weather_preferences,
            widget_content::open_weather_source,
            screen_color::pick_screen_color,
            screen_color::copy_screen_color,
            widget_content::open_agenda_link,
            widgets::begin_widget_drag,
            widgets::read_drag_preview,
            widgets::read_open_folder,
            widgets::open_folder,
            widgets::close_folder,
            widgets::launch_folder_app,
            start_menu::toggle_start_menu,
            start_launcher::read_start_apps,
            start_launcher::read_start_preferences,
            start_launcher::save_start_preferences,
            start_launcher::launch_start_app,
            start_launcher::start_action,
            media::spotify_snapshot,
            media::spotify_control,
            media::open_spotify,
            widget_popup::read_widget_popup,
            widget_popup::read_widget_popup_anchor,
            widget_popup::show_widget_popup,
            widget_popup::move_widget_popup,
            widget_popup::close_widget_popup,
            widget_popup::finish_widget_popup_exit,
            telemetry::system_snapshot,
            system_widgets::audio_snapshot,
            system_widgets::audio_spectrum::audio_output_spectrum,
            system_widgets::spotify_meter::spotify_output_peak,
            system_widgets::audio_devices,
            system_widgets::select_audio_device,
            connectivity::wifi_networks,
            connectivity::wifi_connect,
            connectivity::bluetooth_devices,
            connectivity::bluetooth_connect,
            system_widgets::audio_control,
            system_widgets::system_action,
            system_panel::open_system_panel,
            system_panel::close_system_panel,
            system_panel::read_system_panel,
            system_panel::read_start_backdrop,
            system_panel::read_tray_icons,
            system_panel::activate_tray_icon,
            system_panel::reveal_tray_app,
            system_panel::read_tray_usb_drives,
            system_panel::eject_tray_usb_drive,
            popups::read_widget_editing,
            popups::set_widget_editing,
            popups::read_context_menu,
            popups::open_context_menu,
            popups::close_context_menu,
            popups::execute_app_menu_action,
        ])
        .build(tauri::generate_context!())
        .expect("failed to run Spacebar")
        .run(|app, event| {
            if matches!(event, RunEvent::Exit) {
                let _ = app.state::<Replacement>().update(false);
            }
        });
}
