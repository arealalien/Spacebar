use crate::system_widgets::com_worker;
use serde::Serialize;
use std::{
    collections::HashMap,
    ffi::c_void,
    io,
    ptr::{addr_of, null, null_mut},
    slice,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use windows::{
    core::{Interface, HSTRING},
    Devices::{
        Bluetooth::{
            BluetoothLEDevice,
            GenericAttributeProfile::{GattSession, GattSessionStatus},
        },
        Enumeration::{
            DeviceInformation, DeviceInformationKind, DeviceInformationUpdate,
            DevicePairingResultStatus, DeviceWatcher,
        },
    },
    Foundation::{IPropertyValue, TypedEventHandler},
};
use windows_collections::IIterable;
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{ERROR_ACCESS_DENIED, HANDLE},
        NetworkManagement::WiFi::*,
    },
};

fn require_panel(source: &WebviewWindow) -> Result<(), String> {
    if source.label() == "system" {
        Ok(())
    } else {
        Err("Open Quick Settings to manage connections.".into())
    }
}

struct Wifi(HANDLE);
struct WlanMemory(*mut c_void);
static WIFI_CONNECTION: Mutex<()> = Mutex::new(());

impl Drop for Wifi {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, null());
        }
    }
}

impl Drop for WlanMemory {
    fn drop(&mut self) {
        unsafe {
            WlanFreeMemory(self.0);
        }
    }
}

fn wlan_result(status: u32) -> Result<(), String> {
    match status {
        0 => Ok(()),
        ERROR_ACCESS_DENIED => Err(
            "Windows denied Wi-Fi access. Check Location privacy settings and desktop app access."
                .into(),
        ),
        status => Err(io::Error::from_raw_os_error(status as i32).to_string()),
    }
}

impl Wifi {
    fn open() -> Result<Self, String> {
        let mut handle = null_mut();
        wlan_result(unsafe { WlanOpenHandle(2, null(), &mut 0, &mut handle) })?;
        Ok(Self(handle))
    }

    fn adapters(&self) -> Result<Vec<GUID>, String> {
        let mut list = null_mut();
        wlan_result(unsafe { WlanEnumInterfaces(self.0, null(), &mut list) })?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        let _memory = WlanMemory(list.cast());
        Ok(unsafe {
            slice::from_raw_parts(
                addr_of!((*list).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>(),
                (*list).dwNumberOfItems as usize,
            )
            .iter()
            .map(|adapter| adapter.InterfaceGuid)
            .collect()
        })
    }

    fn networks(&self, adapters: &[GUID]) -> Result<Vec<Network>, String> {
        let mut networks = Vec::new();
        let mut failure = None;
        let mut success = false;
        for adapter in adapters {
            let mut list = null_mut();
            let result =
                unsafe { WlanGetAvailableNetworkList(self.0, adapter, 0, null(), &mut list) };
            if let Err(error) = wlan_result(result) {
                failure = Some(error);
                continue;
            }
            success = true;
            if list.is_null() {
                continue;
            }
            let _memory = WlanMemory(list.cast());
            let rows = unsafe {
                slice::from_raw_parts(
                    addr_of!((*list).Network).cast::<WLAN_AVAILABLE_NETWORK>(),
                    (*list).dwNumberOfItems as usize,
                )
            };
            networks.extend(
                rows.iter()
                    .filter_map(|network| Network::new(*adapter, *network)),
            );
        }
        if !success {
            if let Some(error) = failure {
                return Err(error);
            }
        }
        networks.sort_by_key(|network| {
            (
                std::cmp::Reverse(network.connected()),
                std::cmp::Reverse(network.raw.wlanSignalQuality),
            )
        });
        Ok(networks)
    }
}

struct Network {
    adapter: GUID,
    raw: WLAN_AVAILABLE_NETWORK,
    id: String,
    name: String,
    profile: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    id: String,
    name: String,
    signal: u32,
    connected: bool,
    secured: bool,
    saved: bool,
    needs_password: bool,
    can_connect: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

impl Network {
    fn new(adapter: GUID, raw: WLAN_AVAILABLE_NETWORK) -> Option<Self> {
        let ssid = raw
            .dot11Ssid
            .ucSSID
            .get(..raw.dot11Ssid.uSSIDLength as usize)?;
        if ssid.is_empty() {
            return None;
        }
        let name = String::from_utf8_lossy(ssid).into_owned();
        let profile = String::from_utf16_lossy(
            &raw.strProfileName[..raw
                .strProfileName
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(raw.strProfileName.len())],
        );
        let id = format!(
            "{:08x}{:04x}{:04x}{}:{}:{}:{}:{profile}",
            adapter.data1,
            adapter.data2,
            adapter.data3,
            hex(&adapter.data4),
            hex(ssid),
            raw.dot11DefaultAuthAlgorithm,
            raw.dot11DefaultCipherAlgorithm
        );
        Some(Self {
            adapter,
            raw,
            id,
            name,
            profile,
        })
    }

