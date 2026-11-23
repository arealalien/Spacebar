use crate::{
    apps::{focus_window, request_close_window, AppCatalog, WindowInfo},
    desktop::{
        animations_enabled, center, follow_popup_bounds, handle, monitor_info, move_popup,
        popup_bounds, popup_space, screen_anchor, window, PopupAnchor,
    },
    dock_config::DockStore,
    media::{self, Action, MediaIdentity, PlaybackButtons},
    settings::{DockEdge, PreviewDisplay},
};
use serde::Serialize;
use std::{
    sync::{Mutex, OnceLock},
    thread::{self, Thread},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT, SIZE},
    Graphics::{
        Dwm::*,
        Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    },
    UI::WindowsAndMessaging::*,
};

const ENTER_MS: u64 = 160;
const EXIT_MS: u64 = 125;

#[derive(Default)]
pub struct Previews(Mutex<Option<Strip>>, OnceLock<Thread>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewLayout {
    token: String,
    open: bool,
    animated: bool,
    exit_opacity: f64,
    cards: Vec<PreviewCard>,
    playback: Option<PlaybackButtons>,
    controls_height: f64,
}

#[derive(Clone, Serialize)]
struct PreviewCard {
    id: String,
    title: String,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    header: f64,
    image: bool,
}

struct Thumbnail {
    window: WindowInfo,
    image: Option<(isize, SIZE)>,
    bounds: RECT,
}

struct Strip {
    layout: PreviewLayout,
    thumbnails: Vec<Thumbnail>,
    bounds: RECT,
    area: RECT,
    dock_monitor: RECT,
    scale: f64,
    gap: i32,
    anchor: RECT,
    icon: RECT,
    previous_bounds: Option<RECT>,
    peek: Option<isize>,
    selected: Option<usize>,
    leaving: bool,
    animation: Option<Animation>,
    progress: f64,
    slide: i32,
    edge: DockEdge,
    media: MediaIdentity,
    media_revision: u32,
}

struct Animation {
    start: Instant,
    duration: Duration,
    from: f64,
    to: f64,
}

fn surface(app: &AppHandle, label: &str) -> Result<HWND, String> {
    handle(&window(app, label)?)
}

fn contains(bounds: RECT, point: POINT) -> bool {
    point.x >= bounds.left
        && point.x < bounds.right
        && point.y >= bounds.top
        && point.y < bounds.bottom
}

fn translated(rect: RECT, x: i32, y: i32) -> RECT {
    RECT {
        left: rect.left + x,
        right: rect.right + x,
        top: rect.top + y,
        bottom: rect.bottom + y,
    }
}

fn project(edge: DockEdge, x: i32, y: i32) -> POINT {
    match edge {
        DockEdge::Bottom => POINT { x, y },
        DockEdge::Top => POINT { x, y: -y },
        DockEdge::Left => POINT { x: y, y: -x },
        DockEdge::Right => POINT { x: y, y: x },
    }
}

fn projected_rect(edge: DockEdge, rect: RECT) -> RECT {
    let a = project(edge, rect.left, rect.top);
    let b = project(edge, rect.right, rect.bottom);
    RECT {
        left: a.x.min(b.x),
        right: a.x.max(b.x),
        top: a.y.min(b.y),
        bottom: a.y.max(b.y),
    }
}

fn slide(bounds: RECT, edge: DockEdge, distance: i32) -> RECT {
    match edge {
        DockEdge::Bottom => translated(bounds, 0, distance),
        DockEdge::Top => translated(bounds, 0, -distance),
        DockEdge::Left => translated(bounds, -distance, 0),
        DockEdge::Right => translated(bounds, distance, 0),
    }
}

fn in_hover_path(panel: RECT, icon: RECT, point: POINT) -> bool {
    if contains(panel, point) || contains(icon, point) {
        return true;
    }
    if point.y < panel.bottom || point.y >= icon.bottom {
        return false;
    }
    let distance = (icon.bottom - panel.bottom).max(1);
    let progress = (point.y - panel.bottom) as f64 / distance as f64;
    let edge = |start: i32, end: i32| start as f64 + (end - start) as f64 * progress;
    let x = point.x as f64;
    x >= edge(panel.left, icon.left) && x < edge(panel.right, icon.right)
}

pub(crate) fn popup_hovered(edge: DockEdge, panel: RECT, anchor: RECT, cursor: POINT) -> bool {
    in_hover_path(
        projected_rect(edge, panel),
        projected_rect(edge, anchor),
        project(edge, cursor.x, cursor.y),
    )
}

fn hide(app: &AppHandle, label: &str) {
    if let Ok(hwnd) = surface(app, label) {
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn unregister(thumbnail: isize) {
    unsafe {
        DwmUnregisterThumbnail(thumbnail);
    }
}

fn close_peek(app: &AppHandle, strip: &mut Strip) {
    if let Some(thumbnail) = strip.peek.take() {
        hide(app, "peek");
        unregister(thumbnail);
    }
}

fn dismiss(app: &AppHandle, mut strip: Strip) {
    close_peek(app, &mut strip);
    hide(app, "preview");
    for card in strip.thumbnails {
        if let Some((thumbnail, _)) = card.image {
            unregister(thumbnail);
        }
    }
    let _ = app.emit_to("preview", "preview-layout", Option::<PreviewLayout>::None);
}

pub fn close_all(app: &AppHandle) {
    let previews = app.state::<Previews>();
    if let Ok(mut state) = previews.0.lock() {
        if let Some(strip) = state.take() {
            dismiss(app, strip);
        }
    };
}

fn register(destination: HWND, source: HWND) -> Option<(isize, SIZE)> {
    let mut thumbnail = 0;
    let mut size = SIZE::default();
    unsafe {
        if DwmRegisterThumbnail(destination, source, &mut thumbnail) < 0 {
            return None;
        }
        if DwmQueryThumbnailSourceSize(thumbnail, &mut size) < 0 || size.cx <= 0 || size.cy <= 0 {
            unregister(thumbnail);
            return None;
        }
    }
    Some((thumbnail, size))
}

fn fit(size: SIZE, width: i32, height: i32) -> (i32, i32) {
    let scale = (width.max(1) as f64 / size.cx as f64).min(height.max(1) as f64 / size.cy as f64);
    (
        (size.cx as f64 * scale).round().max(1.0) as i32,
        (size.cy as f64 * scale).round().max(1.0) as i32,
    )
}

fn set_image(thumbnail: isize, destination: RECT, source: Option<RECT>, opacity: u8) -> bool {
    let properties = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION
            | DWM_TNP_VISIBLE
            | DWM_TNP_OPACITY
            | if source.is_some() {
                DWM_TNP_RECTSOURCE
            } else {
                0
            },
        rcDestination: destination,
        rcSource: source.unwrap_or_default(),
        fVisible: 1,
        opacity,
        ..Default::default()
    };
    unsafe { DwmUpdateThumbnailProperties(thumbnail, &properties) >= 0 }
}

fn position(hwnd: HWND, bounds: RECT) -> bool {
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOACTIVATE,
        ) != 0
    }
}

