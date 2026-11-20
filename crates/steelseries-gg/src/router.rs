//! Driver-free fallback for Sonar's mixer: takes audio from any device (an input, or an output
//! by loopback), applies Sonar's per-channel volume, mute, chat mix and EQ, and plays it on an
//! output. It needs no SteelSeries software; with Sonar's virtual devices installed it can stand
//! in for the Sonar app, otherwise route from any virtual cable or the apps' own output.

use crate::chatmix;
use crate::dsp::{Band, Equalizer, default_bands};
use crate::error::{Error, Result};
use crate::sonar::Channel;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

const MAX_BUFFER_MS: usize = 200;

fn index(c: Channel) -> usize {
    match c {
        Channel::Game => 0,
        Channel::ChatRender => 1,
        Channel::Media => 2,
        Channel::Aux => 3,
        Channel::ChatCapture => 4,
    }
}

#[derive(Debug, Clone)]
pub struct MixState {
    pub volume: [f32; 5],
    pub muted: [bool; 5],
    pub master: f32,
    pub master_muted: bool,
    /// -1 all game … 1 all chat, Sonar's curve.
    pub chat_mix: f32,
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
            eq: std::array::from_fn(|_| default_bands()),
            eq_enabled: [false; 5],
            eq_version: 0,
            peaks: [0.0; 5],
        }
    }
}

impl MixState {
    pub fn volume(&self, c: Channel) -> f32 {
        self.volume[index(c)]
    }
    pub fn set_volume(&mut self, c: Channel, v: f32) {
        self.volume[index(c)] = v.clamp(0.0, 1.0);
    }
    pub fn muted(&self, c: Channel) -> bool {
        self.muted[index(c)]
    }
    pub fn set_muted(&mut self, c: Channel, m: bool) {
        self.muted[index(c)] = m;
    }
    pub fn peak(&self, c: Channel) -> f32 {
        self.peaks[index(c)]
    }
    pub fn bands(&self, c: Channel) -> &[Band] {
        &self.eq[index(c)]
    }
    pub fn eq_enabled(&self, c: Channel) -> bool {
        self.eq_enabled[index(c)]
    }
    pub fn set_eq(&mut self, c: Channel, enabled: bool, bands: Vec<Band>) {
        self.eq[index(c)] = bands;
        self.eq_enabled[index(c)] = enabled;
        self.eq_version += 1;
    }

    pub fn eq_version(&self) -> u64 {
        self.eq_version
    }

    pub fn gains(&self) -> [f32; 5] {
        [Channel::Game, Channel::ChatRender, Channel::Media, Channel::Aux, Channel::ChatCapture].map(|c| self.gain(c))
    }

    /// The linear gain one channel gets right now, chat mix included.
    pub fn gain(&self, c: Channel) -> f32 {
        let i = index(c);
        if self.muted[i] {
            return 0.0;
        }
        let (game, chat) = chatmix::gains(self.chat_mix);
        let mix = match c {
            Channel::Game => game,
            Channel::ChatRender => chat,
            _ => 1.0,
        };
        let master = if c.is_capture() { 1.0 } else if self.master_muted { 0.0 } else { self.master };
        self.volume[i] * mix * master
    }
}

