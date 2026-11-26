use crate::widget_popup::WidgetPopup;
use crate::{apps::launch_file, settings::save_json, widgets::valid_color};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::{self, Write},
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewWindow};
use windows::{
    core::{w, HRESULT, HSTRING},
    Win32::{
        Foundation::HWND,
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FileSaveDialog, IFileSaveDialog, FOS_FORCEFILESYSTEM,
            FOS_OVERWRITEPROMPT, SIGDN_FILESYSPATH,
        },
    },
};
use windows_sys::Win32::Foundation::ERROR_CANCELLED;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentKind {
    QuickNote,
    Agenda,
    QuickTasks,
    ColorPicker,
    Weather,
}

impl ContentKind {
    fn file(self) -> &'static str {
        match self {
            Self::QuickNote => "quick-note.json",
            Self::Agenda => "agenda.json",
            Self::QuickTasks => "quick-tasks.json",
            Self::ColorPicker => "color-picker.json",
            Self::Weather => "weather.json",
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct Drawing {
    color: String,
    image: Option<String>,
}

impl Default for Drawing {
    fn default() -> Self {
        Self {
            color: "#303246".into(),
            image: None,
        }
    }
}

const PNG_PREFIX: &str = "data:image/png;base64,";
const MAX_DRAWING_BYTES: usize = 4 * 1024 * 1024;

impl Drawing {
    fn valid(&self) -> bool {
        valid_color(&self.color)
            && self.image.as_ref().is_none_or(|image| {
                image.len() <= MAX_DRAWING_BYTES
                    && image.starts_with("data:image/png;base64,iVBORw0KGgo")
            })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgendaEvent {
    id: String,
    title: String,
    starts_at: i64,
    link: Option<String>,
}

impl AgendaEvent {
    fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 80
            && !self.title.trim().is_empty()
            && self.title.chars().count() <= 200
            && (0..=253_402_300_799_999).contains(&self.starts_at)
            && self
                .link
                .as_ref()
                .is_none_or(|link| link.len() <= 2048 && web_link(link).is_some())
    }
}

fn web_link(link: &str) -> Option<Url> {
    let url = Url::parse(link).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host().is_some()).then_some(url)
}

#[derive(Deserialize, Serialize)]
pub struct QuickTask {
    id: String,
    title: String,
    done: bool,
}

#[derive(Deserialize, Serialize)]
struct WeatherLocation {
    name: String,
    detail: String,
    latitude: f64,
    longitude: f64,
}

#[derive(Default, Deserialize, Serialize)]
pub struct WeatherPreferences {
    location: Option<WeatherLocation>,
    fahrenheit: bool,
}

impl WeatherPreferences {
    fn valid(&self) -> bool {
        self.location.as_ref().is_none_or(|city| {
            !city.name.trim().is_empty()
                && city.name.chars().count() <= 160
                && city.detail.chars().count() <= 200
                && (-90.0..=90.0).contains(&city.latitude)
                && (-180.0..=180.0).contains(&city.longitude)
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "kebab-case")]
pub enum WidgetContent {
    QuickNote(String),
    Agenda(Vec<AgendaEvent>),
    QuickTasks(Vec<QuickTask>),
    ColorPicker(Vec<String>),
    Weather(WeatherPreferences),
}

pub struct WidgetContentStore {
    directory: PathBuf,
    access: Mutex<()>,
}

impl WidgetContentStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory: directory.join("widget-content"),
            access: Mutex::new(()),
        }
    }

    fn drawing_path(&self, id: u32) -> PathBuf {
        self.directory.join("drawings").join(format!("{id}.json"))
    }

    fn drawing_sequence_path(&self) -> PathBuf {
        self.directory.join("drawings").join("last-id.json")
    }

    fn drawing_sequence(&self) -> Result<Option<u32>, String> {
        match fs::read(self.drawing_sequence_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    // Called with access held so importing the old pad and allocating IDs are atomic.
    fn drawing_ids(&self) -> Result<Vec<u32>, String> {
        let directory = self.directory.join("drawings");
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let mut ids = Vec::new();
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(id) = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .and_then(|stem| stem.parse().ok())
                {
                    ids.push(id);
                }
            }
        }
        ids.sort_unstable();
        if ids.is_empty() && self.drawing_sequence()?.is_none() {
            let legacy = match fs::read(self.directory.join("drawing-pad.json")) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Couldn't import the previous drawing. {error}"))?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Drawing::default(),
                Err(error) => return Err(format!("Couldn't read the previous drawing. {error}")),
            };
            save_json(&self.drawing_path(1), &legacy)?;
            ids.push(1);
        }
        Ok(ids)
    }