fn icon_hit_bounds(edge: DockEdge, anchor: RECT, scale: f64, gap: i32) -> RECT {
    let mut icon = projected_rect(edge, anchor);
    let padding = (5.0 * scale).round() as i32;
    icon.left -= padding;
    icon.right += padding;
    icon.top -= gap;
    icon
}

fn grid(
    count: usize,
    width: i32,
    height: i32,
    ideal_width: i32,
    ideal_height: i32,
    gap: i32,
) -> (usize, i32, i32, i32) {
    let mut best = (1, 1, 1, gap);
    let mut best_area = 0;
    for columns in (1..=count).rev() {
        let rows = count.div_ceil(columns);
        let gutter = gap
            .min(width / columns as i32 / 4)
            .min(height / rows as i32 / 4);
        let cell_width =
            ((width - (columns as i32 - 1) * gutter) / columns as i32).min(ideal_width);
        let cell_height = ((height - (rows as i32 - 1) * gutter) / rows as i32).min(ideal_height);
        if cell_width * cell_height > best_area {
            best_area = cell_width * cell_height;
            best = (columns, cell_width, cell_height, gutter);
        }
    }
    best
}

struct PreviewGrid {
    count: usize,
    columns: usize,
    card_width: i32,
    card_height: i32,
    gap: i32,
    width: i32,
    height: i32,
    controls_height: i32,
}

