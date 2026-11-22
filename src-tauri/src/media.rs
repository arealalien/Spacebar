use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    ops::RangeInclusive,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager};
use windows::{
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as ManagerApi,
        GlobalSystemMediaTransportControlsSessionMediaProperties as Properties,
        GlobalSystemMediaTransportControlsSessionPlaybackControls as Controls,
        GlobalSystemMediaTransportControlsSessionPlaybackInfo as Playback,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        GlobalSystemMediaTransportControlsSessionTimelineProperties as Timeline,
    },
    Storage::Streams::DataReader,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

const TICKS_PER_SECOND: f64 = 10_000_000.0;
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpotifyTimeline {
    position: f64,
    duration: f64,
    updated_at: Option<f64>,
    playback_rate: f64,
    can_seek: bool,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spotify {
    available: bool,
    title: String,
    artist: String,
    artwork: Option<String>,
    #[serde(flatten)]
    buttons: PlaybackButtons,
    timeline: Option<SpotifyTimeline>,
}

#[derive(Default)]
pub struct MediaState(Mutex<Option<(String, Option<String>)>>);

#[derive(Clone)]
pub(crate) struct MediaIdentity {
    pub(crate) model_id: Option<String>,
    pub(crate) executable: Option<PathBuf>,
}

impl MediaIdentity {
    pub(crate) fn is_known(&self) -> bool {
        self.model_id.is_some() || self.executable.is_some()
    }

    fn match_strength(&self, source: &str) -> Option<u8> {
        if self
            .model_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(source))
        {
            return Some(2);
        }
        let executable = self.executable.as_deref()?;
        let source_path = Path::new(source);
        if source_path.is_absolute() {
            return executable
                .as_os_str()
                .eq_ignore_ascii_case(source)
                .then_some(2);
        }
        // Unpackaged sessions can report an executable name instead of an explicit AppUserModelID.
        ([executable.file_name(), executable.file_stem()]
            .into_iter()
            .flatten()
            .any(|name| name.eq_ignore_ascii_case(source)))
        .then_some(1)
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlaybackButtons {
    playing: bool,
    can_previous: bool,
    can_toggle: bool,
    can_next: bool,
}

impl PlaybackButtons {
    fn read(playback: &Playback, controls: &Controls) -> windows::core::Result<Self> {
        let status = playback.PlaybackStatus()?;
        if status == Status::Closed {
            return Ok(Self::default());
        }
        let playing = status == Status::Playing;
        let can_toggle = controls.IsPlayPauseToggleEnabled().unwrap_or(false)
            || if playing {
                controls.IsPauseEnabled()
            } else {
                controls.IsPlayEnabled()
            }
            .unwrap_or(false);
        Ok(Self {
            playing,
            can_previous: controls.IsPreviousEnabled().unwrap_or(false),
            can_toggle,
            can_next: controls.IsNextEnabled().unwrap_or(false),
        })
    }

    fn available(self) -> bool {
        self.can_previous || self.can_toggle || self.can_next
    }
}

struct WinRt;
impl WinRt {
    fn enter() -> windows::core::Result<Self> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        Ok(Self)
    }
}
impl Drop for WinRt {
    fn drop(&mut self) {
        unsafe {
            RoUninitialize();
        }
    }
}

fn spotify_session() -> windows::core::Result<Option<Session>> {
    let manager = ManagerApi::RequestAsync()?.get()?;
    let sessions = manager.GetSessions()?;
    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;
        if session
            .SourceAppUserModelId()
            .is_ok_and(|source| source.to_string().to_ascii_lowercase().contains("spotify"))
        {
            return Ok(Some(session));
        }
    }
    Ok(None)
}

