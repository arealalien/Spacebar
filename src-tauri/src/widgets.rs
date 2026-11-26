use crate::{
    apps::{self, AppCatalog},
    desktop::{
        center, handle, monitor_info, popup_bounds, popup_space, screen_anchor, window,
        PanelTrigger, PopupAnchor,
    },
    dock_config::DockStore,
    previews::{self, Previews},
    widget_image::ImageWidget,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    Apps,
    Clock,
    Spotify,
    SpotifyCard,
    SpotifyMini,
    SpotifyText,
    ClockPlain,
    ClockOutline,
    ClockAnalog,
    ClockTwilight,
    ClockFlip,
    DayProgress,
    QuickNote,
    DrawingPad,
    Image,
    Agenda,
    QuickTasks,
    ColorPicker,
    Weather,
    WeatherColorful,
    #[serde(rename = "weather-3d")]
    Weather3d,
    WeatherAmbient,
    AudioVisualizer,
    #[serde(rename = "desk-pet")]
    RemovedDeskPet,
    Volume,
    Microphone,
    Mixer,
    Cpu,
    Ram,
    Gpu,
    Performance,
    SystemRings,
    Network,
    NetworkUsage,
    Storage,
    Tray,
    ShowDesktop,
    Notifications,
    QuickSettings,
    Keyboard,
    Display,
    NotificationSettings,
    Capture,
    Start,
    SpacebarStart,
    Folder,
    WidgetGroup,
    WidgetTray,
    App,
    Spacer,
    Divider,
}

impl Kind {
    pub fn is_clock(self) -> bool {
        matches!(self, Self::Clock | Self::ClockPlain | Self::ClockOutline | Self::ClockAnalog
            | Self::ClockTwilight | Self::ClockFlip | Self::DayProgress)
    }
    pub fn is_widget_container(self) -> bool {
        matches!(self, Self::WidgetGroup | Self::WidgetTray)
    }

    pub fn supports_surface(self) -> bool {
        !matches!(
            self,
            Self::Start | Self::SpacebarStart | Self::Spacer | Self::Divider | Self::Image
        )
    }

    pub fn supports_size(self) -> bool {
        !matches!(self, Self::App | Self::Spacer)
    }

    pub fn supports_color(self) -> bool {
        self.supports_size() && !matches!(self, Self::Apps | Self::Image) && !self.is_widget_container()
    }

