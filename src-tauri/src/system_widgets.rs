use crate::{apps::launch_file, previews, widgets};
use serde::{Deserialize, Serialize};
use std::{path::Path, ptr::null};
use tauri::{AppHandle, Emitter, Manager};
use windows::{
    core::{IUnknown_Vtbl, Interface, GUID, HRESULT, PCWSTR},
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{
            eCapture, eConsole, eMultimedia, eRender, EDataFlow, ERole,
            Endpoints::IAudioEndpointVolume, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
            DEVICE_STATE_ACTIVE,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT, COINIT_APARTMENTTHREADED,
            COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED, STGM_READ,
        },
    },
};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::*,
    WindowsAndMessaging::{KBDLLHOOKSTRUCT, LLKHF_EXTENDED},
};

pub(crate) mod audio_spectrum;
pub(crate) mod spotify_meter;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioDevice {
    Output,
    Input,
}

impl AudioDevice {
    fn flow(self) -> EDataFlow {
        match self {
            Self::Output => eRender,
            Self::Input => eCapture,
        }
    }
}

#[derive(Serialize)]
pub struct AudioLevel {
    level: f32,
    muted: bool,
}

#[derive(Serialize)]
pub struct AudioSnapshot {
    output: Option<AudioLevel>,
    input: Option<AudioLevel>,
}

pub(crate) struct Com;

impl Com {
    pub(crate) fn enter(mode: COINIT) -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, mode).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn endpoint(
    enumerator: &IMMDeviceEnumerator,
    device: AudioDevice,
) -> windows::core::Result<IAudioEndpointVolume> {
    unsafe {
        enumerator
            .GetDefaultAudioEndpoint(device.flow(), eMultimedia)?
            .Activate(CLSCTX_ALL, None)
    }
}

fn audio_level(volume: &IAudioEndpointVolume) -> windows::core::Result<AudioLevel> {
    unsafe {
        Ok(AudioLevel {
            level: volume.GetMasterVolumeLevelScalar()?.clamp(0.0, 1.0),
            muted: volume.GetMute()?.as_bool(),
        })
    }
}

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER) }
}

#[derive(Serialize)]
pub struct AudioEndpoint {
    id: String,
    name: String,
    selected: bool,
}

fn endpoint_id(device: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let raw = device.GetId()?;
        let result = raw.to_string();
        CoTaskMemFree(Some(raw.0.cast()));
        Ok(result?)
    }
}

fn endpoint_name(device: &IMMDevice) -> windows::core::Result<String> {
    const NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };
    unsafe {
        let mut value = device.OpenPropertyStore(STGM_READ)?.GetValue(&NAME)?;
        let name = PropVariantToStringAlloc(&value).and_then(|raw| {
            let result = raw.to_string();
            CoTaskMemFree(Some(raw.0.cast()));
            Ok(result?)
        });
        let _ = PropVariantClear(&mut value);
        name
    }
}