fn album_art(properties: &Properties) -> Option<String> {
    let stream = properties
        .Thumbnail()
        .ok()?
        .OpenReadAsync()
        .ok()?
        .get()
        .ok()?;
    let length = stream.Size().ok()?.min(u32::MAX as u64) as u32;
    if length == 0 || length > 2 * 1024 * 1024 {
        return None;
    }
    let mime = stream.ContentType().ok()?.to_string();
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    let read = reader.LoadAsync(length).ok()?.get().ok()?;
    let mut bytes = vec![0; read as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let mime = match mime.as_str() {
        "image/jpeg" | "image/png" | "image/webp" | "image/gif" => mime.as_str(),
        _ if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "image/png",
        _ if bytes.starts_with(b"\xff\xd8\xff") => "image/jpeg",
        _ => return None,
    };
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn track_range(timeline: &Timeline) -> Option<RangeInclusive<i64>> {
    let start = timeline.StartTime().ok()?.Duration;
    let end = timeline.EndTime().ok()?.Duration;
    (end.checked_sub(start)? > 0).then_some(start..=end)
}

fn seek_range(timeline: &Timeline, track: &RangeInclusive<i64>) -> Option<RangeInclusive<i64>> {
    let start = timeline.MinSeekTime().ok()?.Duration.max(*track.start());
    let end = timeline.MaxSeekTime().ok()?.Duration.min(*track.end());
    (start < end).then_some(start..=end)
}

fn read_timeline(
    session: &Session,
    playback: &Playback,
    controls: &Controls,
) -> Option<SpotifyTimeline> {
    let timeline = session.GetTimelineProperties().ok()?;
    let track = track_range(&timeline)?;
    let position = timeline
        .Position()
        .ok()?
        .Duration
        .clamp(*track.start(), *track.end());
    let updated_at = timeline.LastUpdatedTime().ok().and_then(|updated| {
        (updated.UniversalTime >= UNIX_EPOCH_TICKS)
            .then(|| (updated.UniversalTime - UNIX_EPOCH_TICKS) as f64 / 10_000.0)
    });
    let playback_rate = playback
        .PlaybackRate()
        .and_then(|rate| rate.Value())
        .ok()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .unwrap_or(1.0);
    Some(SpotifyTimeline {
        position: (position - track.start()) as f64 / TICKS_PER_SECOND,
        duration: (track.end() - track.start()) as f64 / TICKS_PER_SECOND,
        updated_at,
        playback_rate,
        can_seek: controls.IsPlaybackPositionEnabled().unwrap_or(false)
            && seek_range(&timeline, &track).is_some(),
    })
}

fn snapshot(media: &MediaState) -> windows::core::Result<Spotify> {
    let _runtime = WinRt::enter()?;
    let Some(session) = spotify_session()? else {
        return Ok(Spotify::default());
    };
    let playback = session.GetPlaybackInfo()?;
    let controls = playback.Controls()?;
    let properties = session.TryGetMediaPropertiesAsync()?.get()?;
    let title = properties.Title()?.to_string();
    let artist = properties.Artist()?.to_string();
    let key = format!(
        "{}\0{title}\0{artist}\0{}",
        session.SourceAppUserModelId()?,
        properties.AlbumTitle()?
    );
    let artwork = media.0.lock().ok().and_then(|mut cache| {
        if cache.as_ref().is_none_or(|(track, _)| track != &key) {
            *cache = Some((key, album_art(&properties)));
        }
        cache.as_ref().and_then(|(_, art)| art.clone())
    });
    Ok(Spotify {
        available: true,
        title,
        artist,
        artwork,
        buttons: PlaybackButtons::read(&playback, &controls)?,
        timeline: read_timeline(&session, &playback, &controls),
    })
}

#[tauri::command]
pub async fn spotify_snapshot(app: AppHandle) -> Result<Spotify, String> {
    tauri::async_runtime::spawn_blocking(move || {
        snapshot(&app.state::<MediaState>()).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Previous,
    Toggle,
    Next,
    Seek,
}

fn seek(session: &Session, position: f64) -> windows::core::Result<bool> {
    if !position.is_finite()
        || !session
            .GetPlaybackInfo()?
            .Controls()?
            .IsPlaybackPositionEnabled()?
    {
        return Ok(false);
    }
    let timeline = session.GetTimelineProperties()?;
    let Some(track) = track_range(&timeline) else {
        return Ok(false);
    };
    let Some(range) = seek_range(&timeline, &track) else {
        return Ok(false);
    };
    let offset = (position * TICKS_PER_SECOND).round() as i64;
    let target = track
        .start()
        .saturating_add(offset)
        .clamp(*range.start(), *range.end());
    session.TryChangePlaybackPositionAsync(target)?.get()
}

fn control(action: Action, position: Option<f64>) -> windows::core::Result<bool> {
    let _runtime = WinRt::enter()?;
    let Some(session) = spotify_session()? else {
        return Ok(false);
    };
    apply_control(&session, action, position)
}

fn apply_control(
    session: &Session,
    action: Action,
    position: Option<f64>,
) -> windows::core::Result<bool> {
    match action {
        Action::Previous => session.TrySkipPreviousAsync()?.get(),
        Action::Toggle
            if session
                .GetPlaybackInfo()?
                .Controls()?
                .IsPlayPauseToggleEnabled()? =>
        {
            session.TryTogglePlayPauseAsync()?.get()
        }
        Action::Toggle if session.GetPlaybackInfo()?.PlaybackStatus()? == Status::Playing => {
            session.TryPauseAsync()?.get()
        }
        Action::Toggle => session.TryPlayAsync()?.get(),
        Action::Next => session.TrySkipNextAsync()?.get(),
        Action::Seek => position.map_or(Ok(false), |position| seek(session, position)),
    }
}

fn matching_session(
    identity: &MediaIdentity,
) -> windows::core::Result<Option<(Session, PlaybackButtons)>> {
    let manager = ManagerApi::RequestAsync()?.get()?;
    let sessions = manager.GetSessions()?;
    let mut best = None;
    for index in 0..sessions.Size()? {
        let Ok(session) = sessions.GetAt(index) else {
            continue;
        };
        let Ok(source) = session.SourceAppUserModelId() else {
            continue;
        };
        let Some(strength) = identity.match_strength(&source.to_string()) else {
            continue;
        };
        let Ok(buttons) = session
            .GetPlaybackInfo()
            .and_then(|playback| PlaybackButtons::read(&playback, &playback.Controls()?))
        else {
            continue;
        };
        let rank = (strength, buttons.playing);
        if buttons.available()
            && best
                .as_ref()
                .is_none_or(|(previous, _, _)| rank > *previous)
        {
            best = Some((rank, session, buttons));
        }
    }
    Ok(best.map(|(_, session, buttons)| (session, buttons)))
}

pub(crate) async fn preview_playback(
    identity: MediaIdentity,
    action: Option<Action>,
) -> Result<Option<PlaybackButtons>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _runtime = WinRt::enter().map_err(|error| error.to_string())?;
        let session = matching_session(&identity).map_err(|error| error.to_string())?;
        let Some(action) = action else {
            return Ok(session.map(|(_, buttons)| buttons));
        };
        let unavailable = "This app’s media session couldn’t perform that action.";
        let (session, _) = session.ok_or(unavailable)?;
        if !apply_control(&session, action, None).map_err(|error| error.to_string())? {
            return Err(unavailable.into());
        }
        let buttons = session
            .GetPlaybackInfo()
            .and_then(|playback| PlaybackButtons::read(&playback, &playback.Controls()?))
            .map_err(|error| error.to_string())?;
        Ok(buttons.available().then_some(buttons))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn spotify_control(
    app: AppHandle,
    action: Action,
    position: Option<f64>,
) -> Result<(), String> {
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        control(action, position).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if accepted {
        let _ = app.emit("spotify-changed", ());
        Ok(())
    } else {
        Err(
            "Spotify couldn’t perform that action. Check that a track is open in the desktop app."
                .into(),
        )
    }
}

#[tauri::command]
pub fn open_spotify() -> Result<(), String> {
    if crate::apps::launch_file(Path::new("spotify:"), None) {
        Ok(())
    } else {
        Err("Couldn’t open Spotify. Check that the desktop app is installed.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_the_apps_packaged_id_or_its_executable_identity() {
        let desktop = MediaIdentity {
            model_id: None,
            executable: Some(PathBuf::from(r"C:\Players\Spotify.exe")),
        };
        assert_eq!(desktop.match_strength(r"c:\players\SPOTIFY.exe"), Some(2));
        assert_eq!(desktop.match_strength("Spotify.exe"), Some(1));
        assert_eq!(desktop.match_strength("Spotify"), Some(1));
        assert_eq!(desktop.match_strength(r"C:\Other\Spotify.exe"), None);
        assert_eq!(desktop.match_strength("OtherSpotify.exe"), None);
        let packaged = MediaIdentity {
            model_id: Some("SpotifyAB.SpotifyMusic_example!Spotify".into()),
            executable: None,
        };
        assert_eq!(
            packaged.match_strength("spotifyab.spotifymusic_example!spotify"),
            Some(2)
        );
        assert_eq!(
            packaged.match_strength("SpotifyAB.SpotifyMusic_example!AnotherApp"),
            None
        );
    }
}