    fn color_channels(self) -> &'static [ColorChannel] {
        use ColorChannel::*;
        match self {
            Self::Performance => &[Cpu, Ram, Gpu],
            Self::SystemRings => &[Cpu, Ram],
            Self::Network | Self::NetworkUsage => &[Download, Upload],
            Self::DayProgress => &[Elapsed, Current, Upcoming],
            Self::ClockAnalog | Self::ClockTwilight => &[Hands, Dial],
            _ => &[],
        }
    }

    pub fn color_picker_count(self) -> usize {
        self.color_channels()
            .len()
            .max(usize::from(self.supports_color()))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
enum ColorChannel {
    Cpu,
    Ram,
    Gpu,
    Download,
    Upload,
    Elapsed,
    Current,
    Upcoming,
    Hands,
    Dial,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum WidgetSize {
    Small,
    Normal,
    Large,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AudioSpeed {
    Slow,
    Normal,
    Fast,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AudioAlignment {
    Center,
    Bottom,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum AudioSensitivity {
    Low,
    Normal,
    High,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    id: String,
    kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    launch_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frame: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    no_padding: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_date: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clock_format: Option<ClockFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    child_spacing: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    child_padding: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spacer_size: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<WidgetSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio_speed: Option<AudioSpeed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio_alignment: Option<AudioAlignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio_sensitivity: Option<AudioSensitivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<ImageWidget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    colors: BTreeMap<ColorChannel, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<Item>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ClockFormat {
    System,
    #[serde(rename = "12h")]
    TwelveHour,
    #[serde(rename = "24h")]
    TwentyFourHour,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Layout {
    pub(crate) revision: u64,
    pub(crate) items: Vec<Item>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            revision: 0,
            items: vec![Item {
                id: "apps-row".into(),
                kind: Kind::Apps,
                name: None,
                app_id: None,
                launch_path: None,
                icon: None,
                background: None,
                frame: None,
                no_padding: None,
                show_date: None,
                clock_format: None,
                child_spacing: None,
                child_padding: None,
                spacer_size: None,
                size: None,
                audio_speed: None,
                audio_alignment: None,
                audio_sensitivity: None,
                image: None,
                color: None,
                colors: BTreeMap::new(),
                children: Vec::new(),
            }],
        }
    }
}

fn widget_entries(items: &[Item]) -> impl Iterator<Item = (&Item, Option<Kind>)> {
    items.iter().flat_map(|item| {
        std::iter::once((item, None)).chain(item.children.iter().flat_map(move |child| {
            std::iter::once((child, Some(item.kind)))
                .chain(child.children.iter().map(move |entry| (entry, Some(child.kind))))
        }))
    })
}

pub(crate) fn is_image_widget(items: &[Item], id: &str) -> bool {
    widget_entries(items).any(|(item, _)| item.kind == Kind::Image && item.id == id)
}

pub(crate) fn valid_color(color: &str) -> bool {
    color
        .strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|digit| digit.is_ascii_hexdigit()))
}

pub(crate) fn valid_layout(items: &[Item]) -> bool {
    if items.len() > 64 {
        return false;
    }
    let mut ids = HashSet::new();
    let mut kinds = HashSet::new();
    let mut apps = HashSet::new();
    for (item, parent) in widget_entries(items) {
        if item.id.is_empty() || item.id.len() > 80 || !ids.insert(&item.id) {
            return false;
        }
        if item.kind == Kind::Folder {
            if parent.is_some_and(|kind| !kind.is_widget_container()) || item.children.len() > 128 {
                return false;
            }
        } else if item.kind.is_widget_container() {
            if parent.is_some() || item.children.len() > 64 {
                return false;
            }
        } else if !item.children.is_empty() {
            return false;
        }
        if item.child_spacing.is_some_and(|value| !item.kind.is_widget_container() || value > 32)
            || item.child_padding.is_some_and(|value| !item.kind.is_widget_container() || value > 24)
        {
            return false;
        }
        if item.kind == Kind::App {
            let Some(id) = item.app_id.as_deref() else {
                return false;
            };
            if parent != Some(Kind::Folder)
                || !apps.insert(id)
                || item.icon.as_ref().is_some_and(|icon| {
                    icon.len() > 512 * 1024 || !icon.starts_with("data:image/png;base64,")
                })
            {
                return false;
            }
        } else if matches!(item.kind, Kind::Spacer | Kind::Divider) {
            if parent == Some(Kind::Folder) {
                return false;
            }
        } else if !matches!(item.kind, Kind::Folder | Kind::WidgetGroup | Kind::WidgetTray | Kind::ClockPlain | Kind::Image)
            && !kinds.insert(item.kind)
        {
            return false;
        }
        if item
            .spacer_size
            .is_some_and(|size| item.kind != Kind::Spacer || !(8..=240).contains(&size))
        {
            return false;
        }
        if item.size.is_some() && !item.kind.supports_size() {
            return false;
        }
        if !item.kind.is_clock() && (item.show_date.is_some() || item.clock_format.is_some()) {
            return false;
        }
        if item
            .image
            .as_ref()
            .is_some_and(|image| item.kind != Kind::Image || !image.valid())
        {
            return false;
        }
        if item.kind != Kind::AudioVisualizer
            && (item.audio_speed.is_some()
                || item.audio_alignment.is_some()
                || item.audio_sensitivity.is_some())
        {
            return false;
        }
        if item
            .color
            .as_ref()
            .is_some_and(|color| !item.kind.supports_color() || !valid_color(color))
            || item.colors.iter().any(|(channel, color)| {
                !item.kind.color_channels().contains(channel) || !valid_color(color)
            })
        {
            return false;
        }
    }
    true
}

pub(crate) fn migrate_widget_items(items: &mut Vec<Item>) {
    items.retain(|item| item.kind != Kind::RemovedDeskPet);
    for item in items {
        if item.kind == Kind::ClockOutline {
            item.kind = Kind::ClockPlain;
            item.frame.get_or_insert(true);
        }
        migrate_widget_items(&mut item.children);
    }
}

impl Layout {
    pub(crate) fn uses_spacebar_start(&self) -> bool {
        widget_entries(&self.items).any(|(item, _)| item.kind == Kind::SpacebarStart)
    }

    pub(crate) fn load_legacy(directory: &Path) -> Self {
        let mut layout = fs::read(directory.join("widgets.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Layout>(&bytes).ok())
            .unwrap_or_default();
        migrate_widget_items(&mut layout.items);
        if !valid_layout(&layout.items) {
            return Self::default();
        }
        let legacy_large = fs::read(directory.join("settings.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .is_some_and(|settings| {
                settings.get("widgetSize").and_then(Value::as_str) == Some("large")
            });
        if legacy_large && migrate_large_widgets(&mut layout.items) {
            layout.revision += 1;
        }
        layout
    }
}

fn migrate_large_widgets(items: &mut [Item]) -> bool {
    let mut changed = false;
    for item in items {
        if item.size.is_none()
            && !matches!(
                item.kind,
                Kind::App
                    | Kind::Apps
                    | Kind::Start
                    | Kind::SpacebarStart
                    | Kind::Spacer
                    | Kind::Divider
            )
        {
            item.size = Some(WidgetSize::Large);
            changed = true;
        }
        changed |= migrate_large_widgets(&mut item.children);
    }
    changed
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn saves_visualizer_options_and_accepts_existing_layouts() {
        for saved in [
            serde_json::json!([{ "id": "audio", "kind": "audio-visualizer", "audioSpeed": "fast" }]),
            serde_json::json!([{
                "id": "audio", "kind": "audio-visualizer", "audioSpeed": "fast",
                "audioAlignment": "bottom", "audioSensitivity": "high"
            }]),
        ] {
            let items: Vec<Item> = serde_json::from_value(saved.clone()).unwrap();
            assert!(valid_layout(&items));
            assert_eq!(serde_json::to_value(items).unwrap(), saved);
        }
    }

    #[test]
    fn merges_digital_clocks_without_losing_existing_styles_or_copies() {
        let mut items: Vec<Item> = serde_json::from_value(serde_json::json!([
            {"id": "plain", "kind": "clock-plain", "size": "large"},
            {"id": "outline", "kind": "clock-outline", "color": "#55bb00"},
            {"id": "folder", "kind": "folder", "children": [
                {"id": "filled", "kind": "clock-outline", "background": true, "frame": false}
            ]}
        ]))
        .unwrap();
        migrate_widget_items(&mut items);
        assert!(valid_layout(&items));
        let saved = serde_json::to_value(&items).unwrap();
        assert_eq!(saved[0]["size"], "large");
        assert!(saved[0].get("frame").is_none());
        assert_eq!(saved[1]["kind"], "clock-plain");
        assert_eq!(saved[1]["frame"], true);
        assert_eq!(saved[1]["color"], "#55bb00");
        assert_eq!(saved[2]["children"][0]["kind"], "clock-plain");
        assert_eq!(saved[2]["children"][0]["frame"], false);
        assert_eq!(saved[2]["children"][0]["background"], true);
    }

    #[test]
    fn removes_the_old_pet_without_discarding_its_folder_or_other_widgets() {
        let mut items: Vec<Item> = serde_json::from_value(serde_json::json!([
            {"id": "note", "kind": "quick-note", "size": "small"},
            {"id": "tools", "kind": "folder", "name": "Tools", "children": [
                {"id": "pet", "kind": "desk-pet"},
                {"id": "drawing", "kind": "drawing-pad", "frame": false}
            ]},
            {"id": "agenda", "kind": "agenda"}
        ]))
        .unwrap();
        migrate_widget_items(&mut items);
        assert!(valid_layout(&items));
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "note");
        assert_eq!(items[1].name.as_deref(), Some("Tools"));
        assert_eq!(items[1].children.len(), 1);
        assert_eq!(items[1].children[0].id, "drawing");
        assert_eq!(items[1].children[0].frame, Some(false));
        assert_eq!(items[2].id, "agenda");
    }

    #[test]
    fn clock_options_round_trip_on_all_clock_widgets() {
        let kinds = ["clock", "clock-plain", "clock-analog", "clock-twilight", "clock-flip", "day-progress"];
        let stored = serde_json::json!(kinds.map(|kind| serde_json::json!({
            "id": kind, "kind": kind, "showDate": false, "clockFormat": "24h"
        })));
        let items: Vec<Item> = serde_json::from_value(stored.clone()).unwrap();
        assert!(valid_layout(&items));
        assert_eq!(serde_json::to_value(items).unwrap(), stored);
    }

    #[test]
    fn validates_inline_widgets_with_folders_and_preserves_container_options() {
        let value = serde_json::json!([{
            "id": "group", "kind": "widget-group", "childSpacing": 0, "childPadding": 12,
            "children": [
                { "id": "divider", "kind": "divider" },
                { "id": "folder", "kind": "folder", "children": [
                    { "id": "start", "kind": "spacebar-start" },
                    { "id": "image", "kind": "image" },
                    { "id": "app", "kind": "app", "appId": "app-id" }
                ]}
            ]
        }, { "id": "tray", "kind": "widget-tray", "children": [{ "id": "tray-tool", "kind": "tray" }] }]);
        let items = serde_json::from_value::<Vec<Item>>(value).unwrap();
        assert!(valid_layout(&items));
        assert!(is_image_widget(&items, "image"));
        assert!(Layout { revision: 0, items: items.clone() }.uses_spacebar_start());
        let mut saved = serde_json::to_value(items).unwrap();
        assert_eq!(saved[0]["childSpacing"], 0);
        assert_eq!(saved[0]["childPadding"], 12);
        saved[0]["children"][0]["kind"] = serde_json::json!("widget-tray");
        assert!(!valid_layout(&serde_json::from_value::<Vec<Item>>(saved).unwrap()));
    }

    #[test]
    fn accepts_individual_colors_for_the_widgets_that_use_them() {
        let mut items: Vec<Item> = serde_json::from_value(serde_json::json!([
            {"id": "legacy", "kind": "performance", "color": "#6699CC"},
            {"id": "folder", "kind": "folder", "children": [
                {"id": "rings", "kind": "system-rings", "colors": {"cpu": "#1166AA", "ram": "#CC55CC"}}
            ]}
        ])).unwrap();
        assert!(valid_layout(&items));
        let saved = serde_json::to_value(&items).unwrap();
        assert_eq!(saved[0]["color"], "#6699CC");
        assert_eq!(saved[1]["children"][0]["colors"]["ram"], "#CC55CC");
        let rings = &mut items[1].children[0];
        rings.colors.insert(ColorChannel::Gpu, "#009933".into());
        assert!(!valid_layout(&items));
    }

    #[test]
    fn keeps_legacy_widget_sizes_without_overriding_individual_choices() {
        let mut items: Vec<Item> = serde_json::from_value(serde_json::json!([
            {"id": "apps", "kind": "apps"},
            {"id": "clock", "kind": "clock"},
            {"id": "start", "kind": "start"},
            {"id": "spacer", "kind": "spacer", "spacerSize": 48},
            {"id": "folder", "kind": "folder", "size": "small", "children": [
                {"id": "cpu", "kind": "cpu", "color": "#6F90AA"},
                {"id": "ram", "kind": "ram", "size": "normal"}
            ]}
        ]))
        .unwrap();
        assert!(valid_layout(&items));
        assert!(migrate_large_widgets(&mut items));
        assert!(!migrate_large_widgets(&mut items));
        assert_eq!(
            items.iter().map(|item| item.size).collect::<Vec<_>>(),
            [
                None,
                Some(WidgetSize::Large),
                None,
                None,
                Some(WidgetSize::Small)
            ]
        );
        assert_eq!(items[4].children[0].size, Some(WidgetSize::Large));
        assert_eq!(items[4].children[1].size, Some(WidgetSize::Normal));
        assert_eq!(items[4].children[0].color.as_deref(), Some("#6F90AA"));
        assert!(valid_layout(&items));
        items[4].children[0].color = Some("red".into());
        assert!(!valid_layout(&items));
    }
}

#[derive(Deserialize)]
pub struct DragSize {
    width: f64,
    height: f64,
}

struct ActiveDrag {
    source: Value,
    width: i32,
    height: i32,
    position: Option<(i32, i32)>,
}

#[derive(Default)]
pub struct WidgetDrag(Mutex<Option<ActiveDrag>>);

impl WidgetDrag {
    pub fn active(&self) -> bool {
        self.0.lock().is_ok_and(|drag| drag.is_some())
    }

    pub fn track(&self, app: &AppHandle, cursor: POINT, x: f64, y: f64, inside: bool, cancel: bool) {
        let Ok(mut current) = self.0.lock() else {
            return;
        };
        let Some(drag) = current.as_mut() else {
            return;
        };
        let cancelled = cancel || unsafe { GetAsyncKeyState(VK_ESCAPE as i32) < 0 };
        let finished = cancelled || unsafe { GetAsyncKeyState(VK_LBUTTON as i32) >= 0 };
        let position = (cursor.x, cursor.y);
        if finished || drag.position != Some(position) {
            let _ = app.emit_to("dock", "widget-drag", serde_json::json!({ "source": drag.source, "x": x, "y": y, "inside": inside, "finished": finished, "cancelled": cancelled }));
        }
        if finished {
            if let Ok(ghost) = window(app, "drag") {
                let _ = ghost.hide();
            }
            let _ = app.emit_to("drag", "drag-preview", Option::<Value>::None);
            *current = None;
        } else if drag.position != Some(position) {
            let first = drag.position.replace(position).is_none();
            if let Ok(ghost) = window(app, "drag") {
                if let Ok(hwnd) = handle(&ghost) {
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            cursor.x - drag.width / 2,
                            cursor.y - drag.height - 9,
                            drag.width,
                            drag.height,
                            SWP_NOACTIVATE | if first { SWP_SHOWWINDOW } else { SWP_NOSIZE },
                        );
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub fn begin_widget_drag(
    app: AppHandle,
    window: WebviewWindow,
    drag: State<'_, WidgetDrag>,
    source: Value,
    size: DragSize,
) -> Result<(), String> {
    if !matches!(window.label(), "dock" | "settings" | "folder")
        || !matches!(
            source.get("type").and_then(Value::as_str),
            Some("new" | "item" | "app")
        )
    {
        return Err("The widget drag could not start.".into());
    }
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let px = |value: f64| (value * scale).round() as i32;
    let width = px(size.width.clamp(24.0, 740.0) + 14.0);
    let height = px(size.height.clamp(24.0, 96.0) + 14.0);
    let _ = app.emit_to("drag", "drag-preview", &source);
    *drag.0.lock().map_err(|error| error.to_string())? = Some(ActiveDrag {
        source,
        width,
        height,
        position: None,
    });
    previews::close_all(&app);
    app.state::<crate::widget_popup::WidgetPopup>()
        .close(&app);
    app.state::<Previews>().wake_watcher();
    Ok(())
}

#[tauri::command]
pub fn read_drag_preview(drag: State<'_, WidgetDrag>) -> Option<Value> {
    drag.0.lock().ok()?.as_ref().map(|drag| drag.source.clone())
}

#[derive(Default)]
pub struct OpenFolder(Mutex<Option<FolderPopup>>);

struct FolderPopup {
    id: String,
    trigger: PanelTrigger,
}

impl OpenFolder {
    pub fn active(&self) -> bool {
        self.0.lock().is_ok_and(|folder| folder.is_some())
    }

    pub fn clear(&self) {
        if let Ok(mut folder) = self.0.lock() {
            *folder = None;
        }
    }

    pub fn points_at_trigger(&self, cursor: POINT, root: HWND) -> bool {
        self.0.lock().is_ok_and(|state| {
            state
                .as_ref()
                .is_some_and(|open| open.trigger.hit(cursor, root))
        })
    }
}

#[tauri::command]
pub fn read_open_folder(folder: State<'_, OpenFolder>) -> Option<String> {
    folder.0.lock().ok()?.as_ref().map(|open| open.id.clone())
}

#[tauri::command]
pub fn close_folder(app: AppHandle, folder: State<'_, OpenFolder>) -> Result<(), String> {
    folder.clear();
    let spotify = app.state::<crate::widget_popup::WidgetPopup>();
    if spotify.from_folder() {
        spotify.close(&app);
    }
    let system = app.state::<crate::system_panel::SystemPanel>();
    if system.from_folder() {
        crate::system_panel::close_system_panel(app.clone(), system);
    }
    let _ = app.emit_to("folder", "folder-closed", ());
    window(&app, "folder")?
        .hide()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_folder(
    app: AppHandle,
    store: State<'_, DockStore>,
    folder: State<'_, OpenFolder>,
    id: String,
    anchor: PopupAnchor,
) -> Result<(), String> {
    if read_open_folder(app.state()).as_deref() == Some(&id) {
        return close_folder(app, folder);
    }
    let exists = widget_entries(&store.widgets()?.items)
        .any(|(item, _)| item.id == id && item.kind == Kind::Folder);
    if !exists {
        return Err("This folder is no longer on the dock.".into());
    }
    let dock = window(&app, "dock")?;
    let scale = dock.scale_factor().map_err(|error| error.to_string())?;
    let anchor = screen_anchor(&dock, anchor, scale)?;
    let trigger = PanelTrigger::new(handle(&dock)?, anchor);
    let settings = store.current();
    let monitor = monitor_info(center(anchor))?;
    let area = if settings.replace_taskbar {
        monitor.rcMonitor
    } else {
        monitor.rcWork
    };
    let space = popup_space(area, anchor, settings.edge, (12.0 * scale).round() as i32);
    let popup = window(&app, "folder")?;
    let bounds = popup_bounds(
        space,
        anchor,
        settings.edge,
        (440.0 * scale).round() as i32,
        (370.0 * scale).round() as i32,
    );
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
        return Err("Windows couldn’t position the folder.".into());
    }
    previews::close_all(&app);
    app.state::<crate::widget_popup::WidgetPopup>()
        .close(&app);
    crate::system_panel::close_system_panel(app.clone(), app.state());
    *folder.0.lock().map_err(|error| error.to_string())? = Some(FolderPopup {
        id: id.clone(),
        trigger,
    });
    let _ = app.emit_to("folder", "folder-opened", id);
    if let Err(error) = popup.show().and_then(|()| popup.set_focus()) {
        let _ = close_folder(app, folder);
        return Err(error.to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn launch_folder_app(
    app: AppHandle,
    store: State<'_, DockStore>,
    id: String,
    new_window: Option<bool>,
) -> Result<(), String> {
    let entry = widget_entries(&store.widgets()?.items)
        .find_map(|(item, _)| (item.kind == Kind::App && item.id == id).then(|| item.clone()))
        .ok_or("This app is no longer in the folder.")?;
    let app_id = entry.app_id.ok_or("This app could not be identified.")?;
    if app_id == apps::SETTINGS_APP_ID {
        return crate::desktop::open_settings(app, None);
    }
    let catalog = app.state::<AppCatalog>();
    if catalog.contains(&app_id) {
        return if new_window == Some(true) {
            apps::launch_app(app.clone(), catalog, app_id)
        } else {
            apps::activate_app(app.clone(), catalog, app_id, None)
        };
    }
    let path = entry
        .launch_path
        .filter(|path| {
            path.is_absolute()
                && path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        })
        .ok_or("This app is closed and its launcher is unavailable.")?;
    if apps::launch_file(&path, None) {
        Ok(())
    } else {
        Err("Windows couldn’t launch this app.".into())
    }
}