    fn connected(&self) -> bool {
        self.raw.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0
    }

    fn personal_security(&self) -> Option<(&'static str, &'static str)> {
        let auth = match self.raw.dot11DefaultAuthAlgorithm {
            DOT11_AUTH_ALGO_RSNA_PSK => "WPA2PSK",
            DOT11_AUTH_ALGO_WPA_PSK => "WPAPSK",
            DOT11_AUTH_ALGO_WPA3_SAE => "WPA3SAE",
            _ => return None,
        };
        let cipher = match self.raw.dot11DefaultCipherAlgorithm {
            DOT11_CIPHER_ALGO_CCMP => "AES",
            DOT11_CIPHER_ALGO_TKIP => "TKIP",
            _ => return None,
        };
        Some((auth, cipher))
    }

    fn tile(&self) -> WifiNetwork {
        let saved = !self.profile.is_empty();
        let secured = self.raw.bSecurityEnabled != 0;
        let personal = self.personal_security().is_some();
        WifiNetwork {
            id: self.id.clone(),
            name: self.name.clone(),
            connected: self.connected(),
            saved,
            secured,
            signal: self.raw.wlanSignalQuality,
            needs_password: secured && !saved && personal,
            can_connect: self.raw.bNetworkConnectable != 0 && (saved || !secured || personal),
        }
    }
}

fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn temporary_profile(network: &Network, password: Option<&str>) -> Result<String, String> {
    let (auth, encryption) = network
        .personal_security()
        .ok_or("Use Windows Wi-Fi settings for this network.")?;
    let password = password.ok_or("Enter the Wi-Fi password.")?;
    let as_hex = auth != "WPA3SAE"
        && password.len() == 64
        && password.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !as_hex && (!(8..=63).contains(&password.len()) || password.chars().any(char::is_control)) {
        return Err("Enter a Wi-Fi password of 8 to 63 characters.".into());
    }
    let kind = if as_hex { "networkKey" } else { "passPhrase" };
    let key = xml_text(password);
    let ssid = hex(&network.raw.dot11Ssid.ucSSID[..network.raw.dot11Ssid.uSSIDLength as usize]);
    Ok(format!(
        "<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{ssid}</name><SSIDConfig><SSID><hex>{ssid}</hex></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>manual</connectionMode><MSM><security><authEncryption><authentication>{auth}</authentication><encryption>{encryption}</encryption><useOneX>false</useOneX></authEncryption><sharedKey><keyType>{kind}</keyType><protected>false</protected><keyMaterial>{key}</keyMaterial></sharedKey></security></MSM></WLANProfile>"
    ))
}

#[tauri::command]
pub async fn wifi_networks(source: WebviewWindow, scan: bool) -> Result<Vec<WifiNetwork>, String> {
    require_panel(&source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let wifi = Wifi::open()?;
        let adapters = wifi.adapters()?;
        // Read first so Windows can report location permission or radio errors distinctly.
        let mut networks = wifi.networks(&adapters)?;
        let mut scanning = false;
        if scan {
            for adapter in &adapters {
                scanning |= unsafe { WlanScan(wifi.0, adapter, null(), null(), null()) == 0 };
            }
        }
        if scanning {
            // WLAN scan results may take up to four seconds to populate.
            thread::sleep(Duration::from_secs(4));
            networks = wifi.networks(&adapters)?;
        }
        Ok(networks.iter().map(Network::tile).collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn wifi_connect(
    source: WebviewWindow,
    id: String,
    password: Option<String>,
) -> Result<(), String> {
    require_panel(&source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _connection = WIFI_CONNECTION
            .try_lock()
            .map_err(|_| "A Wi-Fi connection is already in progress.")?;
        let wifi = Wifi::open()?;
        let mut network = wifi
            .networks(&wifi.adapters()?)?
            .into_iter()
            .find(|network| network.id == id)
            .ok_or("This network is no longer visible.")?;
        if network.connected() {
            return Ok(());
        }
        if network.raw.bNetworkConnectable == 0 {
            return Err("Windows reports that this network is unavailable.".into());
        }
        let (mode, profile) = if !network.profile.is_empty() {
            (wlan_connection_mode_profile, Some(network.profile.clone()))
        } else if network.raw.bSecurityEnabled == 0 {
            (wlan_connection_mode_discovery_unsecure, None)
        } else {
            (
                wlan_connection_mode_temporary_profile,
                Some(temporary_profile(&network, password.as_deref())?),
            )
        };
        let wide = profile
            .as_ref()
            .map(|profile| profile.encode_utf16().chain(Some(0)).collect::<Vec<_>>());
        let params = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: mode,
            strProfile: wide.as_ref().map_or(null(), |profile| profile.as_ptr()),
            pDot11Ssid: if wide.is_none() {
                &mut network.raw.dot11Ssid
            } else {
                null_mut()
            },
            dot11BssType: network.raw.dot11BssType,
            ..Default::default()
        };
        wlan_result(unsafe { WlanConnect(wifi.0, &network.adapter, &params, null()) })?;
        for _ in 0..32 {
            thread::sleep(Duration::from_millis(375));
            if wifi.networks(&[network.adapter])?.iter().any(|candidate| {
                candidate.connected()
                    && candidate.raw.bSecurityEnabled == network.raw.bSecurityEnabled
                    && candidate.raw.dot11Ssid.ucSSID
                        [..candidate.raw.dot11Ssid.uSSIDLength as usize]
                        == network.raw.dot11Ssid.ucSSID
                            [..network.raw.dot11Ssid.uSSIDLength as usize]
            }) {
                return Ok(());
            }
        }
        Err(
            "Windows has not confirmed the connection. Refresh the list to check its status."
                .into(),
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[derive(Default)]
pub struct BluetoothConnections(Mutex<HashMap<String, GattSession>>);

const BLUETOOTH_CLASSIC: &str =
    "System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\"";
const BLUETOOTH_LE: &str =
    "System.Devices.Aep.ProtocolId:=\"{bb7bb05e-5972-42b5-94fc-76eaa7084d49}\"";
const CONNECTED: &str = "System.Devices.Aep.IsConnected";
const PROTOCOL: &str = "System.Devices.Aep.ProtocolId";
const CLASSIC_PROTOCOL: windows::core::GUID =
    windows::core::GUID::from_u128(0xe0cbf06c_cd8b_4647_bb8a_263b43f0f974);
const LE_PROTOCOL: windows::core::GUID =
    windows::core::GUID::from_u128(0xbb7bb05e_5972_42b5_94fc_76eaa7084d49);
const KIND: DeviceInformationKind = DeviceInformationKind::AssociationEndpoint;

fn bluetooth_error(message: &str) -> windows::core::Error {
    windows::core::Error::new(windows::Win32::Foundation::E_FAIL, message)
}

fn begin_bluetooth_action<T: Send + 'static>(
    app: &AppHandle,
    start: impl FnOnce() -> windows::core::Result<T> + Send + 'static,
) -> windows::core::Result<T> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = send.send(start());
    })
    .map_err(|_| bluetooth_error("Couldn’t start the Windows Bluetooth action."))?;
    receive
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| bluetooth_error("Windows Bluetooth is busy. Try again."))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothEntry {
    id: String,
    name: String,
    paired: bool,
    connected: bool,
    can_pair: bool,
    low_energy: bool,
}

fn bluetooth_properties() -> IIterable<HSTRING> {
    vec![HSTRING::from(CONNECTED), HSTRING::from(PROTOCOL)].into()
}

fn device_protocol(device: &DeviceInformation) -> windows::core::Result<windows::core::GUID> {
    device
        .Properties()?
        .Lookup(&HSTRING::from(PROTOCOL))?
        .cast::<IPropertyValue>()?
        .GetGuid()
}

fn discover_bluetooth() -> windows::core::Result<Vec<DeviceInformation>> {
    let selector = HSTRING::from(format!("({BLUETOOTH_CLASSIC}) OR ({BLUETOOTH_LE})"));
    let watcher = DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &selector,
        &bluetooth_properties(),
        KIND,
    )?;
    let results = Arc::new(Mutex::new(HashMap::<HSTRING, DeviceInformation>::new()));
    let added = Arc::clone(&results);
    let add = watcher.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
        move |_, device| {
            if let (Some(device), Ok(mut entries)) = (device.as_ref(), added.lock()) {
                entries.insert(device.Id()?, device.clone());
            }
            Ok(())
        },
    ))?;
    let updated = Arc::clone(&results);
    let update = watcher.Updated(
        &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, change| {
            if let (Some(change), Ok(entries)) = (change.as_ref(), updated.lock()) {
                if let Some(device) = entries.get(&change.Id()?) {
                    device.Update(change)?;
                }
            }
            Ok(())
        }),
    )?;
    let removed = Arc::clone(&results);
    let remove = watcher.Removed(
        &TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, change| {
            if let (Some(change), Ok(mut entries)) = (change.as_ref(), removed.lock()) {
                entries.remove(&change.Id()?);
            }
            Ok(())
        }),
    )?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let complete = watcher.EnumerationCompleted(&TypedEventHandler::<
        DeviceWatcher,
        windows::core::IInspectable,
    >::new(move |_, _| {
        let _ = send.try_send(());
        Ok(())
    }))?;
    watcher.Start()?;
    // Return discoveries even when Windows keeps looking for nearby devices.
    let _ = receive.recv_timeout(Duration::from_secs(6));
    let _ = watcher.Stop();
    let _ = watcher.RemoveAdded(add);
    let _ = watcher.RemoveUpdated(update);
    let _ = watcher.RemoveRemoved(remove);
    let _ = watcher.RemoveEnumerationCompleted(complete);
    let results = results
        .lock()
        .map_err(|_| bluetooth_error("Couldn’t read Bluetooth discoveries."))?;
    Ok(results.values().cloned().collect())
}

