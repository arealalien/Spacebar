use super::{endpoint_id, enumerator, Com};
use std::{
    f32::consts::TAU,
    ptr,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{State, WebviewWindow};
use windows::{
    core::{Error, Result as WindowsResult, GUID},
    Win32::{
        Foundation::E_INVALIDARG,
        Media::Audio::{
            eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IAudioCaptureClient,
            IAudioClient, IMMDevice, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY,
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
        },
        System::Com::{CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED},
    },
};

const BAND_COUNT: usize = 16;
const FFT_SIZE: usize = 4096;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(30);
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_millis(700);
const SILENCE_AFTER: Duration = Duration::from_millis(90);
const IDLE_TIMEOUT: Duration = Duration::from_secs(1);

type Bands = [f32; BAND_COUNT];
const SILENCE: Bands = [0.0; BAND_COUNT];

#[derive(Default)]
pub struct AudioSpectrum(Arc<Mutex<CaptureState>>);

#[derive(Default)]
struct CaptureState {
    last_request: Option<Instant>,
    latest: Option<Bands>,
    running: bool,
}

#[tauri::command]
pub fn audio_output_spectrum(
    webview: WebviewWindow,
    spectrum: State<'_, AudioSpectrum>,
) -> Result<Option<Bands>, String> {
    if !matches!(webview.label(), "dock" | "folder") {
        return Err("The audio visualizer is unavailable here.".into());
    }
    let mut state = spectrum.0.lock().map_err(|error| error.to_string())?;
    state.last_request = Some(Instant::now());
    if !state.running {
        let shared = Arc::clone(&spectrum.0);
        thread::Builder::new()
            .name("audio-spectrum".into())
            .spawn(move || capture(shared))
            .map_err(|error| error.to_string())?;
        state.running = true;
    }
    Ok(state.latest)
}

fn capture(shared: Arc<Mutex<CaptureState>>) {
    let apartment = Com::enter(COINIT_MULTITHREADED).ok();
    let mut devices = None;
    let mut stream: Option<Loopback> = None;
    let mut next_device_check = Instant::now();
    loop {
        {
            let Ok(mut state) = shared.lock() else { return };
            if state
                .last_request
                .is_none_or(|time| time.elapsed() >= IDLE_TIMEOUT)
            {
                state.running = false;
                state.latest = None;
                return;
            }
        }
        let now = Instant::now();
        if now >= next_device_check {
            next_device_check = now + DEVICE_CHECK_INTERVAL;
            if devices.is_none() && apartment.is_some() {
                devices = enumerator().ok();
            }
            let output = devices.as_ref().and_then(|devices| unsafe {
                devices.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()
            });
            let output = output.and_then(|device| endpoint_id(&device).ok().map(|id| (device, id)));
            match output {
                Some((device, id)) if stream.as_ref().is_none_or(|loopback| loopback.id != id) => {
                    drop(stream.take());
                    stream = Loopback::open(&device, id).ok();
                }
                None => stream = None,
                _ => {}
            }
        }
        let latest = match stream.as_mut().map(Loopback::read) {
            Some(Ok(levels)) => Some(levels),
            Some(Err(_)) => {
                stream = None;
                None
            }
            None => None,
        };
        let Ok(mut state) = shared.lock() else { return };
        state.latest = latest;
        drop(state);
        thread::sleep(SAMPLE_INTERVAL.saturating_sub(now.elapsed()));
    }
}

struct Loopback {
    id: String,
    client: IAudioClient,
    input: IAudioCaptureClient,
    volume: Option<IAudioEndpointVolume>,
    format: SampleFormat,
    analyzer: Analyzer,
    last_packet: Instant,
    levels: Bands,
}

impl Loopback {
    fn open(device: &IMMDevice, id: String) -> WindowsResult<Self> {
        unsafe {
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let mix = client.GetMixFormat()?;
            let format = SampleFormat::parse(mix).and_then(|format| {
                // Buffer enough audio for several polls, then drain every available packet.
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    1_000_000,
                    0,
                    mix,
                    None,
                )?;
                Ok(format)
            });
            CoTaskMemFree(Some(mix.cast()));
            let format = format?;
            let input = client.GetService()?;
            let volume = device.Activate(CLSCTX_ALL, None).ok();
            client.Start()?;
            Ok(Self {
                id,
                client,
                input,
                volume,
                format,
                analyzer: Analyzer::new(format.sample_rate, format.channels.min(2)),
                last_packet: Instant::now(),
                levels: SILENCE,
            })
        }
    }

    fn read(&mut self) -> WindowsResult<Bands> {
        let muted = self
            .volume
            .as_ref()
            .is_some_and(|volume| unsafe { volume.GetMute().is_ok_and(|muted| muted.as_bool()) });
        let mut received = false;
        unsafe {
            while self.input.GetNextPacketSize()? != 0 {
                let mut data = ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                self.input
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    self.analyzer.clear();
                }
                if muted || data.is_null() || flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    for _ in 0..(frames as usize).min(FFT_SIZE) {
                        self.analyzer.push([0.0; 2]);
                    }
                } else {
                    let bytes = std::slice::from_raw_parts(
                        data,
                        frames as usize * self.format.block_size(),
                    );
                    self.format.feed(bytes, &mut self.analyzer);
                }
                self.input.ReleaseBuffer(frames)?;
                if frames == 0 {
                    break;
                }
                received = true;
            }
        }
        if received {
            self.last_packet = Instant::now();
        }
        if muted || self.last_packet.elapsed() >= SILENCE_AFTER {
            self.analyzer.clear();
            self.levels = SILENCE;
        } else if received {
            self.levels = self.analyzer.bands();
        }
        Ok(self.levels)
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

