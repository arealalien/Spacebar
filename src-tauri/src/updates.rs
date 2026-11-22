use serde::Serialize;
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

const UPDATE_EVENT: &str = "spacebar-update";
const RELEASES_URL: &str = "https://github.com/arealalien/Spacebar/releases";

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
