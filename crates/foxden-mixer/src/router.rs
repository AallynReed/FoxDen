//! Takes sound from devices (an input, or what plays on an output) and from single apps, applies
//! each channel's volume, mute, chat mix and EQ, and plays the mix on one output.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::Result;
use crate::capture::{self, AppCapture};
use crate::channel::Channel;
use crate::chatmix;
use crate::dsp::{Band, Equalizer, default_bands};

const MAX_BUFFER_MS: usize = 200;

#[derive(Debug, Clone)]
pub struct MixState {
    pub volume: [f32; 5],
    pub muted: [bool; 5],
    pub master: f32,
    pub master_muted: bool,
    /// -1 all game … 1 all chat.
    pub chat_mix: f32,
    /// Game and chat gains from a headset's ChatMix dial; it replaces `chat_mix` while set.
    pub dial: Option<(f32, f32)>,
    pub eq: [Vec<Band>; 5],
    pub eq_enabled: [bool; 5],
    eq_version: u64,
    /// Peak level per channel since the last read, 0..1.
    pub peaks: [f32; 5],
}

impl Default for MixState {
    fn default() -> Self {
        Self {
            volume: [1.0; 5],
            muted: [false; 5],
            master: 1.0,
            master_muted: false,
            chat_mix: 0.0,
            dial: None,
            eq: std::array::from_fn(|_| default_bands()),
            eq_enabled: [false; 5],
            eq_version: 0,
            peaks: [0.0; 5],
        }
    }
}

impl MixState {
    pub fn set_volume(&mut self, c: Channel, v: f32) {
        self.volume[c.index()] = v.clamp(0.0, 1.0);
    }

    pub fn set_muted(&mut self, c: Channel, m: bool) {
        self.muted[c.index()] = m;
    }

    pub fn peak(&self, c: Channel) -> f32 {
        self.peaks[c.index()]
    }

    pub fn set_eq(&mut self, c: Channel, enabled: bool, bands: Vec<Band>) {
        self.eq[c.index()] = bands;
        self.eq_enabled[c.index()] = enabled;
        self.eq_version += 1;
    }

    fn gains(&self) -> [f32; 5] {
        let mut out = [0.0; 5];
        for c in Channel::ALL {
            out[c.index()] = self.gain(c);
        }
        out
    }

    /// The linear gain one channel gets right now, chat mix included.
    pub fn gain(&self, c: Channel) -> f32 {
        let i = c.index();
        if self.muted[i] {
            return 0.0;
        }
        let (game, chat) = self.dial.unwrap_or_else(|| chatmix::gains(self.chat_mix));
        let mix = match c {
            Channel::Game => game,
            Channel::ChatRender => chat,
            _ => 1.0,
        };
        let master = if c.is_capture() {
            1.0
        } else if self.master_muted {
            0.0
        } else {
            self.master
        };
        self.volume[i] * mix * master
    }
}

pub type Controls = Arc<Mutex<MixState>>;

/// A device feeding a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRoute {
    pub channel: Channel,
    /// Device name as [`list_devices`] gives it.
    pub source: String,
    /// The source is an output device: record what plays on it.
    pub source_is_output: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceList {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
    pub default_output: Option<String>,
}

fn dev_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn list_devices() -> Result<DeviceList> {
    let host = cpal::default_host();
    let mut list = DeviceList {
        default_output: host.default_output_device().and_then(|d| d.name().ok()),
        ..Default::default()
    };
    list.outputs = host
        .output_devices()
        .map_err(dev_err)?
        .filter_map(|d| d.name().ok())
        .collect();
    list.inputs = host
        .input_devices()
        .map_err(dev_err)?
        .filter_map(|d| d.name().ok())
        .collect();
    Ok(list)
}

fn find(host: &cpal::Host, name: &str, output: bool) -> Result<cpal::Device> {
    let mut it: Box<dyn Iterator<Item = cpal::Device>> = if output {
        Box::new(host.output_devices().map_err(dev_err)?)
    } else {
        Box::new(host.input_devices().map_err(dev_err)?)
    };
    it.find(|d| d.name().is_ok_and(|n| n == name))
        .ok_or_else(|| format!("no device named {name}"))
}

type Buffer = Arc<Mutex<VecDeque<f32>>>;
type Lanes = Arc<Mutex<Vec<(Channel, Buffer)>>>;

struct App {
    channel: Channel,
    buffer: Buffer,
    capture: AppCapture,
}

/// Running routes. Dropping it stops every stream. Keep it on the thread that started it.
pub struct Router {
    _streams: Vec<cpal::Stream>,
    lanes: Lanes,
    rate: u32,
    chans: usize,
    cap: usize,
    apps: Vec<App>,
}