#[derive(Clone, Copy)]
enum Encoding {
    Integer,
    Float,
}

#[derive(Clone, Copy)]
struct SampleFormat {
    encoding: Encoding,
    bytes: usize,
    channels: usize,
    sample_rate: u32,
}

impl SampleFormat {
    unsafe fn parse(mix: *const WAVEFORMATEX) -> WindowsResult<Self> {
        const PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
        const FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        let invalid = || Error::new(E_INVALIDARG, "Unsupported Windows audio format.");
        if mix.is_null() {
            return Err(invalid());
        }
        let format = *mix;
        let subtype = match format.wFormatTag {
            1 => PCM,
            3 => FLOAT,
            0xfffe if format.cbSize >= 22 => (*mix.cast::<WAVEFORMATEXTENSIBLE>()).SubFormat,
            _ => return Err(invalid()),
        };
        let encoding = match (subtype, format.wBitsPerSample) {
            (PCM, 8 | 16 | 24 | 32) => Encoding::Integer,
            (FLOAT, 32 | 64) => Encoding::Float,
            _ => return Err(invalid()),
        };
        let result = Self {
            encoding,
            bytes: usize::from(format.wBitsPerSample / 8),
            channels: usize::from(format.nChannels),
            sample_rate: format.nSamplesPerSec,
        };
        if result.channels == 0
            || result.sample_rate == 0
            || result.block_size() != usize::from(format.nBlockAlign)
        {
            return Err(invalid());
        }
        Ok(result)
    }

    fn block_size(self) -> usize {
        self.channels * self.bytes
    }

    fn feed(self, packet: &[u8], analyzer: &mut Analyzer) {
        let left_scale = 1.0 / self.channels.div_ceil(2) as f32;
        let right_scale = 1.0 / (self.channels / 2).max(1) as f32;
        for frame in packet.chunks_exact(self.block_size()) {
            // Keep the left and right groups separate so phase differences do not cancel stereo audio.
            let mut channels = [0.0; 2];
            for (index, bytes) in frame.chunks_exact(self.bytes).enumerate() {
                let sample = match (self.encoding, bytes) {
                    (Encoding::Integer, [a]) => (f32::from(*a) - 128.0) / 128.0,
                    (Encoding::Integer, [a, b]) => {
                        f32::from(i16::from_le_bytes([*a, *b])) / 32_768.0
                    }
                    (Encoding::Integer, [a, b, c]) => {
                        i32::from_le_bytes([0, *a, *b, *c]) as f32 / 2_147_483_648.0
                    }
                    (Encoding::Integer, [a, b, c, d]) => {
                        i32::from_le_bytes([*a, *b, *c, *d]) as f32 / 2_147_483_648.0
                    }
                    (Encoding::Float, [a, b, c, d]) => f32::from_le_bytes([*a, *b, *c, *d]),
                    (Encoding::Float, [a, b, c, d, e, f, g, h]) => {
                        f64::from_le_bytes([*a, *b, *c, *d, *e, *f, *g, *h]) as f32
                    }
                    _ => 0.0,
                };
                if sample.is_finite() {
                    channels[index % 2] += sample.clamp(-1.0, 1.0);
                }
            }
            analyzer.push([channels[0] * left_scale, channels[1] * right_scale]);
        }
    }
}