impl PreviewGrid {
    fn new(count: usize, space: RECT, scale: f64, has_playback: bool) -> Self {
        let px = |value: f64| (value * scale).round() as i32;
        let height = space.bottom - space.top;
        let controls_height = if has_playback && height >= px(76.0) {
            px(38.0)
        } else {
            0
        };
        let footer = if controls_height > 0 {
            controls_height + px(8.0)
        } else {
            0
        };
        let (columns, card_width, card_height, gap) = grid(
            count,
            space.right - space.left,
            height - footer,
            px(260.0),
            px(186.0),
            px(8.0),
        );
        let rows = count.div_ceil(columns) as i32;
        Self {
            count,
            columns,
            card_width,
            card_height,
            gap,
            width: columns as i32 * (card_width + gap) - gap,
            height: rows * (card_height + gap) - gap + footer,
            controls_height,
        }
    }

    fn card(&self, index: usize) -> RECT {
        let row = index / self.columns;
        let length = (self.count - row * self.columns).min(self.columns) as i32;
        let stride = self.card_width + self.gap;
        let row_width = length * stride - self.gap;
        let left = (self.width - row_width) / 2 + (index % self.columns) as i32 * stride;
        let top = row as i32 * (self.card_height + self.gap);
        RECT {
            left,
            top,
            right: left + self.card_width,
            bottom: top + self.card_height,
        }
    }
}

#[tauri::command]
pub fn show_window_previews(
    app: AppHandle,
    catalog: State<'_, AppCatalog>,
    previews: State<'_, Previews>,
    token: String,
    app_id: String,
    anchor: PopupAnchor,
) -> Result<(), String> {
    if crate::desktop::dock_suppressed(&app) {
        return Ok(());
    }
    let source = catalog.preview_source(&app_id)?;
    if source.windows.is_empty() {
        return Err("This app has no open windows.".into());
    }
    let dock = window(&app, "dock")?;
    let scale = dock.scale_factor().map_err(|error| error.to_string())?;
    let px = |value: f64| (value * scale).round() as i32;
    let settings = app.state::<DockStore>().current();
    let edge = settings.edge;
    let anchor = screen_anchor(&dock, anchor, scale)?;
    let monitor = monitor_info(center(anchor))?;
    let area = if settings.replace_taskbar {
        monitor.rcMonitor
    } else {
        monitor.rcWork
    };
    let gap_from_dock = px(10.0).max(1);
    let window_surface = surface(&app, "preview")?;
    app.state::<crate::widget_popup::WidgetPopup>()
        .close(&app);
    let mut state = previews.0.lock().map_err(|error| error.to_string())?;
    if let Some(old) = state.take() {
        dismiss(&app, old);
    }
    let animated = animations_enabled();
    let lift = px(8.0);
    let thumbnails = source
        .windows
        .into_iter()
        .map(|(window, hwnd)| Thumbnail {
            window,
            image: register(window_surface, hwnd),
            bounds: RECT::default(),
        })
        .collect();
    let mut strip = Strip {
        layout: PreviewLayout {
            token,
            open: true,
            animated,
            exit_opacity: 1.0,
            cards: Vec::new(),
            playback: None,
            controls_height: 0.0,
        },
        thumbnails,
        bounds: RECT::default(),
        area,
        dock_monitor: monitor.rcMonitor,
        scale,
        gap: gap_from_dock,
        anchor,
        icon: icon_hit_bounds(edge, anchor, scale, gap_from_dock),
        previous_bounds: None,
        peek: None,
        selected: None,
        leaving: false,
        animation: animated.then(|| Animation {
            start: Instant::now(),
            duration: Duration::from_millis(ENTER_MS),
            from: 0.0,
            to: 1.0,
        }),
        progress: if animated { 0.0 } else { 1.0 },
        slide: lift,
        edge,
        media: source.media,
        media_revision: 0,
    };
    if let Err(error) = strip.relayout(window_surface) {
        dismiss(&app, strip);
        return Err(error);
    }
    unsafe {
        ShowWindow(window_surface, SW_SHOWNOACTIVATE);
    }
    let _ = app.emit_to("preview", "preview-layout", &strip.layout);
    *state = Some(strip);
    previews.wake_watcher();
    Ok(())
}