impl Router {
    /// Plays on `output`, opened by name so per-app output choices never move it.
    pub fn start(output: &str, routes: &[DeviceRoute], controls: Controls) -> Result<Router> {
        let host = cpal::default_host();
        let out_dev = find(&host, output, true)?;
        let supported = out_dev.default_output_config().map_err(dev_err)?;
        let config: cpal::StreamConfig = supported.config();
        let (rate, chans) = (config.sample_rate.0, config.channels as usize);
        let cap = rate as usize * chans * MAX_BUFFER_MS / 1000;
        let lanes: Lanes = Arc::default();
        let mut streams = Vec::new();
        for r in routes {
            let buffer: Buffer = Arc::new(Mutex::new(VecDeque::with_capacity(cap)));
            streams.push(start_source(&host, r, buffer.clone(), rate, chans, cap)?);
            lanes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((r.channel, buffer));
        }
        let l = lanes.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => output_stream::<f32>(&out_dev, &config, l, controls),
            SampleFormat::I16 => output_stream::<i16>(&out_dev, &config, l, controls),
            SampleFormat::I32 => output_stream::<i32>(&out_dev, &config, l, controls),
            SampleFormat::U16 => output_stream::<u16>(&out_dev, &config, l, controls),
            f => return Err(format!("unsupported output format {f}")),
        }?;
        stream.play().map_err(dev_err)?;
        streams.push(stream);
        Ok(Router {
            _streams: streams,
            lanes,
            rate,
            chans,
            cap,
            apps: Vec::new(),
        })
    }

    /// Starts and stops app captures so that exactly these process trees play, each on its channel.
    pub fn set_apps(&mut self, wanted: &[(Channel, u32)]) {
        let gone: Vec<Buffer> = self
            .apps
            .iter()
            .filter(|a| !wanted.contains(&(a.channel, a.capture.pid)))
            .map(|a| a.buffer.clone())
            .collect();
        if !gone.is_empty() {
            self.lanes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|(_, b)| !gone.iter().any(|g| Arc::ptr_eq(g, b)));
            self.apps
                .retain(|a| wanted.contains(&(a.channel, a.capture.pid)));
        }
        for &(channel, pid) in wanted {
            if self
                .apps
                .iter()
                .any(|a| a.channel == channel && a.capture.pid == pid)
            {
                continue;
            }
            let buffer: Buffer = Arc::new(Mutex::new(VecDeque::with_capacity(self.cap)));
            let mut feed = Feed::new(
                capture::CHANNELS,
                capture::RATE,
                self.chans,
                self.rate,
                buffer.clone(),
                self.cap,
            );
            let capture = AppCapture::start(pid, move |s| feed.push(s.iter().copied()));
            self.lanes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((channel, buffer.clone()));
            self.apps.push(App {
                channel,
                buffer,
                capture,
            });
        }
    }

    /// Captures that failed, by pid.
    pub fn app_errors(&self) -> Vec<(u32, String)> {
        self.apps
            .iter()
            .filter_map(|a| a.capture.error().map(|e| (a.capture.pid, e)))
            .collect()
    }
}

fn output_stream<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    lanes: Lanes,
    ctl: Controls,
) -> Result<cpal::Stream> {
    let rate = config.sample_rate.0 as f32;
    let chans = config.channels as usize;
    let mut sums: [Vec<f32>; 5] = Default::default();
    let mut used = [false; 5];
    let mut eqs: [Equalizer; 5] = Default::default();
    let mut eq_version = u64::MAX;
    let mut gains = [1f32; 5];
    let mut pending_peaks = [0f32; 5];
    let mut scratch: Vec<f32> = Vec::new();
    dev.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            if let Ok(s) = ctl.try_lock() {
                gains = s.gains();
                if eq_version != s.eq_version {
                    for (i, eq) in eqs.iter_mut().enumerate() {
                        *eq = if s.eq_enabled[i] {
                            Equalizer::new(&s.eq[i], rate, chans)
                        } else {
                            Equalizer::default()
                        };
                    }
                    eq_version = s.eq_version;
                }
            }
            for (sum, u) in sums.iter_mut().zip(used.iter_mut()) {
                sum.clear();
                sum.resize(data.len(), 0.0);
                *u = false;
            }
            for (channel, buffer) in lanes.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                let i = channel.index();
                scratch.clear();
                if let Ok(mut b) = buffer.try_lock() {
                    let n = data.len().min(b.len());
                    scratch.extend(b.drain(..n));
                }
                for (m, s) in sums[i].iter_mut().zip(&scratch) {
                    *m += s;
                }
                used[i] = true;
            }
            let mut peaks = pending_peaks;
            for i in 0..5 {
                if !used[i] {
                    continue;
                }
                eqs[i].process(&mut sums[i]);
                let g = gains[i];
                for s in sums[i].iter_mut() {
                    *s *= g;
                    peaks[i] = peaks[i].max(s.abs());
                }
            }
            for (n, d) in data.iter_mut().enumerate() {
                let m: f32 = (0..5).filter(|&i| used[i]).map(|i| sums[i][n]).sum();
                *d = T::from_sample(m.clamp(-1.0, 1.0));
            }
            match ctl.try_lock() {
                Ok(mut s) => {
                    for (p, n) in s.peaks.iter_mut().zip(peaks) {
                        *p = p.max(n);
                    }
                    pending_peaks = [0.0; 5];
                }
                Err(_) => pending_peaks = peaks,
            }
        },
        |e| eprintln!("output stream: {e}"),
        None,
    )
    .map_err(dev_err)
}

