use crate::apps::{AppCatalog, AppWindowActions};
use crate::desktop::{
    center, handle, monitor_info, popup_bounds, popup_space, screen_anchor, window, PopupAnchor,
};
use crate::previews;
use crate::{dock_config::DockStore, settings::DockEdge};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
        GetAncestor, GetWindow, SetWindowPos, GA_ROOT, GW_OWNER, HWND_TOPMOST, SWP_NOACTIVATE,
    },
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MenuTarget {
    Dock,
    App {
        id: String,
        name: String,
        icon: Option<String>,
        pinned: bool,
        can_pin: bool,
        folder_entry_id: Option<String>,
        #[serde(default)]
        running: bool,
        #[serde(default)]
        can_end_task: bool,
        #[serde(skip)]
        actions: Option<AppWindowActions>,
    },
    Widget {
        id: String,
        name: String,
        kind: Option<crate::widgets::Kind>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppMenuAction {
    Close,
    EndTask,
}

#[derive(Default)]
pub struct PopupState {
    menu: Mutex<Option<MenuTarget>>,
    menu_had_focus: AtomicBool,
    editing: AtomicBool,
}

impl PopupState {
    pub fn menu_open(&self) -> bool {
        self.menu.lock().is_ok_and(|menu| menu.is_some())
    }

    pub fn note_menu_focus(&self) {
        self.menu_had_focus.store(true, Ordering::Relaxed);
    }

    pub fn editing(&self) -> bool {
        self.editing.load(Ordering::Relaxed)
    }

    pub fn set_editing(&self, app: &AppHandle, editing: bool) {
        if self.editing.swap(editing, Ordering::Relaxed) != editing {
            let _ = app.emit_to("dock", "widget-editing", editing);
        }
    }
}

#[tauri::command]
pub fn read_widget_editing(state: State<'_, PopupState>) -> bool {
    state.editing()
}

#[tauri::command]
pub fn set_widget_editing(app: AppHandle, state: State<'_, PopupState>, editing: bool) {
    state.set_editing(&app, editing);
}

#[tauri::command]
pub fn read_context_menu(state: State<'_, PopupState>) -> Option<MenuTarget> {
    state.menu.lock().ok()?.clone()
}

#[tauri::command]
pub fn close_context_menu(app: AppHandle, state: State<'_, PopupState>) {
    if let Ok(mut menu) = state.menu.lock() {
        *menu = None;
    }
    state.menu_had_focus.store(false, Ordering::Relaxed);
    if let Ok(popup) = window(&app, "context") {
        let _ = popup.hide();
    }
    let _ = app.emit_to("context", "context-menu-opened", Option::<MenuTarget>::None);
    let _ = app.emit_to("dock", "context-menu-state", false);
}

pub(crate) fn track_menu_focus(app: &AppHandle, foreground: HWND) -> bool {
    let state = app.state::<PopupState>();
    if !state.menu_open() {
        return false;
    }
    let focused = window(app, "context")
        .and_then(|popup| handle(&popup))
        .is_ok_and(|popup| unsafe {
            // The WebView can have its own native popup, such as the color chooser.
            let mut owner = GetAncestor(foreground, GA_ROOT);
            while !owner.is_null() {
                if owner == popup {
                    return true;
                }
                owner = GetWindow(owner, GW_OWNER);
            }
            false
        });
    if focused {
        state.note_menu_focus();
    } else if state.menu_had_focus.load(Ordering::Relaxed) {
        close_context_menu(app.clone(), state);
    }
    focused
}

#[tauri::command]
pub fn open_context_menu(
    app: AppHandle,
    webview: WebviewWindow,
    state: State<'_, PopupState>,
    mut target: MenuTarget,
    anchor: PopupAnchor,
) -> Result<(), String> {
    if !matches!(webview.label(), "dock" | "folder") {
        return Err("The dock menu is unavailable here.".into());
    }
    let scale = webview.scale_factor().map_err(|error| error.to_string())?;
    let px = |value: f64| (value * scale).round() as i32;
    let anchor = screen_anchor(&webview, anchor, scale)?;
    let edge = if webview.label() == "dock" {
        app.state::<DockStore>().current().edge
    } else {
        DockEdge::Bottom
    };
    let area = monitor_info(center(anchor))?.rcMonitor;
    let space = popup_space(area, anchor, edge, px(5.0));
    if let MenuTarget::App {
        id,
        running,
        can_end_task,
        actions,
        ..
    } = &mut target
    {
        let catalog = app.state::<AppCatalog>();
        *actions = catalog.window_actions(id).filter(AppWindowActions::running);
        *running = actions.is_some();
        *can_end_task = actions
            .as_ref()
            .is_some_and(|actions| actions.can_end(&catalog));
    }
    let (logical_width, logical_height) = match &target {
        MenuTarget::Dock => (218.0, 110.0),
        MenuTarget::Widget { kind, .. } => match kind {
            Some(crate::widgets::Kind::Spacer) => (246.0, 238.0),
            Some(kind) => (
                246.0,
                179.0
                    + if kind.supports_size() { 59.0 } else { 0.0 }
                    + 72.0 * kind.color_picker_count() as f64
                    + if *kind == crate::widgets::Kind::AudioVisualizer { 192.0 } else { 0.0 }
                    + if kind.supports_surface() { 64.0 } else { 0.0 },
            ),
            None => (246.0, 384.0),
        },
        MenuTarget::App {
            pinned,
            can_pin,
            running,
            ..
        } => (
            218.0,
            110.0
                + if *pinned || *can_pin { 32.0 } else { 0.0 }
                + if *running { 68.0 } else { 0.0 },
        ),
    };
    let bounds = popup_bounds(space, anchor, edge, px(logical_width), px(logical_height));
    let popup = window(&app, "context")?;
    if unsafe {
        SetWindowPos(
            handle(&popup)?,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOACTIVATE,
        )
    } == 0
    {
        return Err("Couldn't position the dock menu.".into());
    }
    previews::close_all(&app);
    app.state::<crate::widget_popup::WidgetPopup>()
        .close(&app);
    state.menu_had_focus.store(false, Ordering::Relaxed);
    *state.menu.lock().map_err(|error| error.to_string())? = Some(target.clone());
    let _ = app.emit_to("context", "context-menu-opened", target);
    let _ = app.emit_to("dock", "context-menu-state", true);
    if let Err(error) = popup.show().and_then(|()| popup.set_focus()) {
        close_context_menu(app, state);
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn execute_app_menu_action(
    app: AppHandle,
    webview: WebviewWindow,
    state: State<'_, PopupState>,
    catalog: State<'_, AppCatalog>,
    id: String,
    action: AppMenuAction,
) -> Result<(), String> {
    if webview.label() != "context" {
        return Err("Open the app’s dock menu first.".into());
    }
    let actions = match state
        .menu
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
    {
        Some(MenuTarget::App {
            id: target,
            actions: Some(actions),
            ..
        }) if target == &id => actions.clone(),
        _ => return Err("This app menu has closed.".into()),
    };
    let result = match action {
        AppMenuAction::Close => actions.close(),
        AppMenuAction::EndTask => actions.end(&catalog),
    };
    let _ = app.emit("dock-apps-changed", ());
    result
}
