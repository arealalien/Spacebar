use super::{enumerator, AudioLevel, Com};
use crate::apps::process_path;
use std::{
    ptr::null,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{State, WebviewWindow};
use windows::{
    core::{Interface, Result as WindowsResult},
    Win32::{
        Media::Audio::{
            eRender, AudioSessionStateActive, AudioSessionStateExpired,
            Endpoints::IAudioMeterInformation, IAudioSessionControl, IAudioSessionControl2,
            IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, DEVICE_STATE_ACTIVE,
        },
        System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED},
    },
};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(32);
const SESSION_REFRESH: Duration = Duration::from_millis(900);
const IDLE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct SpotifyMeter(Arc<Mutex<MeterState>>);

#[derive(Default)]
struct MeterState {
    last_request: Option<Instant>,
    peak: Option<f32>,
    running: bool,
}

#[tauri::command]
pub fn spotify_output_peak(
    webview: WebviewWindow,
    meter: State<'_, SpotifyMeter>,
) -> Result<Option<f32>, String> {
    if !matches!(webview.label(), "widget-popup" | "system") {
        return Ok(None);
    }
    let mut state = meter.0.lock().map_err(|error| error.to_string())?;
    state.last_request = Some(Instant::now());
    if !state.running {
        let shared = Arc::clone(&meter.0);
        thread::Builder::new()
            .name("spotify-meter".into())
            .spawn(move || sample(shared))
            .map_err(|error| error.to_string())?;
        state.running = true;
    }
    Ok(state.peak)
}

fn sample(shared: Arc<Mutex<MeterState>>) {
    let apartment = Com::enter(COINIT_MULTITHREADED).ok();
    let mut devices = None;
    let mut meters: Vec<SessionMeter> = Vec::new();
    let mut next_refresh = Instant::now();
    loop {
        {
            let Ok(mut state) = shared.lock() else { return };
            if state
                .last_request
                .is_none_or(|when| when.elapsed() >= IDLE_TIMEOUT)
            {
                state.running = false;
                state.peak = None;
                return;
            }
        }
        let now = Instant::now();
        if now >= next_refresh {
            next_refresh = now + SESSION_REFRESH;
            if devices.is_none() && apartment.is_some() {
                devices = enumerator().ok();
            }
            meters = devices
                .as_ref()
                .and_then(|devices| spotify_sessions(devices).ok())
                .map(|sessions| {
                    sessions
                        .into_iter()
                        .filter_map(SessionMeter::active)
                        .collect()
                })
                .unwrap_or_default();
        }
        let peak = meters
            .iter()
            .filter_map(SessionMeter::peak)
            .reduce(f32::max);
        let Ok(mut state) = shared.lock() else { return };
        state.peak = peak;
        drop(state);
        thread::sleep(SAMPLE_INTERVAL.saturating_sub(now.elapsed()));
    }
}

fn spotify_sessions(devices: &IMMDeviceEnumerator) -> WindowsResult<Vec<IAudioSessionControl>> {
    let mut matches = Vec::new();
    unsafe {
        // Check every playback device, including an app-specific output selected in Windows.
        let outputs = devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for index in 0..outputs.GetCount()? {
            let Ok(sessions) = outputs
                .Item(index)
                .and_then(|device| device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None))
                .and_then(|manager| manager.GetSessionEnumerator())
            else {
                continue;
            };
            for index in 0..sessions.GetCount().unwrap_or(0) {
                if let Ok(session) = sessions.GetSession(index) {
                    if is_spotify_session(&session) {
                        matches.push(session);
                    }
                }
            }
        }
    }
    Ok(matches)
}

fn is_spotify_session(session: &IAudioSessionControl) -> bool {
    let pid = session
        .cast::<IAudioSessionControl2>()
        .and_then(|control| unsafe { control.GetProcessId() });
    pid.ok().and_then(process_path).is_some_and(|path| {
        path.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("Spotify.exe"))
    })
}

#[tauri::command]
pub async fn spotify_volume(
    webview: WebviewWindow,
    level: Option<f32>,
) -> Result<Option<AudioLevel>, String> {
    if webview.label() != "widget-popup" {
        return Err("Spotify volume is unavailable here.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _com = Com::enter(COINIT_MULTITHREADED)?;
        let sessions = spotify_sessions(&enumerator()?)?;
        let mut current = None;
        let mut active = false;
        let mut write_error = None;
        for session in sessions {
            let state = unsafe { session.GetState() };
            if state
                .as_ref()
                .is_ok_and(|state| *state == AudioSessionStateExpired)
            {
                continue;
            }
            let Ok(volume) = session.cast::<ISimpleAudioVolume>() else {
                continue;
            };
            let result: WindowsResult<AudioLevel> = (|| unsafe {
                if let Some(level) = level {
                    volume.SetMasterVolume(level.clamp(0.0, 1.0), null())?;
                    if level > 0.0 && volume.GetMute()?.as_bool() {
                        volume.SetMute(false, null())?;
                    }
                }
                Ok(AudioLevel {
                    level: volume.GetMasterVolume()?,
                    muted: volume.GetMute()?.as_bool(),
                })
            })();
            match result {
                Ok(value) => {
                    let playing = state.is_ok_and(|state| state == AudioSessionStateActive);
                    if current.is_none() || (playing && !active) {
                        current = Some(value);
                        active = playing;
                    }
                }
                Err(error) if level.is_some() => {
                    write_error = Some(error);
                }
                Err(_) => {}
            }
        }
        match (current, write_error) {
            (None, Some(error)) => Err(error),
            (current, _) => Ok(current),
        }
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

struct SessionMeter {
    meter: IAudioMeterInformation,
    volume: Option<ISimpleAudioVolume>,
}

impl SessionMeter {
    fn active(session: IAudioSessionControl) -> Option<Self> {
        unsafe {
            if session.GetState().ok()? != AudioSessionStateActive {
                return None;
            }
            Some(Self {
                meter: session.cast().ok()?,
                volume: session.cast().ok(),
            })
        }
    }

    fn peak(&self) -> Option<f32> {
        unsafe {
            if self
                .volume
                .as_ref()
                .is_some_and(|volume| volume.GetMute().is_ok_and(|muted| muted.as_bool()))
            {
                return Some(0.0);
            }
            self.meter
                .GetPeakValue()
                .ok()
                .filter(|peak| peak.is_finite())
                .map(|peak| peak.clamp(0.0, 1.0))
        }
    }
}