#[tauri::command]
pub async fn bluetooth_devices(source: WebviewWindow) -> Result<Vec<BluetoothEntry>, String> {
    require_panel(&source)?;
    com_worker(|| {
        let read = || -> windows::core::Result<Vec<BluetoothEntry>> {
            let mut entries = Vec::new();
            for device in discover_bluetooth()? {
                let name = device.Name()?.to_string();
                let pairing = device.Pairing()?;
                let paired = pairing.IsPaired()?;
                let can_pair = pairing.CanPair()?;
                if name.trim().is_empty() || !(paired || can_pair) {
                    continue;
                }
                let connected = device
                    .Properties()?
                    .Lookup(&HSTRING::from(CONNECTED))
                    .and_then(|property| property.cast::<IPropertyValue>())
                    .and_then(|property| property.GetBoolean())
                    .unwrap_or(false);
                entries.push(BluetoothEntry {
                    id: device.Id()?.to_string(),
                    name,
                    paired,
                    connected,
                    can_pair,
                    low_energy: device_protocol(&device)
                        .is_ok_and(|protocol| protocol == LE_PROTOCOL),
                });
            }
            entries.sort_by(|a, b| {
                b.connected
                    .cmp(&a.connected)
                    .then_with(|| b.paired.cmp(&a.paired))
                    .then_with(|| a.name.cmp(&b.name))
            });
            Ok(entries)
        };
        read().map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn bluetooth_connect(
    app: AppHandle,
    source: WebviewWindow,
    id: String,
) -> Result<String, String> {
    require_panel(&source)?;
    com_worker(move || {
        let connect = || -> windows::core::Result<&'static str> {
            let device_id = HSTRING::from(&id);
            let device = DeviceInformation::CreateFromIdAsyncWithKindAndAdditionalProperties(
                &device_id,
                &bluetooth_properties(),
                KIND,
            )?
            .get()?;
            let protocol = device_protocol(&device)?;
            if protocol != CLASSIC_PROTOCOL && protocol != LE_PROTOCOL {
                return Err(bluetooth_error("This is not a Bluetooth device."));
            }
            let pairing = device.Pairing()?;
            if !pairing.IsPaired()? {
                if !pairing.CanPair()? {
                    return Err(bluetooth_error(
                        "This device is no longer available for pairing.",
                    ));
                }
                let status = begin_bluetooth_action(&app, move || pairing.PairAsync())?
                    .get()?
                    .Status()?;
                return match status {
                    DevicePairingResultStatus::Paired
                    | DevicePairingResultStatus::AlreadyPaired => Ok("Paired"),
                    _ => Err(bluetooth_error(
                        "Pairing did not finish. Try again in Windows Bluetooth settings.",
                    )),
                };
            }
            if protocol == CLASSIC_PROTOCOL {
                return Ok("Windows");
            }
            let peripheral =
                begin_bluetooth_action(&app, move || BluetoothLEDevice::FromIdAsync(&device_id))?
                    .get()?;
            let session =
                GattSession::FromDeviceIdAsync(&peripheral.BluetoothDeviceId()?)?.get()?;
            if !session.CanMaintainConnection()? {
                return Ok("Windows");
            }
            session.SetMaintainConnection(true)?;
            for _ in 0..20 {
                if session.SessionStatus()? == GattSessionStatus::Active {
                    if let Ok(mut connections) = app.state::<BluetoothConnections>().0.lock() {
                        connections.insert(id.clone(), session);
                    }
                    return Ok("Connected");
                }
                thread::sleep(Duration::from_millis(350));
            }
            let _ = session.SetMaintainConnection(false);
            Ok("Windows")
        };
        connect()
            .map(str::to_owned)
            .map_err(|error| error.to_string())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_wifi_profiles_keep_ssid_bytes_and_escape_key_material() {
        let mut raw = WLAN_AVAILABLE_NETWORK {
            dot11DefaultAuthAlgorithm: DOT11_AUTH_ALGO_RSNA_PSK,
            dot11DefaultCipherAlgorithm: DOT11_CIPHER_ALGO_CCMP,
            ..Default::default()
        };
        raw.dot11Ssid.uSSIDLength = 3;
        raw.dot11Ssid.ucSSID[..3].copy_from_slice(b"A&\xff");
        let network = Network::new(GUID::default(), raw).unwrap();
        let profile = temporary_profile(&network, Some("A&B<pass>")).unwrap();
        assert!(profile.contains("<hex>4126FF</hex>"));
        assert!(profile.contains("<keyMaterial>A&amp;B&lt;pass&gt;</keyMaterial>"));
        assert!(temporary_profile(&network, Some("short")).is_err());
    }
}