#[tauri::command]
pub fn move_window_previews(
    app: AppHandle,
    source: WebviewWindow,
    previews: State<'_, Previews>,
    token: String,
    anchor: PopupAnchor,
) -> Result<bool, String> {
    if source.label() != "dock" {
        return Ok(false);
    }
    let mut state = previews.0.lock().map_err(|error| error.to_string())?;
    let Some(strip) = state
        .as_mut()
        .filter(|strip| strip.layout.open && strip.layout.token == token)
    else {
        return Ok(false);
    };
    let anchor = screen_anchor(&source, anchor, strip.scale)?;
    let space = popup_space(strip.area, anchor, strip.edge, strip.gap);
    let bounds = follow_popup_bounds(space, anchor, strip.edge, strip.bounds);
    if bounds.left != strip.bounds.left || bounds.top != strip.bounds.top {
        let hwnd = surface(&app, "preview")?;
        if !move_popup(hwnd, slide(bounds, strip.edge, strip.lift())) {
            return Err("Windows couldn’t move the preview panel.".into());
        }
        strip.bounds = bounds;
    }
    strip.anchor = anchor;
    strip.icon = icon_hit_bounds(strip.edge, anchor, strip.scale, strip.gap);
    Ok(true)
}

#[tauri::command]
pub fn read_preview_layout(previews: State<'_, Previews>) -> Option<PreviewLayout> {
    previews
        .0
        .lock()
        .ok()?
        .as_ref()
        .map(|strip| strip.layout.clone())
}

#[tauri::command]
pub async fn preview_media(
    app: AppHandle,
    webview: WebviewWindow,
    token: String,
    action: Option<Action>,
) -> Result<bool, String> {
    if webview.label() != "preview" {
        return Ok(false);
    }
    let (identity, revision) = {
        let previews = app.state::<Previews>();
        let mut state = previews.0.lock().map_err(|error| error.to_string())?;
        let Some(strip) = state.as_mut().filter(|strip| {
            strip.layout.open && strip.layout.token == token && strip.media.is_known()
        }) else {
            return Ok(false);
        };
        strip.media_revision = strip.media_revision.wrapping_add(1);
        (strip.media.clone(), strip.media_revision)
    };
    let playback = media::preview_playback(identity, action).await?;
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let previews = handle.state::<Previews>();
        let Ok(mut state) = previews.0.lock() else {
            return;
        };
        let Some(strip) = state.as_mut().filter(|strip| {
            strip.layout.open && strip.layout.token == token && strip.media_revision == revision
        }) else {
            return;
        };
        if strip.layout.playback == playback {
            return;
        }
        let resize = strip.layout.playback.is_some() != playback.is_some();
        let previous = strip.layout.playback;
        strip.layout.playback = playback;
        if resize {
            if let Err(error) = surface(&handle, "preview").and_then(|hwnd| strip.relayout(hwnd)) {
                strip.layout.playback = previous;
                eprintln!("Couldn’t resize the preview for playback controls. {error}");
                return;
            }
        }
        let _ = handle.emit_to("preview", "preview-layout", &strip.layout);
    })
    .map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
pub fn hide_window_previews(previews: State<'_, Previews>, token: String) {
    if let Ok(mut state) = previews.0.lock() {
        if let Some(strip) = state.as_mut().filter(|strip| strip.layout.token == token) {
            strip.leaving = true;
        }
    }
    previews.wake_watcher();
}

#[tauri::command]
pub fn activate_preview_window(
    app: AppHandle,
    previews: State<'_, Previews>,
    id: String,
) -> Result<(), String> {
    let mut state = previews.0.lock().map_err(|error| error.to_string())?;
    let source = state
        .as_ref()
        .and_then(|strip| strip.thumbnails.iter().find(|card| card.window.id == id))
        .and_then(|card| card.window.handle())
        .ok_or("This preview has closed.")?;
    if let Some(strip) = state.take() {
        dismiss(&app, strip);
    }
    if focus_window(source) {
        Ok(())
    } else {
        Err("Windows couldn’t focus this window.".into())
    }
}

#[tauri::command]
pub fn close_preview_window(
    app: AppHandle,
    webview: WebviewWindow,
    previews: State<'_, Previews>,
    id: String,
    token: String,
) -> Result<(), String> {
    if webview.label() != "preview" {
        return Err("Open the window’s preview first.".into());
    }
    let mut state = previews.0.lock().map_err(|error| error.to_string())?;
    let strip = state
        .as_mut()
        .filter(|strip| strip.layout.open && strip.layout.token == token)
        .ok_or("This preview has closed.")?;
    let index = strip
        .thumbnails
        .iter()
        .position(|card| card.window.id == id)
        .ok_or("This preview has closed.")?;
    let source = strip.thumbnails[index]
        .window
        .handle()
        .ok_or("This window has closed.")?;
    close_peek(&app, strip);
    strip.selected = Some(index);
    if !request_close_window(source) {
        return Err("Windows couldn’t close this window.".into());
    }
    let _ = app.emit("dock-apps-changed", ());
    Ok(())
}

