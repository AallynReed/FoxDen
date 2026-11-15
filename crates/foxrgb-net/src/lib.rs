//! Ported from OpenRGB Controllers/*Controller (GPL-2.0-or-later): the network and DMX detectors.

mod ddp;
mod dmx;
mod e131;
mod elgato_key_light;
mod elgato_light_strip;
mod espurna;
mod govee;
mod hue;
mod kasa;
mod lifx;
mod nanoleaf;
mod net;
mod wiz;
mod yeelight;

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use serde::{Deserialize, Serialize};

pub use hue::pair as pair_hue;
pub use nanoleaf::pair as pair_nanoleaf;

/// Every network and DMX light the user has set up, as OpenRGB keeps them in its settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetConfig {
    /// Also drive LIFX, Govee, Yeelight, Wiz, Kasa, WLED, Twinkly and Cololight lights found by a search.
    pub discover: bool,
    pub hue: Vec<HueBridge>,
    pub wiz: Vec<WizLight>,
    pub nanoleaf: Vec<NanoleafPanel>,
    pub lifx: Vec<LifxLight>,
    pub govee: Vec<IpOnly>,
    pub yeelight: Vec<YeelightLight>,
    pub kasa: Vec<KasaLight>,
    pub elgato_key_light: Vec<IpOnly>,
    pub elgato_light_strip: Vec<IpOnly>,
    pub e131: Vec<E131Device>,
    pub dmx: Vec<DmxDevice>,
    pub ddp: Vec<DdpDevice>,
    pub espurna: Vec<EspurnaDevice>,
    /// WLED, LeetDesk and SRGBmods Wifi LC controllers, driven by foxrgb-xpack.
    pub wled: Vec<IpOnly>,
    pub twinkly: Vec<IpOnly>,
    pub cololight: Vec<IpOnly>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IpOnly {
    pub ip: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HueBridge {
    pub ip: String,
    pub username: String,
    pub clientkey: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WizLight {
    pub ip: String,
    pub use_cool_white: bool,
    pub use_warm_white: bool,
    /// "Average", "Minimum" or "None".
    pub white_strategy: String,
}

impl Default for WizLight {
    fn default() -> Self {
        WizLight {
            ip: String::new(),
            use_cool_white: false,
            use_warm_white: false,
            white_strategy: "Average".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NanoleafPanel {
    pub ip: String,
    pub port: u16,
    pub auth_token: String,
}

impl Default for NanoleafPanel {
    fn default() -> Self {
        NanoleafPanel {
            ip: String::new(),
            port: nanoleaf::PORT,
            auth_token: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LifxLight {
    pub ip: String,
    pub name: String,
    pub multizone: bool,
    pub extended_multizone: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct YeelightLight {
    pub ip: String,
    /// This PC's address the light connects back to in music mode; found automatically when empty.
    pub host_ip: String,
    pub music_mode: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KasaLight {
    pub ip: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct E131Device {
    pub name: String,
    /// Empty for multicast.
    pub ip: String,
    pub num_leds: u32,
    pub start_universe: u32,
    pub start_channel: u32,
    pub keepalive_time: u32,
    pub universe_size: u32,
}

impl Default for E131Device {
    fn default() -> Self {
        E131Device {
            name: String::new(),
            ip: String::new(),
            num_leds: 0,
            start_universe: 1,
            start_channel: 1,
            keepalive_time: 0,
            universe_size: 512,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DmxDevice {
    pub name: String,
    /// Serial port, such as COM3.
    pub port: String,
    pub keepalive_time: u32,
    pub red_channel: u32,
    pub green_channel: u32,
    pub blue_channel: u32,
    pub brightness_channel: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DdpDevice {
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub num_leds: u32,
    pub keepalive_time: u32,
}

impl Default for DdpDevice {
    fn default() -> Self {
        DdpDevice {
            name: String::new(),
            ip: String::new(),
            port: ddp::PORT,
            num_leds: 0,
            keepalive_time: 1000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EspurnaDevice {
    pub ip: String,
    pub port: u16,
    pub apikey: String,
}

impl Default for EspurnaDevice {
    fn default() -> Self {
        EspurnaDevice {
            ip: String::new(),
            port: 80,
            apikey: String::new(),
        }
    }
}

/// A light a network search turned up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Found {
    /// The `NetConfig` list it belongs in.
    pub family: &'static str,
    pub ip: String,
    pub port: u16,
    pub name: String,
}

/// Searches the local network for lights: SSDP, mDNS and each vendor's LAN broadcast.
pub fn discover() -> Vec<Found> {
    let searches: [fn() -> Vec<Found>; 7] = [
        hue::discover,
        nanoleaf::discover,
        lifx::discover,
        govee::discover,
        yeelight::discover,
        wiz::discover,
        kasa::discover,
    ];
    let mut found: Vec<Found> = thread::scope(|s| {
        let running: Vec<_> = searches.iter().map(|f| s.spawn(f)).collect();
        running
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    found.sort_by(|a, b| (a.family, &a.ip).cmp(&(b.family, &b.ip)));
    found.dedup_by(|a, b| a.family == b.family && a.ip == b.ip);
    found
}

/// Only addresses on this network: private, link-local or loopback, written as a literal.
pub fn local(ip: &str) -> Result<IpAddr, String> {
    let addr: IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| "not an IP address".to_string())?;
    let ok = match addr {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00
        }
    };
    if ok {
        Ok(addr)
    } else {
        Err("not an address on the local network".into())
    }
}

/// Sends one frame to one light.
trait Driver: Send {
    fn send(&mut self, colors: &[Color]) -> Result<(), String>;

    /// What the keepalive timer sends after a quiet spell.
    fn refresh(&mut self, colors: &[Color]) -> Result<(), String> {
        self.send(colors)
    }
}

/// One light or light group, before it is placed on the canvas.
struct Light {
    id: String,
    name: String,
    kind: DeviceKind,
    grid: [u32; 2],
    leds: Vec<[f32; 2]>,
    driver: Box<dyn Driver>,
    keepalive: Option<Duration>,
}

impl Light {
    fn new(
        id: String,
        name: String,
        kind: DeviceKind,
        count: usize,
        driver: Box<dyn Driver>,
    ) -> Self {
        let count = count.max(1);
        Light {
            id,
            name,
            kind,
            grid: [count as u32, 1],
            leds: (0..count)
                .map(|i| [(i as f32 + 0.5) / count as f32, 0.5])
                .collect(),
            driver,
            keepalive: None,
        }
    }

    fn keep(mut self, every: Option<Duration>) -> Self {
        self.keepalive = every;
        self
    }
}

/// Consecutive failed sends before the light is reported as gone.
const GIVE_UP: u32 = 50;

struct Link {
    driver: Box<dyn Driver>,
    last: Vec<Color>,
    at: Instant,
    failures: u32,
}

fn lock(link: &Mutex<Link>) -> MutexGuard<'_, Link> {
    link.lock().unwrap_or_else(|e| e.into_inner())
}

/// One network light. A keepalive thread repeats the last frame for lights that drop out when idle.
struct NetBackend {
    device: Device,
    link: Arc<Mutex<Link>>,
    stop: Arc<AtomicBool>,
    timer: Option<JoinHandle<()>>,
}

impl NetBackend {
    fn new(light: Light, rect: [f32; 4]) -> Self {
        let device = Device {
            id: light.id,
            name: light.name,
            kind: light.kind,
            rect,
            grid: light.grid,
            leds: light.leds,
        };
        let link = Arc::new(Mutex::new(Link {
            driver: light.driver,
            last: Vec::new(),
            at: Instant::now(),
            failures: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let timer = light.keepalive.map(|every| {
            let (link, stop) = (link.clone(), stop.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(100));
                    let mut l = lock(&link);
                    if !l.last.is_empty() && l.at.elapsed() > every.mul_f32(0.95) {
                        let last = l.last.clone();
                        let _ = l.driver.refresh(&last);
                        l.at = Instant::now();
                    }
                }
            })
        });
        NetBackend {
            device,
            link,
            stop,
            timer,
        }
    }
}

impl Backend for NetBackend {
    fn name(&self) -> &str {
        "network"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, _device_id: &str, colors: &[Color]) -> Result<(), String> {
        let mut l = lock(&self.link);
        l.last = colors.to_vec();
        l.at = Instant::now();
        // Like OpenRGB, a lost packet is not fatal; only a light that stays silent is.
        match l.driver.send(colors) {
            Ok(()) => l.failures = 0,
            Err(e) => {
                l.failures += 1;
                if l.failures >= GIVE_UP {
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

impl Drop for NetBackend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

/// Opens every light in `config`, all at once since each may wait on the network.
pub fn probe(config: &NetConfig) -> Vec<Box<dyn Backend>> {
    let found = if config.discover {
        discover()
    } else {
        Vec::new()
    };
    let mut govee: Vec<(String, String)> = config
        .govee
        .iter()
        .map(|g| (g.ip.clone(), String::new()))
        .collect();
    for f in found.iter().filter(|f| f.family == "govee") {
        if !govee.iter().any(|(ip, _)| *ip == f.ip) {
            let sku = f.name.strip_prefix("Govee ").unwrap_or_default();
            govee.push((f.ip.clone(), sku.to_string()));
        }
    }
    let mut lights: Vec<Light> = thread::scope(|s| {
        let mut jobs: Vec<thread::ScopedJoinHandle<'_, Vec<Light>>> = Vec::new();
        for b in &config.hue {
            jobs.push(s.spawn(|| hue::open(b)));
        }
        for w in &config.wiz {
            jobs.push(s.spawn(|| wiz::open(w).into_iter().collect()));
        }
        for n in &config.nanoleaf {
            jobs.push(s.spawn(|| nanoleaf::open(n).into_iter().collect()));
        }
        for l in &config.lifx {
            jobs.push(s.spawn(|| lifx::open(l).into_iter().collect()));
        }
        for y in &config.yeelight {
            jobs.push(s.spawn(|| yeelight::open(y).into_iter().collect()));
        }
        for k in &config.kasa {
            jobs.push(s.spawn(|| kasa::open(k).into_iter().collect()));
        }
        for e in &config.elgato_key_light {
            jobs.push(s.spawn(|| elgato_key_light::open(&e.ip).into_iter().collect()));
        }
        for e in &config.elgato_light_strip {
            jobs.push(s.spawn(|| elgato_light_strip::open(&e.ip).into_iter().collect()));
        }
        for e in &config.espurna {
            jobs.push(s.spawn(|| espurna::open(e).into_iter().collect()));
        }
        jobs.push(s.spawn(|| govee::open_all(&govee)));
        jobs.push(s.spawn(|| e131::open(&config.e131)));
        jobs.push(s.spawn(|| dmx::open(&config.dmx)));
        jobs.push(s.spawn(|| ddp::open(&config.ddp)));
        jobs.push(s.spawn(|| discovered(config, &found)));
        jobs.into_iter()
            .flat_map(|j| j.join().unwrap_or_default())
            .collect()
    });
    let mut seen = Vec::new();
    lights.retain(|l| {
        let new = !seen.contains(&l.id);
        seen.push(l.id.clone());
        new
    });
    let count = lights.len();
    lights
        .into_iter()
        .enumerate()
        .map(|(i, light)| {
            let rect = flow_rect(i, count, light.grid);
            Box::new(NetBackend::new(light, rect)) as Box<dyn Backend>
        })
        .collect()
}

/// Broadcast lights the user has not added, driven with their default settings.
fn discovered(config: &NetConfig, found: &[Found]) -> Vec<Light> {
    let mut out = Vec::new();
    for f in found {
        let ip = f.ip.clone();
        match f.family {
            "lifx" if !config.lifx.iter().any(|l| l.ip == ip) => {
                out.extend(lifx::open(&LifxLight {
                    ip,
                    name: f.name.clone(),
                    ..Default::default()
                }))
            }
            "yeelight" if !config.yeelight.iter().any(|y| y.ip == ip) => {
                out.extend(yeelight::open(&YeelightLight {
                    ip,
                    ..Default::default()
                }))
            }
            "wiz" if !config.wiz.iter().any(|w| w.ip == ip) => out.extend(wiz::open(&WizLight {
                ip,
                ..Default::default()
            })),
            "kasa" if !config.kasa.iter().any(|k| k.ip == ip) => {
                out.extend(kasa::open(&KasaLight {
                    ip,
                    name: f.name.clone(),
                }))
            }
            _ => {}
        }
    }
    out
}

/// One cell per light in a near-square arrangement, so default rects never overlap.
fn flow_rect(index: usize, count: usize, grid: [u32; 2]) -> [f32; 4] {
    let cols = ((count as f32).sqrt().ceil() as usize).max(1);
    let rows = count.div_ceil(cols).max(1);
    let (w, h) = (1.0 / cols as f32, 1.0 / rows as f32);
    let shape = (grid[1] as f32 / grid[0] as f32).clamp(0.15, 1.0);
    [
        ((index % cols) as f32 + 0.1) * w,
        ((index / cols) as f32 + 0.1) * h,
        0.8 * w,
        0.8 * h * shape,
    ]
}

/// OpenRGB's integer `rgb2hsv` (qt/hsv.cpp): hue 0..359, or `u32::MAX` for grey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hsv {
    hue: u32,
    saturation: u8,
    value: u8,
}

fn rgb2hsv(c: Color) -> Hsv {
    let (r, g, b) = (c.0 as i32, c.1 as i32, c.2 as i32);
    let m = r.min(g).min(b);
    let big = r.max(g).max(b);
    let delta = big - m;
    if delta == 0 {
        return Hsv {
            hue: u32::MAX,
            saturation: 0,
            value: big as u8,
        };
    }
    let mut h = if r == big {
        ((g - b) * 60) / delta
    } else if g == big {
        ((b - r) * 60) / delta + 120
    } else {
        ((r - g) * 60) / delta + 240
    };
    if h < 0 {
        h += 360;
    }
    Hsv {
        hue: h as u32,
        saturation: ((256 * delta - 8) / big) as u8,
        value: big as u8,
    }
}

/// A JSON string literal, escaped the way nlohmann's `dump()` does for ordinary text.
fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// A JSON reply, or `Null` when it does not parse.
fn parse(text: &str) -> serde_json::Value {
    serde_json::from_str(text.trim_end_matches('\0')).unwrap_or_default()
}

fn text(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb2hsv_matches_the_integer_conversion() {
        assert_eq!(
            rgb2hsv(Color(255, 0, 0)),
            Hsv {
                hue: 0,
                saturation: 255,
                value: 255
            }
        );
        // (256*255-8)/255 = 255, hue (0-255)*60/255 + 240 = 180
        assert_eq!(rgb2hsv(Color(0, 255, 255)).hue, 180);
        assert_eq!(rgb2hsv(Color(128, 128, 128)).hue, u32::MAX);
        // r is max: (g-b)*60/delta = (0-100)*60/200 = -30, wraps to 330; sat (256*200-8)/200 = 255
        assert_eq!(
            rgb2hsv(Color(200, 0, 100)),
            Hsv {
                hue: 330,
                saturation: 255,
                value: 200
            }
        );
        // sat (256*100-8)/200 = 127
        assert_eq!(rgb2hsv(Color(200, 100, 100)).saturation, 127);
    }

    #[test]
    fn only_local_addresses_are_accepted() {
        for ok in [
            "192.168.1.20",
            "10.0.0.1",
            "172.16.4.4",
            "169.254.3.3",
            "127.0.0.1",
            "fe80::1",
            "fd00::5",
        ] {
            assert!(local(ok).is_ok(), "{ok}");
        }
        for bad in ["8.8.8.8", "bridge.local", "2001:db8::1", "", "172.32.0.1"] {
            assert!(local(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn settings_fill_in_openrgb_defaults() {
        let c: NetConfig =
            serde_json::from_str(r#"{"e131":[{"num_leds":10}],"ddp":[{"ip":"10.0.0.2"}]}"#)
                .unwrap();
        assert_eq!(
            (
                c.e131[0].start_universe,
                c.e131[0].start_channel,
                c.e131[0].universe_size
            ),
            (1, 1, 512)
        );
        assert_eq!((c.ddp[0].port, c.ddp[0].keepalive_time), (4048, 1000));
        assert!(!c.discover);
    }

    struct Count(Arc<Mutex<Vec<Vec<Color>>>>);

    impl Driver for Count {
        fn send(&mut self, colors: &[Color]) -> Result<(), String> {
            self.0.lock().unwrap().push(colors.to_vec());
            Err("lost".into())
        }
    }

    #[test]
    fn keepalive_repeats_the_last_frame_and_lost_packets_are_not_fatal() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let light = Light::new(
            "net:t".into(),
            "T".into(),
            DeviceKind::Other,
            1,
            Box::new(Count(sent.clone())),
        )
        .keep(Some(Duration::from_millis(150)));
        let mut b = NetBackend::new(light, [0.0, 0.0, 1.0, 1.0]);
        assert!(b.push("net:t", &[Color(1, 2, 3)]).is_ok());
        thread::sleep(Duration::from_millis(600));
        drop(b);
        let sent = sent.lock().unwrap();
        assert!(sent.len() >= 2, "{}", sent.len());
        assert!(sent.iter().all(|f| f == &[Color(1, 2, 3)]));
    }
}