fn read_endpoints(
    devices: &IMMDeviceEnumerator,
    device: AudioDevice,
) -> windows::core::Result<Vec<AudioEndpoint>> {
    unsafe {
        let flow = device.flow();
        let active = devices
            .GetDefaultAudioEndpoint(flow, eMultimedia)
            .and_then(|device| endpoint_id(&device))
            .ok();
        let collection = devices.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let mut list = Vec::new();
        for index in 0..collection.GetCount()? {
            let entry = collection.Item(index)?;
            let id = endpoint_id(&entry)?;
            list.push(AudioEndpoint {
                selected: active.as_ref() == Some(&id),
                id,
                name: endpoint_name(&entry).unwrap_or_else(|_| "Audio device".into()),
            });
        }
        list.sort_by(|a, b| {
            b.selected
                .cmp(&a.selected)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(list)
    }
}

// Windows' audio policy client supplies the default-device setter for desktop apps.
windows::core::imp::define_interface!(
    PolicyConfig,
    PolicyConfigVtable,
    0xf8679f50_850a_41cf_9c72_430f290290c8
);
#[repr(C)]
pub struct PolicyConfigVtable {
    base: IUnknown_Vtbl,
    preceding_methods: [usize; 10],
    set_default_endpoint:
        unsafe extern "system" fn(*mut std::ffi::c_void, PCWSTR, ERole) -> HRESULT,
}

#[tauri::command]
pub async fn audio_devices(device: AudioDevice) -> Result<Vec<AudioEndpoint>, String> {
    audio_worker(move || read_endpoints(&enumerator()?, device)).await
}

#[tauri::command]
pub async fn select_audio_device(
    app: AppHandle,
    device: AudioDevice,
    id: String,
) -> Result<Vec<AudioEndpoint>, String> {
    let endpoints = com_worker(move || {
        let devices = enumerator().map_err(|error| error.to_string())?;
        let list = read_endpoints(&devices, device).map_err(|error| error.to_string())?;
        let selected = list
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("This audio device is no longer available.")?;
        if selected.selected {
            return Ok(list);
        }
        unsafe {
            const CLASS: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
            let policy: PolicyConfig =
                CoCreateInstance(&CLASS, None, CLSCTX_ALL).map_err(|_| {
                    "Windows couldn’t change the default audio device on this installation."
                })?;
            let set = |id: &str, role| {
                let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
                (policy.vtable().set_default_endpoint)(policy.as_raw(), PCWSTR(wide.as_ptr()), role)
                    .ok()
            };
            let old_console = devices
                .GetDefaultAudioEndpoint(device.flow(), eConsole)
                .and_then(|entry| endpoint_id(&entry))
                .ok();
            set(&id, eConsole).map_err(|error| error.to_string())?;
            if let Err(error) = set(&id, eMultimedia) {
                if let Some(old) = old_console {
                    let _ = set(&old, eConsole);
                }
                return Err(error.to_string());
            }
        }
        read_endpoints(&devices, device).map_err(|error| error.to_string())
    })
    .await?;
    let _ = app.emit("audio-endpoint-changed", ());
    Ok(endpoints)
}

#[tauri::command]
pub async fn audio_snapshot() -> Result<AudioSnapshot, String> {
    audio_worker(|| {
        let devices = enumerator()?;
        let read = |device| {
            endpoint(&devices, device)
                .and_then(|volume| audio_level(&volume))
                .ok()
        };
        Ok(AudioSnapshot {
            output: read(AudioDevice::Output),
            input: read(AudioDevice::Input),
        })
    })
    .await
}

#[tauri::command]
pub async fn audio_control(
    device: AudioDevice,
    level: Option<f32>,
    toggle_mute: bool,
) -> Result<AudioLevel, String> {
    audio_worker(move || {
        let volume = endpoint(&enumerator()?, device)?;
        unsafe {
            if let Some(level) = level.filter(|value| value.is_finite()) {
                volume.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), null())?;
            }
            if toggle_mute {
                volume.SetMute(!volume.GetMute()?.as_bool(), null())?;
            }
        }
        audio_level(&volume)
    })
    .await
}

async fn audio_worker<T: Send + 'static>(
    action: impl FnOnce() -> windows::core::Result<T> + Send + 'static,
) -> Result<T, String> {
    com_worker(move || action().map_err(|error| error.to_string())).await
}

pub(crate) async fn com_worker<T: Send + 'static>(
    action: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    apartment_worker(COINIT_MULTITHREADED, action).await
}

pub(crate) async fn shell_worker<T: Send + 'static>(
    action: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    apartment_worker(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE, action).await
}

async fn apartment_worker<T: Send + 'static>(
    mode: COINIT,
    action: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _com = Com::enter(mode).map_err(|error| error.to_string())?;
        action()
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) const SPACEBAR_KEY_INPUT: usize = 0x5350_4342;
pub(crate) const WINDOWS_MENU_MASK_KEY: u16 = 0xff;

fn keyboard_input(code: u16, flags: u32) -> INPUT {
    let extended = if matches!(code, VK_LWIN | VK_SNAPSHOT | VK_RWIN) {
        KEYEVENTF_EXTENDEDKEY
    } else {
        0
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                dwFlags: extended | flags,
                dwExtraInfo: SPACEBAR_KEY_INPUT,
                ..Default::default()
            },
        },
    }
}