fn original_bounds(hwnd: HWND, size: SIZE) -> Option<(RECT, RECT)> {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let mut bounds = RECT::default();
        if IsIconic(hwnd) != 0 {
            let mut placement = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            if GetWindowPlacement(hwnd, &mut placement) == 0 {
                return None;
            }
            if placement.flags & WPF_RESTORETOMAXIMIZED != 0 {
                return Some((info.rcWork, info.rcMonitor));
            }
            bounds = placement.rcNormalPosition;
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW == 0 {
                bounds = translated(
                    bounds,
                    info.rcWork.left - info.rcMonitor.left,
                    info.rcWork.top - info.rcMonitor.top,
                );
            }
        } else {
            let mut frame = RECT::default();
            let frame_result = DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut frame as *mut RECT).cast(),
                std::mem::size_of::<RECT>() as u32,
            );
            // Depending on the source style, DWM may omit invisible resize borders.
            if frame_result >= 0
                && frame.right - frame.left == size.cx
                && frame.bottom - frame.top == size.cy
            {
                bounds = frame;
            } else if GetWindowRect(hwnd, &mut bounds) == 0 {
                return None;
            }
        }
        (bounds.right > bounds.left && bounds.bottom > bounds.top)
            .then_some((bounds, info.rcMonitor))
    }
}

fn clip_image(bounds: RECT, width: i32, height: i32, source: SIZE) -> Option<(RECT, RECT)> {
    let dest = RECT {
        left: bounds.left.max(0),
        top: bounds.top.max(0),
        right: bounds.right.min(width),
        bottom: bounds.bottom.min(height),
    };
    if dest.right <= dest.left || dest.bottom <= dest.top {
        return None;
    }
    let x = |at: i32| {
        ((at - bounds.left) as f64 * source.cx as f64 / (bounds.right - bounds.left) as f64).round()
            as i32
    };
    let y = |at: i32| {
        ((at - bounds.top) as f64 * source.cy as f64 / (bounds.bottom - bounds.top) as f64).round()
            as i32
    };
    let crop = RECT {
        left: x(dest.left),
        top: y(dest.top),
        right: x(dest.right),
        bottom: y(dest.bottom),
    };
    Some((dest, crop))
}

struct PeekPlacement {
    monitor: RECT,
    destination: RECT,
    crop: RECT,
}

fn peek_placement(
    display: PreviewDisplay,
    bounds: RECT,
    source_monitor: RECT,
    dock_monitor: RECT,
    size: SIZE,
) -> Option<PeekPlacement> {
    let monitor = match display {
        PreviewDisplay::Window => source_monitor,
        PreviewDisplay::Dock => dock_monitor,
    };
    let relative = translated(bounds, -source_monitor.left, -source_monitor.top);
    let (destination, crop) = clip_image(
        relative,
        monitor.right - monitor.left,
        monitor.bottom - monitor.top,
        size,
    )?;
    Some(PeekPlacement {
        monitor,
        destination,
        crop,
    })
}

fn show_peek(app: &AppHandle, strip: &mut Strip, index: usize) -> Option<isize> {
    let hwnd = surface(app, "peek").ok()?;
    let card = surface(app, "preview").ok()?;
    let dock = surface(app, "dock").ok()?;
    let source = strip.thumbnails[index].window.handle()?;
    let (thumbnail, size) = register(hwnd, source)?;
    let display = app.state::<DockStore>().current().preview_display;
    let placement = original_bounds(source, size).and_then(|(bounds, source_monitor)| {
        peek_placement(display, bounds, source_monitor, strip.dock_monitor, size)
    });
    let render = placement.is_some_and(|placement| {
        position(hwnd, placement.monitor)
            && set_image(thumbnail, placement.destination, Some(placement.crop), 255)
    });
    if !render {
        unregister(thumbnail);
        return None;
    }
    if let Some(previous) = strip.peek.take() {
        unregister(previous);
    }
    unsafe {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        SetWindowPos(dock, HWND_TOPMOST, 0, 0, 0, 0, flags);
        SetWindowPos(card, HWND_TOPMOST, 0, 0, 0, 0, flags);
    }
    Some(thumbnail)
}