pub type Controls = Arc<Mutex<MixState>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSpec {
    pub channel: Channel,
    /// Device name as [`list_devices`] gives it.
    pub source: String,
    /// The source is an output device: record what plays on it (WASAPI loopback).
    pub source_is_output: bool,
    pub output: String,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceList {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

pub fn list_devices() -> Result<DeviceList> {
    let host = cpal::default_host();
    let mut list = DeviceList {
        default_output: host.default_output_device().and_then(|d| d.name().ok()),
        default_input: host.default_input_device().and_then(|d| d.name().ok()),
        ..Default::default()
    };
    for d in host.output_devices().map_err(dev_err)? {
        if let Ok(n) = d.name() {
            list.outputs.push(n);
        }
    }
    for d in host.input_devices().map_err(dev_err)? {
        if let Ok(n) = d.name() {
            list.inputs.push(n);
        }
    }
    Ok(list)
}

fn dev_err(e: impl std::fmt::Display) -> Error {
    Error::Device(e.to_string())
}

fn find(host: &cpal::Host, name: &str, output: bool) -> Result<cpal::Device> {
    let mut it: Box<dyn Iterator<Item = cpal::Device>> = if output {
        Box::new(host.output_devices().map_err(dev_err)?)
    } else {
        Box::new(host.input_devices().map_err(dev_err)?)
    };
    it.find(|d| d.name().map(|n| n == name).unwrap_or(false)).ok_or_else(|| Error::Device(format!("no device named {name}")))
}

type Buffer = Arc<Mutex<VecDeque<f32>>>;

struct Lane {
    channel: Channel,
    buffer: Buffer,
    eq: Equalizer,
    eq_version: u64,
    scratch: Vec<f32>,
}

/// Running routes. Dropping it stops every stream. Keep it on the thread that started it.
pub struct Router {
    _streams: Vec<cpal::Stream>,
    pub controls: Controls,
}

impl Router {
    pub fn start(specs: &[RouteSpec], controls: Controls) -> Result<Router> {
        let host = cpal::default_host();
        let mut by_output: BTreeMap<&str, Vec<&RouteSpec>> = BTreeMap::new();
        for s in specs {
            by_output.entry(s.output.as_str()).or_default().push(s);
        }
        let mut streams = Vec::new();
        for (out_name, routes) in by_output {
            let out_dev = find(&host, out_name, true)?;
            let supported = out_dev.default_output_config().map_err(dev_err)?;
            let config: cpal::StreamConfig = supported.config();
            let (rate, chans) = (config.sample_rate.0, config.channels as usize);
            let cap = rate as usize * chans * MAX_BUFFER_MS / 1000;
            let mut lanes = Vec::new();
            for r in routes {
                let buffer: Buffer = Arc::new(Mutex::new(VecDeque::with_capacity(cap)));
                streams.push(start_source(&host, r, buffer.clone(), rate, chans, cap)?);
                lanes.push(Lane { channel: r.channel, buffer, eq: Equalizer::default(), eq_version: u64::MAX, scratch: Vec::new() });
            }
            let ctl = controls.clone();
            let stream = match supported.sample_format() {
                SampleFormat::F32 => output_stream::<f32>(&out_dev, &config, lanes, ctl),
                SampleFormat::I16 => output_stream::<i16>(&out_dev, &config, lanes, ctl),
                SampleFormat::I32 => output_stream::<i32>(&out_dev, &config, lanes, ctl),
                SampleFormat::U16 => output_stream::<u16>(&out_dev, &config, lanes, ctl),
                f => return Err(Error::Device(format!("unsupported output format {f}"))),
            }?;
            stream.play().map_err(dev_err)?;
            streams.push(stream);
        }
        Ok(Router { _streams: streams, controls })
    }
}

fn output_stream<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    config: &cpal::StreamConfig,
    mut lanes: Vec<Lane>,
    ctl: Controls,
) -> Result<cpal::Stream> {
    let rate = config.sample_rate.0 as f32;
    let chans = config.channels as usize;
    let mut mix: Vec<f32> = Vec::new();
    let mut gains = [1f32; 5];
    let mut pending_peaks = [0f32; 5];
    dev.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            mix.clear();
            mix.resize(data.len(), 0.0);
            if let Ok(s) = ctl.try_lock() {
                gains = s.gains();
                for lane in lanes.iter_mut() {
                    if lane.eq_version != s.eq_version {
                        let i = index(lane.channel);
                        lane.eq = if s.eq_enabled[i] { Equalizer::new(&s.eq[i], rate, chans) } else { Equalizer::default() };
                        lane.eq_version = s.eq_version;
                    }
                }
            }
            let mut peaks = pending_peaks;
            for lane in lanes.iter_mut() {
                let i = index(lane.channel);
                lane.scratch.clear();
                if let Ok(mut b) = lane.buffer.try_lock() {
                    let n = data.len().min(b.len());
                    lane.scratch.extend(b.drain(..n));
                }
                lane.scratch.resize(data.len(), 0.0);
                lane.eq.process(&mut lane.scratch);
                let g = gains[i];
                for (m, s) in mix.iter_mut().zip(&lane.scratch) {
                    let v = s * g;
                    peaks[i] = peaks[i].max(v.abs());
                    *m += v;
                }
            }
            for (d, m) in data.iter_mut().zip(&mix) {
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

fn start_source(host: &cpal::Host, r: &RouteSpec, buffer: Buffer, out_rate: u32, out_ch: usize, cap: usize) -> Result<cpal::Stream> {
    let dev = find(host, &r.source, r.source_is_output)?;
    let supported = if r.source_is_output { dev.default_output_config() } else { dev.default_input_config() }.map_err(dev_err)?;
    let config: cpal::StreamConfig = supported.config();
    let feed = Feed::new(config.channels as usize, config.sample_rate.0, out_ch, out_rate, buffer, cap);
    let stream = match supported.sample_format() {
        SampleFormat::F32 => input_stream::<f32>(&dev, &config, feed),
        SampleFormat::I16 => input_stream::<i16>(&dev, &config, feed),
        SampleFormat::I32 => input_stream::<i32>(&dev, &config, feed),
        SampleFormat::U16 => input_stream::<u16>(&dev, &config, feed),
        f => return Err(Error::Device(format!("unsupported input format {f}"))),
    }?;
    stream.play().map_err(dev_err)?;
    Ok(stream)
}

fn input_stream<T: SizedSample>(dev: &cpal::Device, config: &cpal::StreamConfig, mut feed: Feed) -> Result<cpal::Stream>
where
    f32: FromSample<T>,
{
    dev.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| feed.push(data.iter().map(|s| f32::from_sample(*s))),
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
    pending: Vec<f32>,
}

impl Feed {
    fn new(in_ch: usize, in_rate: u32, out_ch: usize, out_rate: u32, buffer: Buffer, cap: usize) -> Self {
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
            pending: Vec::new(),
        }
    }

    fn push(&mut self, samples: impl Iterator<Item = f32>) {
        let mut raw: Vec<f32> = Vec::new();
        raw.extend(samples);
        self.pending.clear();
        for f in raw.chunks_exact(self.in_ch) {
            self.frame.clear();
            for c in 0..self.out_ch {
                self.frame.push(if self.in_ch == 1 { f[0] } else { f[c % self.in_ch] });
            }
            if !self.have_prev {
                self.prev.clone_from(&self.frame);
                self.have_prev = true;
                continue;
            }
            while self.t < 1.0 {
                let t = self.t as f32;
                for c in 0..self.out_ch {
                    self.pending.push(self.prev[c] + (self.frame[c] - self.prev[c]) * t);
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
    fn chat_mix_gains_follow_sonar() {
        let mut s = MixState { chat_mix: 1.0, ..Default::default() };
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
        let n = buf.lock().unwrap().len();
        assert_eq!(n, 4 * 2 * 2);
    }
}
