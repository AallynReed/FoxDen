use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream};
use foxrgb_core::{AUDIO_BANDS, AudioFrame};

const MIN_HZ: f32 = 40.0;
const MAX_HZ: f32 = 16_000.0;
const RING_LEN: usize = 16_384;
const TICK: Duration = Duration::from_millis(10);
const STALE: Duration = Duration::from_millis(60);
const RECHECK: Duration = Duration::from_secs(1);

/// Loudness span below the gain ceiling that maps to 0..=1.
const RANGE_DB: f32 = 36.0;
const MIN_CEILING_DB: f32 = -40.0;
const MAX_AMPLITUDE: f32 = 10.0;
const CEILING_FALL_DB: f32 = 3.0;
const ATTACK_SECS: f32 = 0.02;
const FALL_PER_SEC: f32 = 2.5;

/// Captures what the default output device is playing and analyses it.
pub struct AudioCapture {
    shared: Arc<Shared>,
    stop: Sender<()>,
    worker: Option<JoinHandle<()>>,
}

struct Shared {
    ring: Mutex<Ring>,
    frame: Mutex<AudioFrame>,
    failed: AtomicBool,
}

/// Newest mono samples, always `RING_LEN` long.
struct Ring {
    samples: VecDeque<f32>,
    rate: u32,
    fresh: bool,
}