    fn load_drawing(&self, id: u32) -> Result<Drawing, String> {
        let bytes = fs::read(self.drawing_path(id))
            .map_err(|error| format!("Couldn't open Drawing {id}. {error}"))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("Couldn't read Drawing {id}. {error}"))
    }

    fn create_drawing(&self) -> Result<u32, String> {
        let id = self
            .drawing_ids()?
            .last()
            .copied()
            .unwrap_or_default()
            .max(self.drawing_sequence()?.unwrap_or_default())
            .checked_add(1)
            .ok_or("No more drawing IDs are available.")?;
        save_json(&self.drawing_sequence_path(), &id)?;
        save_json(&self.drawing_path(id), &Drawing::default())?;
        Ok(id)
    }

    fn save_drawing(&self, id: u32, drawing: &Drawing) -> Result<(), String> {
        let path = self.drawing_path(id);
        if !path.is_file() || !drawing.valid() {
            return Err(format!("Couldn't save Drawing {id}."));
        }
        save_json(&path, drawing)
    }

    fn delete_drawing(&self, id: u32) -> Result<(), String> {
        let path = self.drawing_path(id);
        if !path.is_file() {
            return Err(format!("Drawing {id} no longer exists."));
        }
        // Older libraries may have no sequence file yet. Keep their highest ID
        // before removing anything, including the last imported drawing.
        let sequence = self.drawing_sequence()?.unwrap_or_default();
        let latest = self.drawing_ids()?.last().copied().unwrap_or_default();
        if latest > sequence {
            save_json(&self.drawing_sequence_path(), &latest)?;
        }
        fs::remove_file(path).map_err(|error| format!("Couldn't delete Drawing {id}. {error}"))
    }

    fn read<T: DeserializeOwned + Default>(&self, kind: ContentKind) -> Result<T, String> {
        let _access = self.access.lock().map_err(|error| error.to_string())?;
        match fs::read(self.directory.join(kind.file())) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("Couldn’t read the saved widget content. {error}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
            Err(error) => Err(format!("Couldn’t read the saved widget content. {error}")),
        }
    }

    fn write(&self, content: &WidgetContent) -> Result<(), String> {
        use ContentKind::*;
        let _access = self.access.lock().map_err(|error| error.to_string())?;
        let path = |kind: ContentKind| self.directory.join(kind.file());
        match content {
            WidgetContent::QuickNote(note) if note.len() <= 128 * 1024 => {
                save_json(&path(QuickNote), note)
            }
            WidgetContent::QuickNote(_) => Err("This note is too large to save.".into()),
            WidgetContent::Agenda(events) => {
                let mut ids = HashSet::new();
                if events.len() > 256 {
                    return Err("Agenda can hold up to 256 events.".into());
                }
                if !events
                    .iter()
                    .all(|entry| entry.valid() && ids.insert(&entry.id))
                {
                    return Err("Check the event’s title, time, and web link.".into());
                }
                save_json(&path(Agenda), events)
            }
            WidgetContent::QuickTasks(tasks) => {
                if tasks.len() > 128 {
                    return Err("Quick Tasks can hold up to 128 tasks.".into());
                }
                let mut ids = HashSet::new();
                if !tasks.iter().all(|task| {
                    !task.id.is_empty()
                        && task.id.len() <= 80
                        && ids.insert(&task.id)
                        && !task.title.trim().is_empty()
                        && task.title.chars().count() <= 160
                }) {
                    return Err("Check the task’s title.".into());
                }
                save_json(&path(QuickTasks), tasks)
            }
            WidgetContent::ColorPicker(colors)
                if colors.len() <= 32 && colors.iter().all(|color| valid_color(color)) =>
            {
                save_json(&path(ColorPicker), colors)
            }
            WidgetContent::ColorPicker(_) => Err("Couldn’t save the color history.".into()),
            WidgetContent::Weather(preferences) if preferences.valid() => {
                save_json(&path(Weather), preferences)
            }
            WidgetContent::Weather(_) => Err("Choose a valid weather location.".into()),
        }
    }
}

fn require_panel(source: &WebviewWindow) -> Result<(), String> {
    if source.label() != "system" {
        return Err("Open the widget’s panel first.".into());
    }
    Ok(())
}

fn require_weather_popup(source: &WebviewWindow) -> Result<(), String> {
    if source.label() == "widget-popup" && source.state::<WidgetPopup>().weather_open() {
        Ok(())
    } else {
        Err("Open the Weather widget first.".into())
    }
}