fn send_inputs(inputs: &[INPUT]) -> usize {
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        ) as usize
    }
}

pub(crate) fn release_windows_key_without_start(key: u16) -> bool {
    // Mask the Win tap for Explorer before releasing the modifier, as PowerToys does.
    let inputs = [
        keyboard_input(WINDOWS_MENU_MASK_KEY, 0),
        keyboard_input(WINDOWS_MENU_MASK_KEY, KEYEVENTF_KEYUP),
        keyboard_input(key, KEYEVENTF_KEYUP),
    ];
    let sent = send_inputs(&inputs);
    if sent == 1 {
        send_inputs(&inputs[1..2]);
    }
    sent == inputs.len()
}

pub(crate) fn replay_windows_chord(win: u16, key: &KBDLLHOOKSTRUCT) -> bool {
    let code = key.vkCode as u16;
    let (code, flags) = if code == VK_PACKET {
        (0, KEYEVENTF_UNICODE)
    } else if key.flags & LLKHF_EXTENDED != 0 {
        (code, KEYEVENTF_EXTENDEDKEY)
    } else {
        (code, 0)
    };
    let mut key_down = keyboard_input(code, flags);
    key_down.Anonymous.ki.wScan = key.scanCode as u16;
    // Queue the modifier and the original chord key in order before consuming that key-down.
    let inputs = [keyboard_input(win, 0), key_down];
    send_inputs(&inputs) == inputs.len()
}

pub(crate) fn press_keys(keys: &[u16]) -> Result<(), String> {
    let inputs: Vec<_> = keys
        .iter()
        .map(|&code| keyboard_input(code, 0))
        .chain(
            keys.iter()
                .rev()
                .map(|&code| keyboard_input(code, KEYEVENTF_KEYUP)),
        )
        .collect();
    let sent = send_inputs(&inputs);
    if sent == inputs.len() {
        return Ok(());
    }
    if sent > 0 {
        let releases: Vec<_> = keys
            .iter()
            .take(sent.min(keys.len()))
            .rev()
            .map(|&code| keyboard_input(code, KEYEVENTF_KEYUP))
            .collect();
        send_inputs(&releases);
    }
    Err("Windows couldn’t run this shortcut.".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemAction {
    Desktop,
    Notifications,
    Wifi,
    Location,
    Bluetooth,
    Keyboard,
    Mixer,
    Display,
    NotificationSettings,
    Region,
    Fullscreen,
    SnippingTool,
}

#[tauri::command]
pub async fn system_action(app: AppHandle, action: SystemAction) -> Result<(), String> {
    previews::close_all(&app);
    if app.state::<widgets::OpenFolder>().active() {
        let _ = widgets::close_folder(app.clone(), app.state());
    }
    match action {
        SystemAction::Desktop => press_keys(&[VK_LWIN, b'D' as u16]),
        SystemAction::Notifications => press_keys(&[VK_LWIN, b'N' as u16]),
        SystemAction::Keyboard => press_keys(&[VK_LWIN, VK_CONTROL, b'O' as u16]),
        SystemAction::Region => press_keys(&[VK_LWIN, VK_SHIFT, b'S' as u16]),
        SystemAction::Fullscreen => press_keys(&[VK_LWIN, VK_SNAPSHOT]),
        other => {
            let target = match other {
                SystemAction::Mixer => "ms-settings:apps-volume",
                SystemAction::Wifi => "ms-settings:network-wifi",
                SystemAction::Location => "ms-settings:privacy-location",
                SystemAction::Bluetooth => "ms-settings:bluetooth",
                SystemAction::Display => "ms-settings:display",
                SystemAction::NotificationSettings => "ms-settings:notifications",
                SystemAction::SnippingTool => "SnippingTool.exe",
                _ => unreachable!(),
            };
            if launch_file(Path::new(target), None)
                || (matches!(other, SystemAction::SnippingTool)
                    && launch_file(
                        Path::new("shell:AppsFolder\\Microsoft.ScreenSketch_8wekyb3d8bbwe!App"),
                        None,
                    ))
            {
                Ok(())
            } else {
                Err("Windows couldn’t open this tool.".into())
            }
        }
    }
}