impl Strip {
    fn relayout(&mut self, hwnd: HWND) -> Result<(), String> {
        let space = popup_space(self.area, self.anchor, self.edge, self.gap);
        let grid = PreviewGrid::new(
            self.thumbnails.len(),
            space,
            self.scale,
            self.layout.playback.is_some(),
        );
        let bounds = popup_bounds(space, self.anchor, self.edge, grid.width, grid.height);
        if !position(hwnd, slide(bounds, self.edge, self.lift())) {
            return Err("Windows couldn’t position the preview panel.".into());
        }
        if grid.width < self.bounds.right - self.bounds.left
            || grid.height < self.bounds.bottom - self.bounds.top
        {
            // Keep a path back to the remaining cards if the pointer is on the disappearing area.
            self.previous_bounds = Some(slide(self.bounds, self.edge, self.lift()));
        }
        self.bounds = bounds;
        let px = |value: f64| (value * self.scale).round() as i32;
        let logical = |value: i32| value as f64 / self.scale;
        let header = px(27.0).min(grid.card_height / 2);
        let padding = px(7.0).min(grid.card_width / 6).min(grid.card_height / 8);
        let opacity = (self.progress * 255.0).round() as u8;
        self.layout.controls_height = logical(grid.controls_height);
        self.layout.cards = self
            .thumbnails
            .iter_mut()
            .enumerate()
            .map(|(index, card)| {
                let rect = grid.card(index);
                card.bounds = rect;
                if let Some((thumbnail, size)) = card.image {
                    let content_height = grid.card_height - header - padding;
                    let (width, height) = fit(size, grid.card_width - padding * 2, content_height);
                    let left = rect.left + (grid.card_width - width) / 2;
                    let top = rect.top + header + (content_height - height) / 2;
                    let image = RECT {
                        left,
                        top,
                        right: left + width,
                        bottom: top + height,
                    };
                    if !set_image(thumbnail, image, None, opacity) {
                        unregister(thumbnail);
                        card.image = None;
                    }
                }
                PreviewCard {
                    id: card.window.id.clone(),
                    title: card.window.title.clone(),
                    left: logical(rect.left),
                    top: logical(rect.top),
                    width: logical(grid.card_width),
                    height: logical(grid.card_height),
                    header: logical(header),
                    image: card.image.is_some(),
                }
            })
            .collect();
        Ok(())
    }

    fn remove_closed_windows(&mut self, app: &AppHandle) {
        let previous_count = self.thumbnails.len();
        self.thumbnails.retain(|card| {
            if card.window.handle().is_some() {
                return true;
            }
            if let Some((thumbnail, _)) = card.image {
                unregister(thumbnail);
            }
            false
        });
        if self.thumbnails.len() == previous_count {
            return;
        }
        if self.thumbnails.is_empty() {
            self.layout.cards.clear();
            self.layout.playback = None;
            self.exit(app);
            return;
        }
        if let Err(error) = surface(app, "preview").and_then(|hwnd| self.relayout(hwnd)) {
            eprintln!("{error}");
            self.exit(app);
            return;
        }
        self.selected = None;
        close_peek(app, self);
        let _ = app.emit_to("preview", "preview-layout", &self.layout);
    }

    fn exit(&mut self, app: &AppHandle) {
        if !self.layout.open {
            return;
        }
        self.layout.open = false;
        self.layout.exit_opacity = self.progress;
        close_peek(app, self);
        self.animation = self.layout.animated.then(|| Animation {
            start: Instant::now(),
            duration: Duration::from_millis(EXIT_MS),
            from: self.progress,
            to: 0.0,
        });
        if self.animation.is_none() {
            self.progress = 0.0;
        }
        let _ = app.emit_to("preview", "preview-layout", &self.layout);
    }

    fn advance(&mut self, app: &AppHandle) -> bool {
        if let Some(animation) = &self.animation {
            let t = (animation.start.elapsed().as_secs_f64() / animation.duration.as_secs_f64())
                .min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            self.progress = animation.from + (animation.to - animation.from) * eased;
            if let Ok(hwnd) = surface(app, "preview") {
                move_popup(hwnd, slide(self.bounds, self.edge, self.lift()));
            }
            let opacity = (self.progress * 255.0).round() as u8;
            let properties = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY,
                opacity,
                ..Default::default()
            };
            for (item, _) in self.thumbnails.iter().filter_map(|card| card.image) {
                unsafe {
                    DwmUpdateThumbnailProperties(item, &properties);
                }
            }
            if t == 1.0 {
                self.animation = None;
            }
        }
        self.layout.open || self.progress > 0.0
    }

    fn lift(&self) -> i32 {
        ((1.0 - self.progress) * self.slide as f64).round() as i32
    }
}