async fn drawing_worker<T: Send + 'static>(
    app: AppHandle,
    source: WebviewWindow,
    work: impl FnOnce(&WidgetContentStore) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    require_panel(&source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<WidgetContentStore>();
        let _access = store.access.lock().map_err(|error| error.to_string())?;
        work(&store)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn list_drawings(app: AppHandle, source: WebviewWindow) -> Result<Vec<u32>, String> {
    drawing_worker(app, source, WidgetContentStore::drawing_ids).await
}

#[tauri::command]
pub async fn create_drawing(app: AppHandle, source: WebviewWindow) -> Result<u32, String> {
    drawing_worker(app, source, WidgetContentStore::create_drawing).await
}

#[tauri::command]
pub async fn read_drawing(
    app: AppHandle,
    source: WebviewWindow,
    id: u32,
) -> Result<Drawing, String> {
    drawing_worker(app, source, move |store| store.load_drawing(id)).await
}

#[tauri::command]
pub async fn save_drawing(
    app: AppHandle,
    source: WebviewWindow,
    id: u32,
    drawing: Drawing,
) -> Result<(), String> {
    drawing_worker(app, source, move |store| store.save_drawing(id, &drawing)).await
}

#[tauri::command]
pub async fn delete_drawing(app: AppHandle, source: WebviewWindow, id: u32) -> Result<(), String> {
    drawing_worker(app, source, move |store| store.delete_drawing(id)).await
}

fn choose_drawing_export(owner: usize, id: u32) -> windows::core::Result<Option<PathBuf>> {
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetTitle(w!("Save drawing as PNG"))?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: w!("PNG image (*.png)"),
            pszSpec: w!("*.png"),
        }])?;
        dialog.SetDefaultExtension(w!("png"))?;
        dialog.SetFileName(&HSTRING::from(format!("Drawing {id}.png")))?;
        match dialog.Show(Some(HWND(owner as *mut _))) {
            Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED) => return Ok(None),
            result => result?,
        }
        let display_name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let name = OsString::from_wide(display_name.as_wide());
        CoTaskMemFree(Some(display_name.0.cast()));
        Ok(Some(PathBuf::from(name)))
    }
}