impl AudioCapture {
    /// Starts loopback capture on a background thread. Capture stops when the value is dropped.
    pub fn start() -> Result<Self, String> {
        let shared = Arc::new(Shared {
            ring: Mutex::new(Ring { samples: vec![0.0; RING_LEN].into(), rate: 48_000, fresh: false }),
            frame: Mutex::default(),
            failed: AtomicBool::new(false),
        });
        let (stop, stopped) = mpsc::channel();
        let (ready, opened) = mpsc::channel();
        let worker = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("foxrgb-audio".into())
                .spawn(move || run(&shared, &stopped, &ready))
                .map_err(|e| e.to_string())?
        };
        opened.recv().map_err(|e| e.to_string())??;
        Ok(Self { shared, stop, worker: Some(worker) })
    }

    /// Latest analysis. Cheap; called once per rendered frame.
    pub fn frame(&self) -> AudioFrame {
        *self.shared.frame.lock().unwrap()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Owns the stream and refreshes the shared frame until told to stop.
fn run(shared: &Arc<Shared>, stopped: &Receiver<()>, ready: &Sender<Result<(), String>>) {
    let mut input = match open(shared) {
        Ok(input) => Some(input),
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut meter = Meter::new();
    let mut window = Vec::new();
    let mut last = Instant::now();
    let mut heard = last;
    let mut checked = last;
    while stopped.recv_timeout(TICK) == Err(RecvTimeoutError::Timeout) {
        let now = Instant::now();
        if shared.failed.swap(false, Ordering::Relaxed) {
            input = None;
        }
        if now - checked > RECHECK {
            checked = now;
            // follow the default device when it changes or comes back
            if input.as_ref().is_none_or(|(_, device)| *device != default_name()) {
                drop(input.take());
                // an error from the stream just closed must not close the new one
                shared.failed.store(false, Ordering::Relaxed);
                input = open(shared).ok();
            }
        }

        let rate = {
            let mut ring = shared.ring.lock().unwrap();
            if std::mem::take(&mut ring.fresh) {
                heard = now;
            } else if now - heard > STALE {
                // loopback delivers nothing while nothing plays
                ring.samples.iter_mut().for_each(|s| *s = 0.0);
            }
            window.clear();
            window.extend(ring.samples.range(RING_LEN - window_len(ring.rate)..));
            ring.rate
        };
        let frame = meter.update(&spectrum(&window, rate), (now - last).as_secs_f32());
        last = now;
        *shared.frame.lock().unwrap() = frame;
    }
}

fn default_name() -> Option<String> {
    cpal::default_host().default_output_device()?.name().ok()
}

/// Starts a loopback stream on the default output device and returns it with the device's name.
fn open(shared: &Arc<Shared>) -> Result<(Stream, Option<String>), String> {
    let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config = config.config();
    shared.ring.lock().unwrap().rate = config.sample_rate.0;
    let stream = match format {
        SampleFormat::F32 => listen::<f32>(&device, &config, shared),
        SampleFormat::F64 => listen::<f64>(&device, &config, shared),
        SampleFormat::I8 => listen::<i8>(&device, &config, shared),
        SampleFormat::I16 => listen::<i16>(&device, &config, shared),
        SampleFormat::I32 => listen::<i32>(&device, &config, shared),
        SampleFormat::I64 => listen::<i64>(&device, &config, shared),
        SampleFormat::U8 => listen::<u8>(&device, &config, shared),
        SampleFormat::U16 => listen::<u16>(&device, &config, shared),
        SampleFormat::U32 => listen::<u32>(&device, &config, shared),
        SampleFormat::U64 => listen::<u64>(&device, &config, shared),
        other => Err(format!("unsupported sample format {other:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, device.name().ok()))
}

/// An input stream on an output device is a loopback of what it plays.
fn listen<T>(device: &cpal::Device, config: &cpal::StreamConfig, shared: &Arc<Shared>) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels).max(1);
    // stereo material on a surround device fills two channels, so never divide by more
    let divisor = channels.min(2) as f32;
    let (data, error) = (shared.clone(), shared.clone());
    device
        .build_input_stream(
            config,
            move |input: &[T], _: &cpal::InputCallbackInfo| {
                let mut ring = data.ring.lock().unwrap();
                for frame in input.chunks_exact(channels) {
                    let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
                    ring.samples.push_back(sum / divisor);
                }
                let extra = ring.samples.len() - RING_LEN;
                ring.samples.drain(..extra);
                ring.fresh = true;
            },
            move |_| error.failed.store(true, Ordering::Relaxed),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Band edges in Hz, log spaced: band `i` covers `edges[i]..edges[i + 1]`.
pub fn band_edges() -> [f32; AUDIO_BANDS + 1] {
    std::array::from_fn(|i| MIN_HZ * (MAX_HZ / MIN_HZ).powf(i as f32 / AUDIO_BANDS as f32))
}

/// Samples analysed at a sample rate: a power of two that keeps bins under 24 Hz wide.
fn window_len(rate: u32) -> usize {
    (rate as usize / 24).next_power_of_two().clamp(256, RING_LEN)
}

/// Amplitude per band of the newest samples; a full-scale sine reads about 1 in its band.
pub fn spectrum(samples: &[f32], rate: u32) -> [f32; AUDIO_BANDS] {
    let n = window_len(rate);
    let newest = &samples[samples.len().saturating_sub(n)..];
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    for (i, sample) in newest.iter().enumerate() {
        re[i] = sample * (0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos());
    }
    fft(&mut re, &mut im);

    let bin_hz = rate as f32 / n as f32;
    let edges = band_edges();
    std::array::from_fn(|band| {
        let start = (edges[band] / bin_hz).ceil() as usize;
        let end = ((edges[band + 1] / bin_hz).ceil() as usize).max(start.saturating_add(1)).min(n / 2);
        let power: f32 = (start..end).map(|k| re[k] * re[k] + im[k] * im[k]).sum();
        // 4 / n undoes the Hann window's gain, 1.5 is its noise bandwidth in bins
        (power / 1.5).sqrt() * 4.0 / n as f32
    })
}

/// In-place radix-2 FFT. The length must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = -2.0 * PI / len as f32;
        for k in 0..len / 2 {
            let (sin, cos) = (step * k as f32).sin_cos();
            for a in (k..n).step_by(len) {
                let b = a + len / 2;
                let (tr, ti) = (re[b] * cos - im[b] * sin, re[b] * sin + im[b] * cos);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len *= 2;
    }
}

/// Turns raw band amplitudes into the smoothed 0..=1 frame effects see.
struct Meter {
    frame: AudioFrame,
    band_ceiling: f32,
    level_ceiling: f32,
}

impl Meter {
    fn new() -> Self {
        Self { frame: AudioFrame::default(), band_ceiling: MIN_CEILING_DB, level_ceiling: MIN_CEILING_DB }
    }

    fn update(&mut self, raw: &[f32; AUDIO_BANDS], dt: f32) -> AudioFrame {
        let db = raw.map(to_db);
        let level_db = to_db(raw.iter().map(|b| b * b).sum::<f32>().sqrt());
        let peak_db = db.iter().copied().fold(f32::MIN, f32::max);
        self.band_ceiling = ceiling(self.band_ceiling, peak_db, dt);
        self.level_ceiling = ceiling(self.level_ceiling, level_db, dt);
        for (value, db) in self.frame.bands.iter_mut().zip(db) {
            *value = smooth(*value, scale(db, self.band_ceiling), dt);
        }
        self.frame.level = smooth(self.frame.level, scale(level_db, self.level_ceiling), dt);
        self.frame
    }
}

fn to_db(amplitude: f32) -> f32 {
    // broken samples give NaN or infinity: NaN counts as silence, the rest is capped
    let amplitude = if amplitude.is_nan() { 0.0 } else { amplitude };
    20.0 * amplitude.clamp(1e-9, MAX_AMPLITUDE).log10()
}

/// Slow automatic gain: the ceiling jumps to a new peak and sinks back slowly.
fn ceiling(old: f32, peak_db: f32, dt: f32) -> f32 {
    (old - CEILING_FALL_DB * dt).max(peak_db).max(MIN_CEILING_DB)
}

fn scale(db: f32, ceiling: f32) -> f32 {
    ((db - ceiling) / RANGE_DB + 1.0).clamp(0.0, 1.0)
}

/// Fast attack, steady fall.
fn smooth(old: f32, target: f32, dt: f32) -> f32 {
    if target > old {
        old + (target - old) * (1.0 - (-dt / ATTACK_SECS).exp())
    } else {
        (old - FALL_PER_SEC * dt).max(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32) -> Vec<f32> {
        (0..window_len(rate)).map(|i| (2.0 * PI * hz * i as f32 / rate as f32).sin()).collect()
    }

    #[test]
    fn silence_is_zero() {
        assert_eq!(spectrum(&[0.0; 4096], 48_000), [0.0; AUDIO_BANDS]);
        assert_eq!(spectrum(&[], 48_000), [0.0; AUDIO_BANDS]);
        assert_eq!(spectrum(&[1.0; 64], 0), [0.0; AUDIO_BANDS]);
        assert_eq!(Meter::new().update(&[0.0; AUDIO_BANDS], 0.01), AudioFrame::default());
    }

    #[test]
    fn sine_peaks_in_its_band() {
        let edges = band_edges();
        for rate in [44_100, 48_000, 96_000] {
            for band in 0..AUDIO_BANDS {
                let hz = (edges[band] * edges[band + 1]).sqrt();
                let bands = spectrum(&sine(hz, rate), rate);
                let loudest = (0..AUDIO_BANDS).max_by(|&a, &b| bands[a].total_cmp(&bands[b]));
                assert_eq!(loudest, Some(band), "{hz} Hz sampled at {rate} Hz: {bands:?}");
            }
        }
        let full_scale = spectrum(&sine(1000.0, 48_000), 48_000)[8];
        assert!((full_scale - 1.0).abs() < 0.05, "{full_scale}");
    }

    #[test]
    fn edges_cover_the_range() {
        let edges = band_edges();
        assert!(edges.windows(2).all(|pair| pair[0] < pair[1]));
        assert!((edges[0] - MIN_HZ).abs() < 0.01);
        assert!((edges[AUDIO_BANDS] - MAX_HZ).abs() < 1.0);
    }

    #[test]
    fn meter_follows_then_decays() {
        let mut meter = Meter::new();
        let mut raw = [0.0; AUDIO_BANDS];
        raw[3] = 0.5;
        let mut frame = AudioFrame::default();
        for _ in 0..50 {
            frame = meter.update(&raw, 0.01);
        }
        assert!(frame.bands[3] > 0.95 && frame.level > 0.95, "{frame:?}");
        assert_eq!(frame.bands[4], 0.0);
        for _ in 0..100 {
            frame = meter.update(&[0.0; AUDIO_BANDS], 0.01);
        }
        assert_eq!(frame, AudioFrame::default());
    }

    #[test]
    fn meter_survives_broken_samples() {
        let in_range = |frame: AudioFrame| {
            frame.bands.iter().chain([&frame.level]).all(|v| (0.0..=1.0).contains(v))
        };
        let mut meter = Meter::new();
        for bad in [f32::INFINITY, f32::NAN, 1e30, -1.0] {
            for _ in 0..50 {
                let frame = meter.update(&[bad; AUDIO_BANDS], 0.01);
                assert!(in_range(frame), "{bad}: {frame:?}");
            }
        }
        let frame = meter.update(&spectrum(&[f32::INFINITY; 2048], 48_000), 0.01);
        assert!(in_range(frame), "{frame:?}");

        // the gain ceiling has to come back down, or the meter stays dark for good
        let mut raw = [0.0; AUDIO_BANDS];
        raw[3] = 0.5;
        let mut frame = AudioFrame::default();
        for _ in 0..2000 {
            frame = meter.update(&raw, 0.01);
        }
        assert!(frame.bands[3] > 0.95 && frame.level > 0.95, "{frame:?}");
    }
}