struct Analyzer {
    channels: usize,
    samples: Vec<[f32; 2]>,
    next: usize,
    window: Vec<f32>,
    rotations: Vec<(f32, f32)>,
    bins: Vec<(f32, f32)>,
    edges: [usize; BAND_COUNT + 1],
}

impl Analyzer {
    fn new(rate: u32, channels: usize) -> Self {
        let high = 12_000.0_f32.min(rate as f32 / 2.0);
        let edges = std::array::from_fn(|i| {
            let hz = 45.0 * (high / 45.0).powf(i as f32 / BAND_COUNT as f32);
            ((hz * FFT_SIZE as f32 / rate as f32).ceil() as usize).clamp(1, FFT_SIZE / 2)
        });
        Self {
            channels,
            samples: vec![[0.0; 2]; FFT_SIZE],
            next: 0,
            window: (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_SIZE as f32).cos())
                .collect(),
            rotations: (0..FFT_SIZE / 2)
                .map(|i| {
                    let (sin, cos) = (-TAU * i as f32 / FFT_SIZE as f32).sin_cos();
                    (cos, sin)
                })
                .collect(),
            bins: vec![(0.0, 0.0); FFT_SIZE],
            edges,
        }
    }

    fn push(&mut self, sample: [f32; 2]) {
        self.samples[self.next] = sample;
        self.next = (self.next + 1) % FFT_SIZE;
    }

    fn clear(&mut self) {
        self.samples.fill([0.0; 2]);
    }

    fn bands(&mut self) -> Bands {
        let mut levels = SILENCE;
        if self.samples.iter().all(|frame| *frame == [0.0; 2]) {
            return levels;
        }
        for channel in 0..self.channels {
            // Apply a Hann window and load samples in bit-reversed order for the radix-2 FFT.
            for (i, weight) in self.window.iter().enumerate() {
                let index = i.reverse_bits() >> (usize::BITS - FFT_SIZE.trailing_zeros());
                self.bins[index] = (
                    self.samples[(self.next + i) % FFT_SIZE][channel] * weight,
                    0.0,
                );
            }
            self.transform();
            for (level, edge) in levels.iter_mut().zip(self.edges.windows(2)) {
                let end = edge[1].max(edge[0] + 1).min(FFT_SIZE / 2);
                let power = self.bins[edge[0]..end]
                    .iter()
                    .map(|(real, imaginary)| real * real + imaginary * imaginary)
                    .fold(0.0_f32, f32::max);
                *level = level.max(power.sqrt() * (4.0 / FFT_SIZE as f32));
            }
        }
        levels
    }

    fn transform(&mut self) {
        for stage in 1..=FFT_SIZE.trailing_zeros() {
            let size = 1 << stage;
            for block in self.bins.chunks_exact_mut(size) {
                let (left, right) = block.split_at_mut(size / 2);
                for (i, (even, odd)) in left.iter_mut().zip(right).enumerate() {
                    let (cos, sin) = self.rotations[i * FFT_SIZE / size];
                    let real = odd.0 * cos - odd.1 * sin;
                    let imaginary = odd.0 * sin + odd.1 * cos;
                    *odd = (even.0 - real, even.1 - imaginary);
                    *even = (even.0 + real, even.1 + imaginary);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equally_loud_tones_move_different_bars() {
        let format = SampleFormat {
            encoding: Encoding::Float,
            bytes: 4,
            channels: 2,
            sample_rate: 48_000,
        };
        let mut analyzer = Analyzer::new(format.sample_rate, format.channels);
        let peak = |analyzer: &mut Analyzer, hz: f32| {
            let mut packet = Vec::with_capacity(FFT_SIZE * format.block_size());
            for i in 0..FFT_SIZE {
                let amplitude = 0.28 * (TAU * hz * i as f32 / 48_000.0).sin();
                packet.extend_from_slice(&amplitude.to_le_bytes());
                packet.extend_from_slice(&(-amplitude).to_le_bytes());
            }
            format.feed(&packet, analyzer);
            let levels = analyzer.bands();
            let (band, level) = levels
                .into_iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(level > 0.22);
            band
        };
        assert!(peak(&mut analyzer, 105.0) < 4);
        assert!(peak(&mut analyzer, 3_200.0) > 10);
        analyzer.clear();
        assert_eq!(analyzer.bands(), SILENCE);
    }
}
