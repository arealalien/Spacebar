use crate::{
    settings::{save_json, AppAppearance, DockEdge, Material, Settings},
    start_launcher::StartPreferences,
    start_menu, system_panel,
    widgets::{migrate_widget_items, valid_layout, Item, Layout},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct DockData {
    settings: Settings,
    app_appearance: Option<AppAppearance>,
    widgets: Layout,
    app_order: Vec<String>,
    saved_docks: Vec<SavedDock>,
    active_dock: Option<String>,
    start_menu: StartPreferences,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedDock {
    id: String,
    name: String,
    settings: Settings,
    items: Vec<Item>,
    #[serde(default)]
    app_order: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedDockSummary {
    id: String,
    name: String,
    edge: DockEdge,
    material: Material,
    widget_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedDocks {
    active_id: Option<String>,
    docks: Vec<SavedDockSummary>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum SavedDockEdit {
    Save { id: String, name: String },
    Load { id: String },
    Update { id: String },
    Rename { id: String, name: String },
    Delete { id: String },
}

#[derive(Serialize)]
pub(crate) struct SavedDockResult {
    list: SavedDocks,
    settings: Option<Settings>,
}

impl DockData {
    fn load(path: &Path, directory: &Path) -> Self {
        if let Some(mut data) = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
        {
            data.settings = data.settings.normalized();
            migrate_widget_items(&mut data.widgets.items);
            if !valid_layout(&data.widgets.items) {
                data.widgets = Layout::default();
            }
            data.saved_docks.retain_mut(|dock| {
                migrate_widget_items(&mut dock.items);
                valid_layout(&dock.items)
            });
            return data;
        }
        let settings = fs::read(directory.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .unwrap_or_default();
        Self {
            settings: settings.normalized(),
            widgets: Layout::load_legacy(directory),
            ..Self::default()
        }
    }

    fn summaries(&self) -> SavedDocks {
        SavedDocks {
            active_id: self.active_dock.clone(),
            docks: self
                .saved_docks
                .iter()
                .map(|dock| SavedDockSummary {
                    id: dock.id.clone(),
                    name: dock.name.clone(),
                    edge: dock.settings.edge,
                    material: dock.settings.material,
                    widget_count: dock.items.len(),
                })
                .collect(),
        }
    }

    fn snapshot(&self, id: String, name: String) -> SavedDock {
        SavedDock {
            id,
            name,
            settings: self.settings,
            items: self.widgets.items.clone(),
            app_order: self.app_order.clone(),
        }
    }

    fn saved_index(&self, id: &str) -> Result<usize, String> {
        self.saved_docks
            .iter()
            .position(|dock| dock.id == id)
            .ok_or_else(|| "This saved dock is no longer available.".into())
    }

    fn edit_saved(&mut self, edit: SavedDockEdit) -> Result<bool, String> {
        match edit {
            SavedDockEdit::Save { id, name } => {
                if id.is_empty() || id.len() > 80 || self.saved_index(&id).is_ok() {
                    return Err("Couldn’t create a unique ID for the saved dock.".into());
                }
                let dock = self.snapshot(id.clone(), dock_name(&name)?);
                self.saved_docks.insert(0, dock);
                self.active_dock = Some(id);
            }
            SavedDockEdit::Load { id } => {
                let dock = &self.saved_docks[self.saved_index(&id)?];
                self.settings = dock.settings.normalized();
                self.widgets = Layout {
                    revision: self.widgets.revision + 1,
                    items: dock.items.clone(),
                };
                self.app_order = dock.app_order.clone();
                self.active_dock = Some(id);
                return Ok(true);
            }
            SavedDockEdit::Update { id } => {
                let index = self.saved_index(&id)?;
                if self.active_dock.as_deref() != Some(&id) {
                    return Err("Load this dock before saving changes to it.".into());
                }
                self.saved_docks[index] = self.snapshot(id, self.saved_docks[index].name.clone());
            }
            SavedDockEdit::Rename { id, name } => {
                let index = self.saved_index(&id)?;
                self.saved_docks[index].name = dock_name(&name)?;
            }
            SavedDockEdit::Delete { id } => {
                let index = self.saved_index(&id)?;
                self.saved_docks.remove(index);
                if self.active_dock.as_deref() == Some(&id) {
                    self.active_dock = None;
                }
            }
        }
        Ok(false)
    }
}

fn dock_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Enter a dock name between 1 and 64 characters.".into());
    }
    Ok(name.to_owned())
}

pub(crate) struct DockStore {
    path: PathBuf,
    data: Mutex<DockData>,
}

impl DockStore {
    pub fn load(app: &AppHandle) -> tauri::Result<Self> {
        let directory = app.path().app_config_dir()?;
        let path = directory.join("dock.json");
        let mut data = DockData::load(&path, &directory);
        data.app_appearance
            .get_or_insert_with(|| data.settings.legacy_app_appearance());
        start_menu::set_spacebar_start(
            data.widgets.uses_spacebar_start(),
            data.settings.replace_windows_start,
        );
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, DockData>, String> {
        self.data.lock().map_err(|error| error.to_string())
    }

    fn commit(&self, current: &mut DockData, next: DockData) -> Result<(), String> {
        save_json(&self.path, &next)?;
        *current = next;
        Ok(())
    }

    pub fn current(&self) -> Settings {
        self.data
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .settings
    }

    pub fn widgets(&self) -> Result<Layout, String> {
        Ok(self.lock()?.widgets.clone())
    }

    pub(crate) fn start_preferences(&self) -> Result<StartPreferences, String> {
        Ok(self.lock()?.start_menu)
    }

    pub(crate) fn save_start_preferences(
        &self,
        preferences: StartPreferences,
    ) -> Result<StartPreferences, String> {
        let mut current = self.lock()?;
        let mut next = current.clone();
        next.start_menu = preferences;
        self.commit(&mut current, next)?;
        Ok(current.start_menu)
    }

    pub fn app_order(&self) -> Vec<String> {
        self.data
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .app_order
            .clone()
    }

    pub fn move_app(
        &self,
        visible: &[String],
        app: &str,
        anchor: &str,
        before: bool,
    ) -> Result<(), String> {
        let mut current = self.lock()?;
        let order = moved_app_order(&current.app_order, visible, app, anchor, before);
        if order != current.app_order {
            let mut next = current.clone();
            next.app_order = order;
            self.commit(&mut current, next)?;
        }
        Ok(())
    }
}

fn update_start_mode(app: &AppHandle, data: &DockData) {
    let installed = data.widgets.uses_spacebar_start();
    start_menu::set_spacebar_start(installed, data.settings.replace_windows_start);
    if !installed {
        system_panel::close_spacebar_start(app);
    }
}

fn moved_app_order(
    saved: &[String],
    visible: &[String],
    app: &str,
    anchor: &str,
    before: bool,
) -> Vec<String> {
    let from = visible.iter().position(|id| id == app);
    let to = visible.iter().position(|id| id == anchor);
    let (Some(from), Some(to)) = (from, to) else {
        return saved.to_vec();
    };
    if from == to || (before && from + 1 == to) || (!before && to + 1 == from) {
        return saved.to_vec();
    }
    // Retain slots for apps which are temporarily closed or inside folders.
    let mut seen = HashSet::new();
    let mut next: Vec<String> = saved
        .iter()
        .chain(visible)
        .filter(|id| id.as_str() != app && seen.insert(id.as_str()))
        .cloned()
        .collect();
    if let Some(index) = next.iter().position(|id| id == anchor) {
        next.insert(index + usize::from(!before), app.to_owned());
    }
    next
}

#[tauri::command]
pub fn read_settings(store: State<'_, DockStore>) -> Settings {
    store.current()
}

#[tauri::command]
pub(crate) fn read_app_appearance(store: State<'_, DockStore>) -> Result<AppAppearance, String> {
    Ok(store.lock()?.app_appearance.unwrap_or_default())
}

#[tauri::command]
pub(crate) fn write_app_appearance(
    store: State<'_, DockStore>,
    appearance: AppAppearance,
) -> Result<(), String> {
    let mut current = store.lock()?;
    let mut next = current.clone();
    next.app_appearance = Some(appearance);
    store.commit(&mut current, next)
}

#[tauri::command]
pub fn write_settings(
    app: AppHandle,
    store: State<'_, DockStore>,
    settings: Settings,
) -> Result<(), String> {
    let mut current = store.lock()?;
    let mut next = current.clone();
    next.settings = settings.normalized();
    store.commit(&mut current, next)?;
    update_start_mode(&app, &current);
    let _ = app.emit("settings-changed", current.settings);
    Ok(())
}

#[tauri::command]
pub fn read_widgets(store: State<'_, DockStore>) -> Result<Layout, String> {
    store.widgets()
}

#[tauri::command]
pub fn save_widgets(
    app: AppHandle,
    store: State<'_, DockStore>,
    revision: u64,
    mut items: Vec<Item>,
) -> Result<bool, String> {
    migrate_widget_items(&mut items);
    if !valid_layout(&items) {
        return Err("This widget layout is invalid or too large.".into());
    }
    let mut current = store.lock()?;
    if current.widgets.revision != revision {
        return Ok(false);
    }
    let mut next = current.clone();
    next.widgets = Layout {
        revision: revision + 1,
        items,
    };
    store.commit(&mut current, next)?;
    update_start_mode(&app, &current);
    let _ = app.emit("widgets-changed", &current.widgets);
    Ok(true)
}

#[tauri::command]
pub fn read_saved_docks(store: State<'_, DockStore>) -> Result<SavedDocks, String> {
    Ok(store.lock()?.summaries())
}

#[tauri::command]
pub fn edit_saved_docks(
    app: AppHandle,
    store: State<'_, DockStore>,
    edit: SavedDockEdit,
) -> Result<SavedDockResult, String> {
    let mut current = store.lock()?;
    let mut next = current.clone();
    let loaded = next.edit_saved(edit)?;
    store.commit(&mut current, next)?;
    if loaded {
        update_start_mode(&app, &current);
        let _ = app.emit("settings-changed", current.settings);
        let _ = app.emit("widgets-changed", &current.widgets);
        let _ = app.emit("dock-apps-changed", ());
    }
    Ok(SavedDockResult {
        list: current.summaries(),
        settings: loaded.then_some(current.settings),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_app_appearance_and_keeps_it_when_loading_a_dock() {
        let mut data: DockData =
            serde_json::from_str(r#"{"settings":{"accent":"light"}}"#).unwrap();
        let appearance = data.settings.legacy_app_appearance();
        assert_eq!(
            serde_json::to_value(appearance).unwrap(),
            serde_json::json!({"theme": "light", "tintColor": "#e8e9ed", "blurAmount": 100, "tintAmount": 100})
        );
        data.app_appearance = Some(appearance);
        data.edit_saved(SavedDockEdit::Save {
            id: "work".into(),
            name: "Work".into(),
        })
        .unwrap();

        data.app_appearance = serde_json::from_value(
            serde_json::json!({"theme": "dark", "tintColor": "#52A1CF", "blurAmount": 73, "tintAmount": 81}),
        )
        .unwrap();
        let mut reloaded: DockData =
            serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
        reloaded
            .edit_saved(SavedDockEdit::Load { id: "work".into() })
            .unwrap();
        assert_eq!(
            serde_json::to_value(reloaded.app_appearance).unwrap(),
            serde_json::json!({"theme": "dark", "tintColor": "#52a1cf", "blurAmount": 73, "tintAmount": 81})
        );
    }

    #[test]
    fn start_preferences_preserve_app_layout_from_older_docks() {
        let mut data: DockData = serde_json::from_str(r#"{"settings":{"offset":23}}"#).unwrap();
        assert_eq!(
            serde_json::to_value(&data.start_menu).unwrap(),
            serde_json::json!({"appLayout":"list"})
        );

        data.start_menu = serde_json::from_value::<StartPreferences>(serde_json::json!({
            "appLayout": "grid", "widgets": ["volume", "calendar", "clock"]
        }))
        .unwrap();
        let reloaded: DockData =
            serde_json::from_slice(&serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(reloaded.settings.offset, 23);
        assert_eq!(
            serde_json::to_value(&reloaded.start_menu).unwrap(),
            serde_json::json!({"appLayout":"grid"})
        );
    }

    #[test]
    fn snapshots_include_widgets_and_app_order_and_can_be_updated() {
        let mut data = DockData {
            app_order: vec!["browser".into(), "editor".into()],
            ..DockData::default()
        };
        data.settings.edge = DockEdge::Left;
        data.settings.padding = 18;
        data.widgets.items.push(
            serde_json::from_value(serde_json::json!({
                "id": "clock", "kind": "clock", "size": "small", "color": "#55aaff"
            }))
            .unwrap(),
        );
        let id = String::from("work");
        data.edit_saved(SavedDockEdit::Save {
            id: id.clone(),
            name: "Work".into(),
        })
        .unwrap();
        let saved = serde_json::to_vec(&data).unwrap();
        let mut data: DockData = serde_json::from_slice(&saved).unwrap();
        assert_eq!(data.active_dock.as_deref(), Some("work"));
        data.settings.edge = DockEdge::Top;
        data.settings.padding = 4;
        data.widgets.items.pop();
        data.widgets.revision = 9;
        data.app_order.reverse();
        assert!(data
            .edit_saved(SavedDockEdit::Load { id: id.clone() })
            .unwrap());
        assert!(data.settings.edge == DockEdge::Left);
        assert_eq!(data.settings.padding, 18);
        assert_eq!(data.widgets.revision, 10);
        assert_eq!(
            serde_json::to_value(&data.widgets.items).unwrap()[1]["color"],
            "#55aaff"
        );
        assert_eq!(data.app_order, ["browser", "editor"]);
        data.settings.edge = DockEdge::Right;
        data.edit_saved(SavedDockEdit::Update { id: id.clone() })
            .unwrap();
        assert!(data.saved_docks[0].settings.edge == DockEdge::Right);
    }

    #[test]
    fn moving_apps_preserves_positions_for_closed_apps() {
        let saved = ["browser", "closed", "editor", "music"].map(String::from);
        let visible = ["browser", "editor", "music", "terminal"].map(String::from);
        let next = moved_app_order(&saved, &visible, "music", "browser", true);
        assert_eq!(next, ["music", "browser", "closed", "editor", "terminal"]);
        let visible = ["music", "browser", "editor", "terminal"].map(String::from);
        let next = moved_app_order(&next, &visible, "browser", "terminal", false);
        assert_eq!(next, ["music", "closed", "editor", "terminal", "browser"]);
        assert!(moved_app_order(&[], &visible, "music", "browser", true).is_empty());
    }
}
