use crate::widget_popup::WidgetPopup;
use crate::{apps::launch_file, settings::save_json, widgets::valid_color};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewWindow};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentKind {
    QuickNote,
    DrawingPad,
    Agenda,
    QuickTasks,
    ColorPicker,
    Weather,
}

impl ContentKind {
    fn file(self) -> &'static str {
        match self {
            Self::QuickNote => "quick-note.json",
            Self::DrawingPad => "drawing-pad.json",
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
    DrawingPad(Drawing),
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
            WidgetContent::DrawingPad(drawing)
                if valid_color(&drawing.color)
                    && drawing.image.as_ref().is_none_or(|image| {
                        image.len() <= 4 * 1024 * 1024
                            && image.starts_with("data:image/png;base64,iVBORw0KGgo")
                    }) =>
            {
                save_json(&path(DrawingPad), drawing)
            }
            WidgetContent::DrawingPad(_) => Err("Couldn’t save the drawing.".into()),
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
            ContentKind::DrawingPad => store.read(kind).map(WidgetContent::DrawingPad),
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