fn start_source(
    host: &cpal::Host,
    r: &DeviceRoute,
    buffer: Buffer,
    out_rate: u32,
    out_ch: usize,
    cap: usize,
) -> Result<cpal::Stream> {
    let dev = find(host, &r.source, r.source_is_output)?;
    let supported = if r.source_is_output {
        dev.default_output_config()
    } else {
        dev.default_input_config()
    }
    .map_err(dev_err)?;
    let config: cpal::StreamConfig = supported.config();
    let feed = Feed::new(
        config.channels as usize,
        config.sample_rate.0,
        out_ch,
        out_rate,
        buffer,
        cap,
    );
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input_stream::<f32>(&dev, &config, feed),
        SampleFormat::I16 => input_stream::<i16>(&dev, &config, feed),
        SampleFormat::I32 => input_stream::<i32>(&dev, &config, feed),
        SampleFormat::U16 => input_stream::<u16>(&dev, &config, feed),
        f => return Err(format!("unsupported input format {f}")),
    }?;
    stream.play().map_err(dev_err)?;
    Ok(stream)
}

fn input_stream<T: SizedSample>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    mut feed: Feed,
) -> Result<cpal::Stream>
where
    f32: FromSample<T>,
{
    dev.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            feed.push(data.iter().map(|s| f32::from_sample(*s)))
        },
        |e| eprintln!("input stream: {e}"),
        None,
    )
    .map_err(dev_err)
}

/// Channel mapping and linear resampling into an output lane.
struct Feed {
    in_ch: usize,
    out_ch: usize,
    step: f64,
    t: f64,
    prev: Vec<f32>,
    frame: Vec<f32>,
    have_prev: bool,
    buffer: Buffer,
    cap: usize,
    raw: Vec<f32>,
    pending: Vec<f32>,
}

impl Feed {
    fn new(
        in_ch: usize,
        in_rate: u32,
        out_ch: usize,
        out_rate: u32,
        buffer: Buffer,
        cap: usize,
    ) -> Self {
        Self {
            in_ch: in_ch.max(1),
            out_ch: out_ch.max(1),
            step: in_rate as f64 / out_rate as f64,
            t: 0.0,
            prev: vec![0.0; out_ch.max(1)],
            frame: Vec::with_capacity(out_ch.max(1)),
            have_prev: false,
            buffer,
            cap,
            raw: Vec::new(),
            pending: Vec::new(),
        }
    }

    fn push(&mut self, samples: impl Iterator<Item = f32>) {
        self.raw.clear();
        self.raw.extend(samples);
        self.pending.clear();
        for f in self.raw.chunks_exact(self.in_ch) {
            self.frame.clear();
            for c in 0..self.out_ch {
                self.frame.push(if self.in_ch == 1 {
                    f[0]
                } else {
                    f[c % self.in_ch]
                });
            }
            if !self.have_prev {
                self.prev.clone_from(&self.frame);
                self.have_prev = true;
                continue;
            }
            while self.t < 1.0 {
                let t = self.t as f32;
                for c in 0..self.out_ch {
                    self.pending
                        .push(self.prev[c] + (self.frame[c] - self.prev[c]) * t);
                }
                self.t += self.step;
            }
            self.t -= 1.0;
            self.prev.clone_from(&self.frame);
        }
        if let Ok(mut b) = self.buffer.lock() {
            b.extend(self.pending.iter().copied());
            let over = b.len().saturating_sub(self.cap);
            if over > 0 {
                let over = over - over % self.out_ch;
                b.drain(..over);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_mix_gains() {
        let mut s = MixState {
            chat_mix: 1.0,
            ..Default::default()
        };
        assert!(s.gain(Channel::Game) < 1e-6);
        assert!((s.gain(Channel::ChatRender) - 1.0).abs() < 1e-6);
        assert_eq!(s.gain(Channel::Media), 1.0);
        s.set_muted(Channel::Media, true);
        assert_eq!(s.gain(Channel::Media), 0.0);
        s.master = 0.5;
        assert!((s.gain(Channel::ChatRender) - 0.5).abs() < 1e-6);
        assert_eq!(s.gain(Channel::ChatCapture), 1.0);
    }

    #[test]
    fn resampler_doubles_rate() {
        let buf: Buffer = Arc::default();
        let mut f = Feed::new(1, 24000, 2, 48000, buf.clone(), 10_000);
        f.push([0.0f32, 1.0, 0.0, 1.0, 0.0].into_iter());
        assert_eq!(buf.lock().unwrap().len(), 4 * 2 * 2);
    }
}
