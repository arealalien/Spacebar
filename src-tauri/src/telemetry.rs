use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    ptr::{null, null_mut},
    slice,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows_sys::Win32::{
    Foundation::FILETIME,
    NetworkManagement::{
        IpHelper::{FreeMibTable, GetIfTable2},
        Ndis::IfOperStatusUp,
    },
    Storage::FileSystem::GetDiskFreeSpaceExW,
    System::{
        Performance::*,
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::GetSystemTimes,
    },
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capacity {
    total: u64,
    available: u64,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Network {
    received: u64,
    sent: u64,
    rates: Option<[f64; 2]>,
    history: VecDeque<[f64; 2]>,
}

#[derive(Clone, Default, Serialize)]
pub struct Snapshot {
    cpu: Option<f64>,
    gpu: Option<f64>,
    ram: Option<Capacity>,
    disk: Option<Capacity>,
    network: Option<Network>,
}

#[derive(Default)]
pub struct Telemetry(Mutex<Sampler>);

#[derive(Default)]
struct Sampler {
    previous: Option<Instant>,
    processor: Option<(u64, u64)>,
    adapters: Option<HashMap<u64, [u64; 2]>>,
    traffic: Network,
    gpu: Option<GpuCounter>,
    cached: Snapshot,
}

fn processor_time() -> Option<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } != 0)
        .then(|| (ticks(idle), ticks(kernel) + ticks(user)))
}

fn memory() -> Option<Capacity> {
    let mut memory = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    (unsafe { GlobalMemoryStatusEx(&mut memory) } != 0).then_some(Capacity {
        total: memory.ullTotalPhys,
        available: memory.ullAvailPhys,
    })
}

fn disk() -> Option<Capacity> {
    let drive = std::env::var("SystemDrive").ok()?;
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(Some(0)).collect();
    let (mut total, mut available) = (0, 0);
    (unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, null_mut()) } != 0)
        .then_some(Capacity { total, available })
}

fn network_counters() -> Option<HashMap<u64, [u64; 2]>> {
    let mut table = null_mut();
    if unsafe { GetIfTable2(&mut table) } != 0 || table.is_null() {
        return None;
    }
    let mut adapters = HashMap::new();
    unsafe {
        let rows = slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        // HardwareInterface is the first bit. Exclude virtual adapters to avoid
        // counting the same traffic again through a VPN or virtual switch.
        for row in rows.iter().filter(|row| {
            row.OperStatus == IfOperStatusUp && row.InterfaceAndOperStatusFlags._bitfield & 1 != 0
        }) {
            adapters.insert(row.InterfaceLuid.Value, [row.InOctets, row.OutOctets]);
        }
        FreeMibTable(table.cast());
    }
    Some(adapters)
}

impl Sampler {
    fn snapshot(&mut self) -> Snapshot {
        let now = Instant::now();
        let elapsed = self.previous.map(|prev| now.duration_since(prev));
        if elapsed.is_some_and(|elapsed| elapsed < Duration::from_millis(850)) {
            return self.cached.clone();
        }
        let processor = processor_time();
        let cpu =
            self.processor
                .zip(processor)
                .and_then(|((old_idle, old_total), (idle, total))| {
                    let total = total.checked_sub(old_total)?;
                    (total > 0).then(|| {
                        100.0 * total.saturating_sub(idle.saturating_sub(old_idle)) as f64
                            / total as f64
                    })
                });
        self.processor = processor;
        let adapters = network_counters();
        self.traffic.rates = self
            .adapters
            .as_ref()
            .zip(adapters.as_ref())
            .zip(elapsed)
            .map(|((old, now), elapsed)| {
                let mut delta = [0u64; 2];
                for (id, counters) in now {
                    if let Some(before) = old.get(id) {
                        for index in 0..2 {
                            delta[index] = delta[index]
                                .saturating_add(counters[index].saturating_sub(before[index]));
                        }
                    }
                }
                self.traffic.received = self.traffic.received.saturating_add(delta[0]);
                self.traffic.sent = self.traffic.sent.saturating_add(delta[1]);
                let rates = delta.map(|bytes| bytes as f64 / elapsed.as_secs_f64());
                if self.traffic.history.len() >= 32 {
                    self.traffic.history.pop_front();
                }
                self.traffic.history.push_back(rates);
                rates
            });
        let network = adapters.as_ref().map(|_| self.traffic.clone());
        self.adapters = adapters;
        if self.previous.is_none() {
            self.gpu = GpuCounter::open();
        }
        self.previous = Some(now);
        self.cached = Snapshot {
            cpu,
            gpu: self.gpu.as_ref().and_then(GpuCounter::read),
            ram: memory(),
            disk: disk(),
            network,
        };
        self.cached.clone()
    }
}

struct GpuCounter {
    // PDH handles are process-wide. Access is serialized by the sampler lock.
    query: usize,
    counter: usize,
}

impl GpuCounter {
    fn open() -> Option<Self> {
        let (mut query, mut counter) = (null_mut(), null_mut());
        unsafe {
            if PdhOpenQueryW(null(), 0, &mut query) != 0 {
                return None;
            }
            if PdhAddEnglishCounterW(
                query,
                w!(r"\GPU Engine(*)\Utilization Percentage").0,
                0,
                &mut counter,
            ) != 0
            {
                PdhCloseQuery(query);
                return None;
            }
        }
        Some(Self {
            query: query as usize,
            counter: counter as usize,
        })
    }

    fn read(&self) -> Option<f64> {
        let counter = self.counter as PDH_HCOUNTER;
        let (mut bytes, mut count) = (0, 0);
        unsafe {
            if PdhCollectQueryData(self.query as PDH_HQUERY) != 0
                || PdhGetFormattedCounterArrayW(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut bytes,
                    &mut count,
                    null_mut(),
                ) != PDH_MORE_DATA
                || bytes > 8 * 1024 * 1024
            {
                return None;
            }
            let unit = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            let mut buffer =
                vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); (bytes as usize).div_ceil(unit)];
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut bytes,
                &mut count,
                buffer.as_mut_ptr(),
            ) != 0
            {
                return None;
            }
            let mut engines = HashMap::<String, f64>::new();
            for item in buffer.iter().take(count as usize) {
                if item.szName.is_null()
                    || !matches!(
                        item.FmtValue.CStatus,
                        PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                    )
                {
                    continue;
                }
                let name = PCWSTR(item.szName).to_string().ok()?;
                let engine = name
                    .split_once("_luid_")
                    .map_or(name.as_str(), |(_, engine)| engine);
                *engines.entry(engine.to_owned()).or_default() +=
                    item.FmtValue.Anonymous.doubleValue;
            }
            engines
                .into_values()
                .filter(|value| value.is_finite())
                .reduce(f64::max)
                .map(|value| value.clamp(0.0, 100.0))
        }
    }
}

impl Drop for GpuCounter {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query as PDH_HQUERY);
        }
    }
}

#[tauri::command]
pub async fn system_snapshot(app: AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Telemetry>()
            .0
            .lock()
            .map_err(|error| error.to_string())
            .map(|mut sampler| sampler.snapshot())
    })
    .await
    .map_err(|error| error.to_string())?
}
