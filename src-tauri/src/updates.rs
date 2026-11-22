use reqwest::{
    header::{HeaderValue, ACCEPT, AUTHORIZATION},
    Client, StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use std::{ptr, slice, sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};
use windows_core::PCWSTR;
use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
};

const UPDATE_EVENT: &str = "spacebar-update";
const CREDENTIAL_NAME: &str = "Spacebar GitHub Updates";
const GITHUB_API_VERSION: &str = "2022-11-28";

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

struct Credential(*mut CREDENTIALW);

impl Drop for Credential {
    fn drop(&mut self) {
        unsafe { CredFree(self.0.cast()) };
    }
}

fn github_credential() -> Option<(String, HeaderValue)> {
    let target: Vec<u16> = CREDENTIAL_NAME.encode_utf16().chain([0]).collect();
    let mut raw = ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 || raw.is_null() {
        return None;
    }
    let credential = Credential(raw);
    // CredReadW owns the username and password buffers until CredFree.
    let details = unsafe { &*credential.0 };
    if details.UserName.is_null()
        || details.CredentialBlob.is_null()
        || details.CredentialBlobSize == 0
    {
        return None;
    }
    let repository = unsafe { PCWSTR(details.UserName).to_string() }.ok()?;
    let (owner, name) = repository.split_once('/')?;
    if ![owner, name].into_iter().all(|part| {
        !matches!(part, "" | "." | "..")
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    }) {
        return None;
    }

    let blob = unsafe {
        slice::from_raw_parts(details.CredentialBlob, details.CredentialBlobSize as usize)
    };
    if blob.len() % 2 != 0 {
        return None;
    }
    let units: Vec<_> = blob
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let password = String::from_utf16(&units).ok()?;
    let token = password.trim_end_matches('\0').trim();
    if token.is_empty() {
        return None;
    }
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    authorization.set_sensitive(true);
    Some((repository, authorization))
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
}

async fn check(
    app: &AppHandle,
) -> Result<Option<Update>, Box<dyn std::error::Error + Send + Sync>> {
    let Some((repository, authorization)) = github_credential() else {
        return Ok(None);
    };
    let base = format!("https://api.github.com/repos/{repository}/releases");
    let response = Client::builder()
        .https_only(true)
        .user_agent("Spacebar Updater")
        .timeout(Duration::from_secs(10))
        .build()?
        .get(format!("{base}/latest"))
        .header(AUTHORIZATION, authorization.clone())
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .send()
        .await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let release = response.error_for_status()?.json::<Release>().await?;
    let Some(version) = release.tag_name.strip_prefix('v') else {
        return Ok(None);
    };
    if version == app.package_info().version.to_string() {
        return Ok(None);
    }
    let installer = format!("Spacebar_{version}_x64-setup.exe");
    let (Some(manifest), Some(installer)) = (
        release
            .assets
            .iter()
            .find(|asset| asset.name == "latest.json"),
        release.assets.iter().find(|asset| asset.name == installer),
    ) else {
        return Ok(None);
    };
    let asset_url = |id| Url::parse(&format!("{base}/assets/{id}"));
    let installer_url = asset_url(installer.id)?;
    let update = app
        .updater_builder()
        .endpoints(vec![asset_url(manifest.id)?])?
        .header(AUTHORIZATION, authorization)?
        .header(ACCEPT, "application/octet-stream")?
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)?
        .configure_client(|client| client.https_only(true))
        .timeout(Duration::from_secs(12))
        .build()?
        .check()
        .await?;

    // Only the listed installer for this release may receive the GitHub token.
    Ok(update.filter(|update| {
        update.version == version
            && update
                .download_url
                .as_str()
                .eq_ignore_ascii_case(installer_url.as_str())
    }))
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