#[tauri::command]
pub async fn export_drawing(source: WebviewWindow, id: u32, image: String) -> Result<bool, String> {
    require_panel(&source)?;
    let owner = crate::desktop::handle(&source)? as usize;
    crate::system_widgets::shell_worker(move || {
        let payload = image
            .strip_prefix(PNG_PREFIX)
            .filter(|value| value.len() <= MAX_DRAWING_BYTES)
            .ok_or("Couldn't export the drawing as PNG.")?;
        let png = STANDARD
            .decode(payload)
            .ok()
            .filter(|bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
            .ok_or("Couldn't export the drawing as PNG.")?;
        let Some(path) = choose_drawing_export(owner, id).map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        save_export(&path, &png).map_err(|error| format!("Couldn't save the PNG. {error}"))?;
        Ok(true)
    })
    .await
}

fn save_export(path: &Path, png: &[u8]) -> io::Result<()> {
    let directory = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(png)?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod drawing_tests {
    use super::*;

    #[test]
    fn deleting_the_only_imported_drawing_keeps_the_library_empty() {
        let directory = tempfile::tempdir().unwrap();
        let store = WidgetContentStore::new(directory.path().to_owned());
        save_json(&store.directory.join("drawing-pad.json"), &Drawing::default()).unwrap();
        assert_eq!(store.drawing_ids().unwrap(), [1]);
        store.delete_drawing(1).unwrap();
        assert!(store.save_drawing(1, &Drawing::default()).is_err());

        let reopened = WidgetContentStore::new(directory.path().to_owned());
        assert!(reopened.drawing_ids().unwrap().is_empty());
        assert_eq!(reopened.create_drawing().unwrap(), 2);
        assert_eq!(reopened.create_drawing().unwrap(), 3);
        reopened.delete_drawing(3).unwrap();
        assert_eq!(reopened.drawing_ids().unwrap(), [2]);
        assert_eq!(reopened.create_drawing().unwrap(), 4);
    }

    #[test]
    fn writes_an_export_to_a_new_or_existing_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Drawing 1.png");
        save_export(&path, b"initial contents").unwrap();
        save_export(&path, b"updated PNG bytes").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"updated PNG bytes");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn imports_the_old_pad_once_and_keeps_drawings_independent() {
        let directory = tempfile::tempdir().unwrap();
        let store = WidgetContentStore::new(directory.path().to_owned());
        let legacy = Drawing {
            color: "#123456".into(),
            image: Some("data:image/png;base64,iVBORw0KGgo=".into()),
        };
        save_json(&store.directory.join("drawing-pad.json"), &legacy).unwrap();
        assert_eq!(store.drawing_ids().unwrap(), [1]);
        assert_eq!(store.load_drawing(1).unwrap().image, legacy.image);
        assert_eq!(store.create_drawing().unwrap(), 2);
        assert!(store.load_drawing(2).unwrap().image.is_none());
        assert_eq!(store.load_drawing(1).unwrap().color, legacy.color);
        store.save_drawing(2, &legacy).unwrap();
        store.save_drawing(1, &Drawing::default()).unwrap();

        let reopened = WidgetContentStore::new(directory.path().to_owned());
        assert_eq!(reopened.drawing_ids().unwrap(), [1, 2]);
        assert!(reopened.load_drawing(1).unwrap().image.is_none());
        assert_eq!(reopened.load_drawing(2).unwrap().image, legacy.image);
    }
}

#[tauri::command]
pub async fn read_widget_content(
    app: AppHandle,
    source: WebviewWindow,
    kind: ContentKind,
) -> Result<WidgetContent, String> {
    if matches!(kind, ContentKind::Weather) {
        require_weather_popup(&source)?;
    } else {
        require_panel(&source)?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<WidgetContentStore>();
        match kind {
            ContentKind::QuickNote => store.read(kind).map(WidgetContent::QuickNote),
            ContentKind::Agenda => store.read(kind).map(WidgetContent::Agenda),
            ContentKind::QuickTasks => store.read(kind).map(WidgetContent::QuickTasks),
            ContentKind::ColorPicker => store.read(kind).map(WidgetContent::ColorPicker),
            ContentKind::Weather => store.read(kind).map(WidgetContent::Weather),
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn write_widget_content(
    app: AppHandle,
    source: WebviewWindow,
    content: WidgetContent,
) -> Result<(), String> {
    if matches!(&content, WidgetContent::Weather(_)) {
        require_weather_popup(&source)?;
    } else {
        require_panel(&source)?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<WidgetContentStore>().write(&content)?;
        match content {
            WidgetContent::Agenda(events) => {
                let _ = app.emit_to("dock", "agenda-changed", &events);
                let _ = app.emit_to("folder", "agenda-changed", &events);
            }
            WidgetContent::QuickTasks(tasks) => {
                let _ = app.emit_to("dock", "quick-tasks-changed", &tasks);
                let _ = app.emit_to("folder", "quick-tasks-changed", &tasks);
            }
            WidgetContent::Weather(preferences) => {
                let _ = app.emit("weather-preferences-changed", &preferences);
            }
            _ => {}
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn read_agenda(
    app: AppHandle,
    source: WebviewWindow,
) -> Result<Vec<AgendaEvent>, String> {
    if !matches!(source.label(), "dock" | "folder") {
        return Err("The agenda widget is unavailable here.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<WidgetContentStore>().read(ContentKind::Agenda)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn read_quick_tasks(
    app: AppHandle,
    source: WebviewWindow,
) -> Result<Vec<QuickTask>, String> {
    if !matches!(source.label(), "dock" | "folder") {
        return Err("Quick Tasks is unavailable here.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<WidgetContentStore>()
            .read(ContentKind::QuickTasks)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn read_weather_preferences(
    app: AppHandle,
    source: WebviewWindow,
) -> Result<WeatherPreferences, String> {
    if !matches!(source.label(), "dock" | "folder" | "widget-popup") {
        return Err("Weather is unavailable here.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<WidgetContentStore>().read(ContentKind::Weather)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_weather_source(source: WebviewWindow) -> Result<(), String> {
    require_weather_popup(&source)?;
    tauri::async_runtime::spawn_blocking(|| {
        launch_file(Path::new("https://open-meteo.com/"), None)
            .then_some(())
            .ok_or_else(|| "Couldn’t open the weather data source.".into())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_agenda_link(
    app: AppHandle,
    source: WebviewWindow,
    id: String,
) -> Result<(), String> {
    require_panel(&source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let entries: Vec<AgendaEvent> = app
            .state::<WidgetContentStore>()
            .read(ContentKind::Agenda)?;
        let link = entries
            .into_iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.link)
            .and_then(|link| web_link(&link))
            .ok_or("This event doesn’t have a web link.")?;
        launch_file(Path::new(link.as_str()), None)
            .then_some(())
            .ok_or_else(|| "Couldn’t open the event link.".into())
    })
    .await
    .map_err(|error| error.to_string())?
}
