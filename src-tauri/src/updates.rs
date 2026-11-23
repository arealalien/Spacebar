use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

const UPDATE_EVENT: &str = "spacebar-update";
const RELEASES_URL: &str = "https://github.com/arealalien/Spacebar/releases";
const RELEASE_NOTES: &str = include_str!(concat!(env!("OUT_DIR"), "/release-notes.md"));

#[derive(Serialize)]
pub(crate) struct WhatsNew {
    version: String,
    notes: &'static str,
    url: String,
}

fn update_marker(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().app_config_dir()?.join("whats-new-version"))
}

fn matches_update(path: &Path, version: &str) -> io::Result<bool> {
    match fs::read_to_string(path) {
        Ok(marker) => Ok(marker.trim() == version),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_matching_update_marker_enables_release_notes() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("whats-new-version");
        assert!(!matches_update(&marker, "1.2.0").unwrap());
        fs::write(&marker, "1.1.0").unwrap();
        assert!(!matches_update(&marker, "1.2.0").unwrap());
        fs::write(&marker, "1.2.0\r\n").unwrap();
        assert!(matches_update(&marker, "1.2.0").unwrap());
    }
}

#[tauri::command]
pub(crate) fn read_whats_new(window: WebviewWindow) -> Option<WhatsNew> {
    if window.label() != "settings" {
        return None;
    }
    let app = window.app_handle();
    let version = app.package_info().version.to_string();
    matches_update(&update_marker(app).ok()?, &version)
        .ok()?
        .then(|| WhatsNew {
            url: format!("{RELEASES_URL}/tag/v{version}"),
            version,
            notes: RELEASE_NOTES.trim(),
        })
}

#[tauri::command]
pub(crate) fn dismiss_whats_new(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("Open Spacebar Settings to dismiss the release notes.".into());
    }
    let app = window.app_handle();
    let marker = update_marker(app).map_err(|error| error.to_string())?;
    if matches_update(&marker, &app.package_info().version.to_string())
        .map_err(|error| error.to_string())?
    {
        fs::remove_file(marker)
            .or_else(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn open_release_notes_link(window: WebviewWindow, url: Url) -> Result<(), String> {
    if window.label() != "settings"
        || !matches!(url.scheme(), "https" | "http")
        || url.host().is_none()
    {
        return Err("This release note link is unavailable.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::apps::launch_file(Path::new(url.as_str()), None)
            .then_some(())
            .ok_or_else(|| "Couldn’t open the release note link.".into())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Default)]
pub(crate) struct Updates(Mutex<Option<Pending>>);

struct Pending {
    release: Update,
    phase: Phase,
    installer: Option<Vec<u8>>,
}

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Available,
    Downloading,
    Ready,
    Installing,
}

#[derive(Clone, Serialize)]
pub(crate) struct Notice {
    version: String,
    phase: Phase,
}

impl Pending {
    fn notice(&self) -> Notice {
        Notice {
            version: self.release.version.clone(),
            phase: self.phase,
        }
    }

    fn publish(&self, app: &AppHandle) {
        let _ = app.emit_to("settings", UPDATE_EVENT, self.notice());
    }
}

async fn check(app: &AppHandle) -> tauri_plugin_updater::Result<Option<Update>> {
    let Some(mut update) = app
        .updater_builder()
        .endpoints(vec![
            format!("{RELEASES_URL}/latest/download/latest.json").parse()?
        ])?
        .configure_client(|client| client.https_only(true))
        .timeout(Duration::from_secs(12))
        .build()?
        .check()
        .await?
    else {
        return Ok(None);
    };

    // The manifest retains the asset API URL for older builds configured with a GitHub token.
    let version = &update.version;
    update.download_url =
        format!("{RELEASES_URL}/download/v{version}/Spacebar_{version}_x64-setup.exe").parse()?;
    Ok(Some(update))
}

pub(crate) fn check_on_startup(app: AppHandle) {
    if !cfg!(target_arch = "x86_64") || tauri::is_dev() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Ok(Some(mut release)) = check(&app).await {
            release.timeout = Some(Duration::from_secs(180));
            let pending = Pending {
                release,
                phase: Phase::Available,
                installer: None,
            };
            let state = app.state::<Updates>();
            if let Ok(mut slot) = state.0.lock() {
                pending.publish(&app);
                *slot = Some(pending);
            };
        }
    });
}

#[tauri::command]
pub(crate) fn read_update(state: State<'_, Updates>) -> Option<Notice> {
    state.0.lock().ok()?.as_ref().map(Pending::notice)
}

#[tauri::command]
pub(crate) async fn download_update(
    window: WebviewWindow,
    state: State<'_, Updates>,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("Open Spacebar Settings to download the update".into());
    }
    let app = window.app_handle();
    let release = {
        let mut slot = state.0.lock().map_err(|error| error.to_string())?;
        let pending = slot
            .as_mut()
            .filter(|update| update.phase == Phase::Available)
            .ok_or("No update is available to download")?;
        pending.phase = Phase::Downloading;
        pending.publish(app);
        pending.release.clone()
    };
    let result = release.download(|_, _| {}, || {}).await;
    let mut slot = state.0.lock().map_err(|error| error.to_string())?;
    let pending = slot.as_mut().ok_or("No update is available")?;
    let result = match result {
        Ok(bytes) => {
            pending.installer = Some(bytes);
            pending.phase = Phase::Ready;
            Ok(())
        }
        Err(_) => {
            pending.phase = Phase::Available;
            Err("Download failed. Try again.".into())
        }
    };
    pending.publish(app);
    result
}

#[tauri::command]
pub(crate) async fn install_update(
    window: WebviewWindow,
    state: State<'_, Updates>,
) -> Result<(), String> {
    if window.label() != "settings" {
        return Err("Open Spacebar Settings to install the update".into());
    }
    let app = window.app_handle();
    let (release, installer) = {
        let mut slot = state.0.lock().map_err(|error| error.to_string())?;
        let pending = slot
            .as_mut()
            .filter(|update| update.phase == Phase::Ready)
            .ok_or("Download the update first")?;
        let installer = pending
            .installer
            .take()
            .ok_or("Download the update first")?;
        pending.phase = Phase::Installing;
        pending.publish(app);
        (pending.release.clone(), installer)
    };

    // The taskbar guardian restores the Windows taskbar when Spacebar exits.
    if release.install(&installer).is_err() {
        let mut slot = state.0.lock().map_err(|error| error.to_string())?;
        let pending = slot.as_mut().ok_or("No update is available")?;
        pending.installer = Some(installer);
        pending.phase = Phase::Ready;
        pending.publish(app);
        return Err("Couldn’t start the installer. Try again.".into());
    }
    Ok(())
}