impl Previews {
    pub fn attach_watcher(&self) {
        let _ = self.1.set(thread::current());
    }

    pub(crate) fn wake_watcher(&self) {
        if let Some(thread) = self.1.get() {
            thread.unpark();
        }
    }

    pub fn active(&self) -> bool {
        self.0.lock().is_ok_and(|state| state.is_some())
    }

    pub fn track(&self, app: &AppHandle, cursor: POINT) -> bool {
        let Ok(mut state) = self.0.lock() else {
            return false;
        };
        let Some(strip) = state.as_mut() else {
            return false;
        };
        if strip.layout.open {
            strip.remove_closed_windows(app);
        }
        let painted = slide(strip.bounds, strip.edge, strip.lift());
        let over_panel = contains(painted, cursor);
        let over_previous = strip
            .previous_bounds
            .is_some_and(|bounds| contains(bounds, cursor));
        if over_panel || !over_previous {
            strip.previous_bounds = None;
        }
        if over_panel {
            strip.leaving = true;
        }
        if strip.layout.open
            && strip.leaving
            && !over_panel
            && !over_previous
            && !in_hover_path(
                projected_rect(strip.edge, strip.bounds),
                strip.icon,
                project(strip.edge, cursor.x, cursor.y),
            )
        {
            strip.exit(app);
        }
        if strip.layout.open {
            let local = POINT {
                x: cursor.x - painted.left,
                y: cursor.y - painted.top,
            };
            let selected = strip
                .thumbnails
                .iter()
                .position(|card| contains(card.bounds, local));
            if selected != strip.selected {
                strip.selected = selected;
                if let Some(thumbnail) = selected.and_then(|index| show_peek(app, strip, index)) {
                    strip.peek = Some(thumbnail);
                } else {
                    close_peek(app, strip);
                }
            }
        }
        if !strip.advance(app) {
            if let Some(strip) = state.take() {
                dismiss(app, strip);
            }
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_follows_its_icon_and_keeps_the_same_thumbnail_size() {
        let area = RECT {
            left: -1000,
            top: 50,
            right: 0,
            bottom: 850,
        };
        let original = RECT {
            left: -760,
            top: 560,
            right: -500,
            bottom: 746,
        };
        let anchor = RECT {
            left: -502,
            top: 726,
            right: -438,
            bottom: 790,
        };
        let space = popup_space(area, anchor, DockEdge::Bottom, 10);
        let moved = follow_popup_bounds(space, anchor, DockEdge::Bottom, original);
        assert_eq!(center(moved).x, center(anchor).x);
        assert_eq!(moved.bottom, anchor.top - 10);
        assert_eq!(
            (moved.right - moved.left, moved.bottom - moved.top),
            (260, 186)
        );
        let hit = icon_hit_bounds(DockEdge::Bottom, anchor, 1.0, 10);
        assert!(in_hover_path(moved, hit, POINT { x: -470, y: 721 }));
    }

    #[test]
    fn a_preview_keeps_its_size_and_stays_on_screen_when_the_gap_shrinks() {
        let area = RECT {
            left: 0,
            top: 0,
            right: 500,
            bottom: 500,
        };
        let original = RECT {
            left: 70,
            top: 10,
            right: 330,
            bottom: 196,
        };
        let anchor = RECT {
            left: 170,
            top: 50,
            right: 290,
            bottom: 125,
        };
        let space = popup_space(area, anchor, DockEdge::Left, 10);
        let moved = follow_popup_bounds(space, anchor, DockEdge::Left, original);
        assert_eq!(
            (moved.left, moved.top, moved.right, moved.bottom),
            (230, 10, 490, 196)
        );
    }

    #[test]
    fn remaining_previews_and_partial_rows_are_centered_with_playback_controls() {
        let space = RECT {
            left: 0,
            top: 0,
            right: 600,
            bottom: 500,
        };
        let anchor = RECT {
            left: 275,
            top: 510,
            right: 325,
            bottom: 560,
        };
        let three = PreviewGrid::new(3, space, 1.0, true);
        assert_eq!(three.columns, 2);
        assert_eq!(center(three.card(2)).x, three.width / 2);
        assert!(three.card(2).bottom < three.height - three.controls_height);
        let one = PreviewGrid::new(1, space, 1.0, true);
        assert_eq!(one.card(0).left, 0);
        assert_eq!(one.card(0).right, one.width);
        assert_eq!(one.controls_height, 38);
        let bounds = popup_bounds(space, anchor, DockEdge::Bottom, one.width, one.height);
        assert_eq!(center(bounds).x, center(anchor).x);
    }

    #[test]
    fn the_gap_stays_interactive_without_extending_to_neighboring_icons() {
        let panel = RECT {
            left: 100,
            right: 320,
            top: 80,
            bottom: 200,
        };
        let icon = RECT {
            left: 190,
            right: 230,
            top: 225,
            bottom: 260,
        };
        assert!(in_hover_path(panel, icon, POINT { x: 213, y: 218 }));
        assert!(in_hover_path(panel, icon, POINT { x: 275, y: 205 }));
        assert!(!in_hover_path(panel, icon, POINT { x: 280, y: 252 }));
        assert!(!in_hover_path(panel, icon, POINT { x: 213, y: 265 }));
    }

    #[test]
    fn oversized_window_is_cropped_at_its_original_scale() {
        let bounds = RECT {
            left: -40,
            top: 10,
            right: 860,
            bottom: 610,
        };
        let (dest, crop) = clip_image(bounds, 800, 500, SIZE { cx: 900, cy: 600 }).unwrap();
        assert_eq!(
            (dest.left, dest.top, dest.right, dest.bottom),
            (0, 10, 800, 500)
        );
        assert_eq!(
            (crop.left, crop.top, crop.right, crop.bottom),
            (40, 0, 840, 490)
        );
    }

    #[test]
    fn full_size_preview_keeps_the_window_position_on_either_display() {
        let source_monitor = RECT {
            left: -2560,
            top: -360,
            right: 0,
            bottom: 1080,
        };
        let dock_monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let window = RECT {
            left: -2310,
            top: -280,
            right: -710,
            bottom: 520,
        };
        let size = SIZE { cx: 1600, cy: 800 };
        let original = peek_placement(
            PreviewDisplay::Window,
            window,
            source_monitor,
            dock_monitor,
            size,
        )
        .unwrap();
        assert_eq!(
            (
                original.monitor.left,
                original.monitor.top,
                original.monitor.right,
                original.monitor.bottom
            ),
            (-2560, -360, 0, 1080)
        );
        let screen = translated(
            original.destination,
            original.monitor.left,
            original.monitor.top,
        );
        assert_eq!(
            (screen.left, screen.top, screen.right, screen.bottom),
            (-2310, -280, -710, 520)
        );
        assert_eq!(
            (
                original.crop.left,
                original.crop.top,
                original.crop.right,
                original.crop.bottom
            ),
            (0, 0, 1600, 800)
        );

        let mirrored = peek_placement(
            PreviewDisplay::Dock,
            window,
            source_monitor,
            dock_monitor,
            size,
        )
        .unwrap();
        let screen = translated(
            mirrored.destination,
            mirrored.monitor.left,
            mirrored.monitor.top,
        );
        assert_eq!(
            (screen.left, screen.top, screen.right, screen.bottom),
            (250, 80, 1850, 880)
        );
    }

    #[test]
    fn window_monitor_preview_fits_positions_past_the_docks_display_bounds() {
        let source_monitor = RECT {
            left: 1920,
            top: -720,
            right: 4480,
            bottom: 720,
        };
        let dock_monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let window = RECT {
            left: 3920,
            top: -620,
            right: 4440,
            bottom: -20,
        };
        let size = SIZE { cx: 520, cy: 600 };
        let preview = peek_placement(
            PreviewDisplay::Window,
            window,
            source_monitor,
            dock_monitor,
            size,
        )
        .unwrap();
        assert_eq!(
            (preview.destination.left, preview.destination.top),
            (2000, 100)
        );
        assert!(peek_placement(
            PreviewDisplay::Dock,
            window,
            source_monitor,
            dock_monitor,
            size
        )
        .is_none());
    }

    #[test]
    fn multiple_windows_fit_on_the_monitor() {
        let (columns, width, height, gap) = grid(18, 1000, 420, 260, 186, 8);
        let rows = 18_usize.div_ceil(columns) as i32;
        assert!(columns as i32 * (width + gap) - gap <= 1000);
        assert!(rows * (height + gap) - gap <= 420);
        assert!(width > 80 && height > 60);
    }
}
