//! Corsair Commander Core, Commander Core XT and iCUE LINK hub cooling (protocol learned from
//! SignalRGB's Corsair_Lighting_Commander_Core.js, Corsair_Lighting_Commander_Core_XT.js and
//! Corsair_ICUE_Link_Hub.js).

use super::util::{Cached, ask_strict, device_key, due, fan_id, u16le};
use crate::fan::{Channel, FanHub, PUMP_MIN, backend_on, percent};
use crate::hid::{Io, open_io};
use foxrgb_fan::FanBackend;
use foxrgb_hid::port::FANS;
use foxrgb_hid::registry::Info;
use hidapi::HidApi;
use std::time::Instant;

pub const VID: u16 = 0x1B1C;
pub const CORE: u16 = 0x0C1C;
pub const CORE2: u16 = 0x0C32;
pub const XT: u16 = 0x0C2A;
pub const LINK: u16 = 0x0C3F;
pub const DUO: u16 = super::commander_duo::PID;
pub const IDS: &[(u16, &str)] = &[
    (CORE, "Corsair Commander Core"),
    (CORE2, "Corsair Commander Core"),
    (XT, "Corsair Commander Core XT"),
    (LINK, "Corsair iCUE LINK Hub"),
    (DUO, "Corsair iCUE Commander DUO"),
];
const PAGE: u16 = 0xFF42;

const SET_PROPERTY: u8 = 0x01;
const GET_PROPERTY: u8 = 0x02;
const CLOSE: u8 = 0x05;
const WRITE: u8 = 0x06;
const READ: u8 = 0x08;
const OPEN: u8 = 0x0D;
const PROP_MODE: u8 = 0x03;
const PROP_FIRMWARE: u8 = 0x13;
const HARDWARE: u32 = 0x01;
const SOFTWARE: u32 = 0x02;
const BACKGROUND: u8 = 0x01;
const AUXILIARY: u8 = 0x02;
const LINK_DEVICE: u8 = 0x01;

const EP_RPM: u8 = 0x17;
const EP_DUTY: u8 = 0x18;
const EP_STATES: u8 = 0x1A;
const EP_TEMPS: u8 = 0x21;
const EP_CHILDREN: u8 = 0x36;
const T_RPM: u8 = 0x06;
const T_DUTY: u8 = 0x07;
const T_STATES: u8 = 0x09;
const T_TEMPS: u8 = 0x10;
const T_CHILDREN: u8 = 0x21;
/// Fan state from endpoint 0x1A: nothing on the header.
const DISCONNECTED: u8 = 0x01;

/// `SendCoolingdata` / `SetFanSpeeds` defaults when a channel has no level.
const CORE_DEFAULT: u8 = 0x32;
const LINK_DEFAULT: u8 = 0x35;
/// Below 60% the Capellix pump goes into failsafe until power cycled.
const CORE_PUMP_MIN: u8 = 60;
const LINK_SIZE: usize = 513;
/// Header bytes before the data of a continued LINK read.
const LINK_MORE: usize = 4;
const REPLY_MS: u64 = 500;
const POLL_MS: u64 = 1500;
const STATE_MS: u64 = 10_000;
const RESEND_MS: u64 = 2000;
const MODE_CHECK_MS: u64 = 10_000;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Flavor {
    Core,
    Xt,
    Link,
}

impl Flavor {
    fn of(pid: u16) -> Option<Flavor> {
        match pid {
            CORE | CORE2 => Some(Flavor::Core),
            XT | DUO => Some(Flavor::Xt),
            LINK => Some(Flavor::Link),
            _ => None,
        }
    }

    /// Reply bytes sit one later on the LINK hub, which addresses a device in its header.
    fn base(self) -> usize {
        usize::from(self == Flavor::Link)
    }

    fn header(self) -> &'static [u8] {
        match self {
            Flavor::Link => &[0x00, 0x00, LINK_DEVICE],
            _ => &[0x00, 0x08],
        }
    }

    fn default_pct(self, pump: bool) -> u8 {
        let base = if self == Flavor::Link {
            LINK_DEFAULT
        } else {
            CORE_DEFAULT
        };
        if pump { base.max(CORE_PUMP_MIN) } else { base }
    }
}

/// `Validate`: interface 0, usage page 0xFF42, usage 1.
pub fn wants(info: &Info) -> bool {
    info.vid == VID
        && Flavor::of(info.pid).is_some()
        && info.interface == 0
        && info.usage_page == PAGE
        && info.usage == 0x01
}

/// One request: the flavour's header then the command bytes.
pub fn packet(flavor: Flavor, cmd: &[u8]) -> Vec<u8> {
    let mut p = flavor.header().to_vec();
    p.extend_from_slice(cmd);
    p
}

/// `WriteEndpoint`: length as 32-bit little-endian, then the data.
pub fn write_cmd(handle: u8, data: &[u8]) -> Vec<u8> {
    let mut c = vec![WRITE, handle];
    c.extend((data.len() as u32).to_le_bytes());
    c.extend_from_slice(data);
    c
}

/// The fan duty table: `07 00 n`, then `id 00 pct 00` per channel.
pub fn duty_table(entries: &[(u8, u8)]) -> Vec<u8> {
    let mut d = vec![T_DUTY, 0x00, entries.len() as u8];
    for &(id, pct) in entries {
        d.extend([id, 0x00, pct.min(100), 0x00]);
    }
    d
}

/// Packet size before the firmware is known, as the devices declare it.
fn first_size(pid: u16) -> usize {
    match pid {
        CORE2 | DUO => 65,
        XT => 385,
        LINK => LINK_SIZE,
        _ => 97,
    }
}

/// The original Commander Core moved to 1025-byte packets with firmware 1.x.
fn size_after(pid: u16, size: usize, firmware_major: u8) -> usize {
    if pid == CORE && firmware_major == 1 {
        1025
    } else {
        size
    }
}

/// The data of an endpoint read when its type matches: (count, entries).
pub fn payload(flavor: Flavor, reply: &[u8], kind: u8) -> Option<(usize, &[u8])> {
    let b = flavor.base();
    if reply.get(3 + b)? != &kind || reply.get(4 + b)? != &0 {
        return None;
    }
    Some((*reply.get(5 + b)? as usize, reply.get(6 + b..)?))
}

/// Fan RPMs: two bytes each, or status plus two bytes on LINK.
pub fn parse_rpms(flavor: Flavor, reply: &[u8]) -> Option<Vec<u32>> {
    let (n, d) = payload(flavor, reply, T_RPM)?;
    let (stride, skip) = if flavor == Flavor::Link {
        (3, 1)
    } else {
        (2, 0)
    };
    Some(
        (0..n)
            .map_while(|i| u16le(d, i * stride + skip).map(u32::from))
            .collect(),
    )
}

/// Temperatures: status plus tenths of a degree, little-endian. Zero means no reading.
pub fn parse_temps(flavor: Flavor, reply: &[u8]) -> Option<Vec<Option<f32>>> {
    let (n, d) = payload(flavor, reply, T_TEMPS)?;
    Some(
        (0..n)
            .map_while(|i| u16le(d, i * 3 + 1))
            .map(|raw| (raw != 0).then_some(f32::from(raw) / 10.0))
            .collect(),
    )
}

pub fn parse_states(flavor: Flavor, reply: &[u8]) -> Option<Vec<u8>> {
    let (n, d) = payload(flavor, reply, T_STATES)?;
    Some(d.get(..n)?.to_vec())
}

/// A LINK child: (type, subtype), as `fetchLinkDevices` scans the 0x36 endpoint.
pub fn parse_children(data: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut xd6 = false;
    let mut b = 7;
    while b + 5 < data.len() {
        let rec = &data[b..b + 6];
        if rec[0] == 0 && rec[1] == 0 && rec[2] != 0 && rec[4] == 0 && rec[5] == 0 {
            let (kind, sub) = (rec[2], rec[3]);
            b += 32;
            if kind == 25 {
                xd6 = true;
            }
            if kind == 14 && xd6 {
                xd6 = false;
                continue;
            }
            if kind != 6 {
                out.push((kind, sub));
            }
        } else {
            b += 1;
        }
    }
    out
}

struct Kind {
    kind: u8,
    model: Option<u8>,
    name: &'static str,
    rpm: bool,
    probe: bool,
    pump: bool,
}

const fn k(kind: u8, name: &'static str, rpm: bool, probe: bool, pump: bool) -> Kind {
    Kind {
        kind,
        model: None,
        name,
        rpm,
        probe,
        pump,
    }
}

/// `CorsairLibrary.ChildList`: which LINK devices report RPM and temperature.
const KINDS: &[Kind] = &[
    k(1, "QX Fan", true, true, false),
    k(2, "LX Fan", true, false, false),
    k(15, "RX Fan", true, false, false),
    k(19, "RX PWM Fan", true, false, false),
    k(3, "RX MAX Fan", true, true, false),
    k(4, "RX MAX PWM Fan", true, true, false),
    k(7, "LINK AIO Pump", true, true, true),
    k(17, "LINK Titan AIO Pump", true, true, true),
    k(10, "LINK XG3 HYBRID", true, true, true),
    k(12, "LINK XD5 Elite", true, true, true),
    k(14, "LINK XD5 Elite", true, true, true),
    k(25, "LINK XD6 Elite", true, true, true),
    k(16, "LINK VRM Fan", true, false, false),
    k(27, "Commander DUO Channel", true, true, false),
    k(9, "LINK XC7 Elite", false, true, false),
    k(11, "LINK PSU", false, true, false),
    Kind {
        model: Some(0),
        ..k(26, "LINK Fan", true, false, false)
    },
    Kind {
        model: Some(1),
        ..k(26, "LINK AIO Pump", true, true, true)
    },
];

fn kind_of(kind: u8, model: u8) -> Option<&'static Kind> {
    KINDS
        .iter()
        .find(|k| k.kind == kind && k.model.is_none_or(|m| m == model))
}

/// The child list read record by record: (channel, type, model) of every occupied channel.
/// A record is eight bytes, the device id's length in its last, then the id.
pub fn parse_link(data: &[u8]) -> Option<Vec<(u8, u8, u8)>> {
    let n = *data.get(6)?;
    let mut at = 7;
    let mut out = Vec::new();
    for ch in 1..=n {
        let rec = data.get(at..at + 8)?;
        let len = usize::from(rec[7]);
        if len > 0 {
            out.push((ch, rec[2], rec[3]));
        }
        at += 8 + len;
    }
    Some(out)
}

/// Every RPM-reporting child on its own channel's slot; `None` when a slot was not read.
pub fn map_link_exact(children: &[(u8, u8, u8)], rpms: &[u32]) -> Option<Vec<Chan>> {
    children
        .iter()
        .filter_map(|&(slot, kind, model)| Some((slot, kind_of(kind, model).filter(|k| k.rpm)?)))
        .enumerate()
        .map(|(i, (slot, k))| {
            (usize::from(slot) < rpms.len()).then(|| Chan {
                name: format!("{} {}", k.name, i + 1),
                pump: k.pump,
                slot,
            })
        })
        .collect()
}

/// Probe-reporting children read their own channel's temperature slot.
pub fn map_link_sensors_exact(
    children: &[(u8, u8, u8)],
    temps: &[Option<f32>],
) -> Option<Vec<(String, usize)>> {
    children
        .iter()
        .filter_map(|&(slot, kind, model)| Some((slot, kind_of(kind, model).filter(|k| k.probe)?)))
        .enumerate()
        .map(|(i, (slot, k))| {
            let slot = usize::from(slot);
            (slot < temps.len()).then(|| (format!("{} {}", k.name, i + 1), slot))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chan {
    pub name: String,
    pub pump: bool,
    pub slot: u8,
}

/// Gives each RPM-reporting child the next slot that read non-zero, in order, as the plugin
/// does. When the counts disagree every slot becomes a pump-safe channel of its own.
pub fn map_link(children: &[(u8, u8)], rpms: &[u32]) -> Vec<Chan> {
    let live: Vec<u8> = (0..rpms.len() as u8)
        .filter(|&i| rpms[i as usize] != 0)
        .collect();
    let fans: Vec<&Kind> = children
        .iter()
        .filter_map(|c| KINDS.iter().find(|k| k.kind == c.0))
        .filter(|k| k.rpm)
        .collect();
    if !live.is_empty() && fans.len() == live.len() {
        return fans
            .iter()
            .zip(&live)
            .enumerate()
            .map(|(i, (k, &slot))| Chan {
                name: format!("{} {}", k.name, i + 1),
                pump: k.pump,
                slot,
            })
            .collect();
    }
    (0..rpms.len() as u8)
        .map(|slot| Chan {
            name: format!("Slot {}", slot + 1),
            pump: true,
            slot,
        })
        .collect()
}

/// Probe-reporting children get the temperature slots that read, in order.
pub fn map_link_sensors(children: &[(u8, u8)], temps: &[Option<f32>]) -> Vec<(String, usize)> {
    let live: Vec<usize> = (0..temps.len()).filter(|&i| temps[i].is_some()).collect();
    let probes: Vec<&Kind> = children
        .iter()
        .filter_map(|c| KINDS.iter().find(|k| k.kind == c.0))
        .filter(|k| k.probe)
        .collect();
    if probes.len() == live.len() {
        return probes
            .iter()
            .zip(&live)
            .enumerate()
            .map(|(i, (k, &slot))| (format!("{} {}", k.name, i + 1), slot))
            .collect();
    }
    (0..temps.len())
        .map(|i| (format!("Probe {}", i + 1), i))
        .collect()
}

struct Bus {
    io: Box<dyn Io>,
    flavor: Flavor,
    size: usize,
    handle: u8,
}

impl Bus {
    fn call(&mut self, cmd: &[u8], want: impl Fn(&[u8]) -> bool) -> Result<Vec<u8>, String> {
        let p = packet(self.flavor, cmd);
        ask_strict(self.io.as_mut(), &p, self.size, self.size, REPLY_MS, want)
    }

    /// A command whose reply only carries a status byte.
    fn command(&mut self, cmd: &[u8]) -> Result<(), String> {
        let (b, first) = (self.flavor.base(), cmd[0]);
        let core = self.flavor != Flavor::Link;
        let r = self.call(cmd, |r| !core || r.get(1) == Some(&first))?;
        match r.get(2 + b) {
            Some(0) | None => Ok(()),
            Some(e) => Err(format!("device error {e}")),
        }
    }

    fn get_property(&mut self, prop: u8) -> Option<u32> {
        let b = self.flavor.base();
        let core = self.flavor != Flavor::Link;
        let r = self
            .call(&[GET_PROPERTY, prop, 0x00], |r| {
                !core || r.get(1) == Some(&GET_PROPERTY)
            })
            .ok()?;
        if r.get(2 + b) != Some(&0) {
            return None;
        }
        let v = r.get(3 + b..6 + b)?;
        Some(u32::from(v[0]) | u32::from(v[1]) << 8 | u32::from(v[2]) << 16)
    }

    fn set_property(&mut self, prop: u8, value: u32) -> Result<(), String> {
        let v = value.to_le_bytes();
        self.command(&[SET_PROPERTY, prop, 0x00, v[0], v[1], v[2]])
    }

    fn open(&mut self, endpoint: u8) -> Result<(), String> {
        if self.command(&[OPEN, self.handle, endpoint]).is_ok() {
            return Ok(());
        }
        self.close();
        self.command(&[OPEN, self.handle, endpoint])
    }

    fn close(&mut self) {
        let _ = self.command(&[CLOSE, 0x01, self.handle]);
    }

    /// `ReadEndpoint`: open, read until the data type answers, close.
    fn read(&mut self, endpoint: u8, kind: u8) -> Option<Vec<u8>> {
        self.open(endpoint).ok()?;
        let (flavor, handle) = (self.flavor, self.handle);
        let mut out = None;
        for _ in 0..3 {
            if let Ok(r) = self.call(&[READ, handle], |r| payload(flavor, r, kind).is_some()) {
                out = Some(r);
                break;
            }
        }
        self.close();
        out
    }

    /// The LINK child list spans two reads; the second's three header bytes are dropped.
    fn read_children(&mut self) -> Option<Vec<u8>> {
        self.open(EP_CHILDREN).ok()?;
        let handle = self.handle;
        let first = self
            .call(&[READ, handle], |r| r.get(4) == Some(&T_CHILDREN))
            .ok();
        let second = first
            .as_ref()
            .and_then(|_| self.call(&[READ, handle], |_| true).ok());
        self.close();
        let mut data = first?;
        if let Some(s) = second {
            data.extend(s.get(LINK_MORE..).unwrap_or(&[]));
        }
        Some(data)
    }

    fn write(&mut self, endpoint: u8, data: &[u8]) -> Result<(), String> {
        self.open(endpoint)?;
        let r = self.command(&write_cmd(self.handle, data));
        self.close();
        r
    }

    fn mode(&mut self) -> Option<u32> {
        self.get_property(PROP_MODE)
    }

    /// Runs `f`, and once more in software mode when hardware mode refused it.
    fn with_access<T>(&mut self, f: impl Fn(&mut Bus) -> Option<T>) -> Option<T> {
        if let Some(v) = f(self) {
            return Some(v);
        }
        if self.mode() != Some(HARDWARE) || self.set_property(PROP_MODE, SOFTWARE).is_err() {
            return None;
        }
        let v = f(self);
        let _ = self.set_property(PROP_MODE, HARDWARE);
        v
    }
}

struct Hub {
    bus: Bus,
    id: String,
    name: String,
    chans: Vec<Chan>,
    sensors: Vec<(String, usize)>,
    duty: Vec<Option<u8>>,
    rpm: Cached<Vec<u32>>,
    temps: Cached<Vec<Option<f32>>>,
    states: Cached<Vec<u8>>,
    took: bool,
    controlling: bool,
    sent: Option<(Vec<u8>, Instant)>,
    mode_at: Option<Instant>,
    /// Released while lighting holds software mode: fans follow the balanced liquid curve.
    following: bool,
}

impl Hub {
    fn table(&self) -> Vec<u8> {
        let flavor = self.bus.flavor;
        let entries: Vec<(u8, u8)> = self
            .chans
            .iter()
            .zip(&self.duty)
            .map(|(c, d)| {
                let pct = d.unwrap_or_else(|| match (self.following, self.liquid()) {
                    (true, Some(t)) if !c.pump => super::platinum::balanced_pct(t),
                    _ => flavor.default_pct(c.pump),
                });
                let floor = if c.pump && flavor == Flavor::Core {
                    CORE_PUMP_MIN
                } else if c.pump {
                    percent(PUMP_MIN)
                } else {
                    0
                };
                (c.slot, pct.max(floor))
            })
            .collect();
        duty_table(&entries)
    }

    fn push(&mut self, force: bool) -> Result<(), String> {
        let t = self.table();
        let fresh = self
            .sent
            .as_ref()
            .is_some_and(|(s, at)| *s == t && !due(Some(*at), RESEND_MS));
        if force || !fresh {
            self.bus.write(EP_DUTY, &t)?;
            self.sent = Some((t, Instant::now()));
        }
        Ok(())
    }

    /// Takes software mode when the device is in hardware mode; rechecked now and then.
    fn take(&mut self) -> Result<(), String> {
        if self.controlling && !due(self.mode_at, MODE_CHECK_MS) {
            return Ok(());
        }
        self.mode_at = Some(Instant::now());
        if self.bus.mode() == Some(HARDWARE) {
            self.bus.set_property(PROP_MODE, SOFTWARE)?;
            self.took = true;
            self.sent = None;
        }
        self.controlling = true;
        self.following = false;
        if let Some(p) = self.bus.io.port() {
            p.device().set_active(FANS, true);
        }
        Ok(())
    }

    /// Hardware mode hands lighting back too, so it is sent only once lighting has let go.
    fn let_go(&mut self) -> Result<(), String> {
        self.controlling = false;
        let device = self.bus.io.port().map(|p| p.device().clone());
        let lit = device.as_ref().is_some_and(|d| {
            d.set_active(FANS, false);
            d.others_active(FANS)
        });
        let owed = device.as_ref().is_some_and(|d| d.owed());
        let took = std::mem::take(&mut self.took);
        if (took || owed) && !lit {
            if let Some(d) = &device {
                d.owe(false);
            }
            self.sent = None;
            return self.bus.set_property(PROP_MODE, HARDWARE);
        }
        self.following = lit;
        self.read_temps();
        self.push(true)
    }

    /// The hottest temperature the hub last read.
    fn liquid(&self) -> Option<f32> {
        let temps = self.temps.peek()?;
        temps.iter().flatten().copied().reduce(f32::max)
    }

    fn read_temps(&mut self) {
        let flavor = self.bus.flavor;
        let bus = &mut self.bus;
        self.temps
            .get(|| parse_temps(flavor, &bus.read(EP_TEMPS, T_TEMPS)?));
    }

    /// Follows the liquid while lighting still holds the device; its release hands the fans back.
    fn follow(&mut self) {
        let lit = self
            .bus
            .io
            .port()
            .is_some_and(|p| p.device().others_active(FANS));
        if !lit {
            self.following = false;
            return;
        }
        self.read_temps();
        let _ = self.push(false);
    }
}

impl FanHub for Hub {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.clone()
    }
    fn channels(&self) -> Vec<Channel> {
        self.chans
            .iter()
            .map(|c| match (c.pump, self.bus.flavor) {
                (true, Flavor::Core) => Channel {
                    min_duty: f32::from(CORE_PUMP_MIN) / 100.0,
                    ..Channel::pump(&c.name)
                },
                (true, _) => Channel::pump(&c.name),
                (false, _) => Channel::fan(&c.name),
            })
            .collect()
    }
    fn sensors(&self) -> Vec<String> {
        self.sensors.iter().map(|s| s.0.clone()).collect()
    }
    fn rpm(&mut self, channel: usize) -> Option<u32> {
        let slot = self.chans.get(channel)?.slot as usize;
        if self.controlling {
            let _ = self.take();
            let _ = self.push(false);
        } else if self.following {
            self.follow();
        }
        let flavor = self.bus.flavor;
        let bus = &mut self.bus;
        if flavor != Flavor::Link {
            let states = self
                .states
                .get(|| parse_states(flavor, &bus.read(EP_STATES, T_STATES)?));
            if states.and_then(|s| s.get(slot).copied()) == Some(DISCONNECTED) {
                return None;
            }
        }
        let rpms = self
            .rpm
            .get(|| parse_rpms(flavor, &bus.read(EP_RPM, T_RPM)?))?;
        rpms.get(slot).copied()
    }
    fn set_duty(&mut self, channel: usize, duty: f32) -> Result<(), String> {
        if channel >= self.chans.len() {
            return Err("no such channel".into());
        }
        self.duty[channel] = Some(percent(duty));
        self.take()?;
        self.push(false)
    }
    fn release(&mut self, channel: usize) -> Result<(), String> {
        if let Some(d) = self.duty.get_mut(channel) {
            *d = None;
        }
        if !self.controlling {
            return Ok(());
        }
        if self.duty.iter().any(Option::is_some) {
            return self.push(true);
        }
        self.let_go()
    }
    fn temperature(&mut self, sensor: usize) -> Option<f32> {
        let slot = self.sensors.get(sensor)?.1;
        let flavor = self.bus.flavor;
        let bus = &mut self.bus;
        let temps = self
            .temps
            .get(|| parse_temps(flavor, &bus.read(EP_TEMPS, T_TEMPS)?))?;
        temps.get(slot).copied().flatten()
    }
}

fn open_bus(api: &HidApi, info: &Info, flavor: Flavor) -> Option<Bus> {
    let io = open_io(api, info).ok()?;
    let first = first_size(info.pid);
    let mut bus = Bus {
        io,
        flavor,
        size: first,
        handle: BACKGROUND,
    };
    if flavor == Flavor::Core {
        let major = bus
            .get_property(PROP_FIRMWARE)
            .map(|v| v as u8)
            .unwrap_or(2);
        bus.size = size_after(info.pid, first, major);
    }
    Some(bus)
}

pub fn hub(api: &HidApi, info: &Info) -> Option<Box<dyn FanBackend>> {
    let flavor = Flavor::of(info.pid)?;
    let name = IDS.iter().find(|i| i.0 == info.pid)?.1;
    let mut bus = open_bus(api, info, flavor)?;
    let (chans, sensors) = match flavor {
        Flavor::Core | Flavor::Xt => {
            let temps = bus.with_access(|b| parse_temps(flavor, &b.read(EP_TEMPS, T_TEMPS)?));
            let core = flavor == Flavor::Core;
            let probes = temps.map_or(if core { 1 } else { 2 }, |t| t.len());
            let sensors = (0..probes)
                .map(|i| match (core, i) {
                    (true, 0) => ("Liquid Temperature".to_string(), 0),
                    _ => (format!("Temperature Probe {}", i + 1), i),
                })
                .collect();
            let mut chans = Vec::new();
            if core {
                chans.push(Chan {
                    name: "Pump".into(),
                    pump: true,
                    slot: 0,
                });
            }
            let from = u8::from(core);
            let fans = if info.pid == DUO { 2 } else { 6 };
            chans.extend((0..fans).map(|i| Chan {
                name: format!("Fan {}", i + 1),
                pump: false,
                slot: from + i,
            }));
            (chans, sensors)
        }
        Flavor::Link => {
            let (children, rpms, temps) = bus.with_access(|b| {
                b.handle = AUXILIARY;
                if b.open(EP_RPM).is_ok() {
                    b.close();
                } else {
                    b.handle = BACKGROUND;
                }
                let children = b.read_children().unwrap_or_default();
                let rpms = parse_rpms(flavor, &b.read(EP_RPM, T_RPM)?)?;
                let temps = b
                    .read(EP_TEMPS, T_TEMPS)
                    .and_then(|r| parse_temps(flavor, &r))
                    .unwrap_or_default();
                Some((children, rpms, temps))
            })?;
            let exact = parse_link(&children);
            let chans = exact
                .as_ref()
                .and_then(|e| map_link_exact(e, &rpms))
                .filter(|c| !c.is_empty());
            let sensors = exact
                .as_ref()
                .and_then(|e| map_link_sensors_exact(e, &temps));
            let kids = parse_children(&children);
            (
                chans.unwrap_or_else(|| map_link(&kids, &rpms)),
                sensors.unwrap_or_else(|| map_link_sensors(&kids, &temps)),
            )
        }
    };
    if chans.is_empty() {
        return None;
    }
    let n = chans.len();
    Some(backend_on(
        info,
        Hub {
            bus,
            id: fan_id(info, "corsair-modern"),
            name: name.into(),
            chans,
            sensors,
            duty: vec![None; n],
            rpm: Cached::new(POLL_MS),
            temps: Cached::new(POLL_MS),
            states: Cached::new(STATE_MS),
            took: false,
            controlling: false,
            sent: None,
            mode_at: None,
            following: false,
        },
    ))
}

pub fn key(info: &Info) -> String {
    device_key(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_and_commands() {
        assert_eq!(
            packet(Flavor::Core, &[OPEN, BACKGROUND, EP_RPM]),
            vec![0x00, 0x08, 0x0D, 0x01, 0x17]
        );
        assert_eq!(
            packet(Flavor::Link, &[READ, AUXILIARY]),
            vec![0x00, 0x00, 0x01, 0x08, 0x02]
        );
        assert_eq!(
            write_cmd(1, &[0xAA, 0xBB]),
            vec![0x06, 0x01, 2, 0, 0, 0, 0xAA, 0xBB]
        );
    }

    // SendCoolingdata: 07 00 07, then id 00 level 00
    #[test]
    fn duty_table_layout() {
        let t = duty_table(&[(0, 60), (1, 50), (2, 140)]);
        assert_eq!(t, vec![7, 0, 3, 0, 0, 60, 0, 1, 0, 50, 0, 2, 0, 100, 0]);
    }

    #[test]
    fn core_replies() {
        let rpm = [
            0x00, 0x08, 0x00, 0x06, 0x00, 0x03, 0xDC, 0x05, 0x00, 0x00, 0x20, 0x03,
        ];
        assert_eq!(parse_rpms(Flavor::Core, &rpm), Some(vec![1500, 0, 800]));
        let mut wrong = rpm;
        wrong[3] = 0x09;
        assert_eq!(parse_rpms(Flavor::Core, &wrong), None);
        let temps = [
            0x00, 0x08, 0x00, 0x10, 0x00, 0x02, 0x00, 0x2C, 0x01, 0x01, 0x00, 0x00,
        ];
        assert_eq!(
            parse_temps(Flavor::Core, &temps),
            Some(vec![Some(30.0), None])
        );
        let states = [0x00, 0x08, 0x00, 0x09, 0x00, 0x03, 0x07, 0x01, 0x07];
        assert_eq!(parse_states(Flavor::Core, &states), Some(vec![7, 1, 7]));
    }

    #[test]
    fn link_rpms_have_a_status_byte() {
        let r = [
            0x00, 0x00, 0x08, 0x00, 0x06, 0x00, 0x02, 0x00, 0xE8, 0x03, 0x00, 0x00, 0x00,
        ];
        assert_eq!(parse_rpms(Flavor::Link, &r), Some(vec![1000, 0]));
    }

    #[test]
    fn link_children_and_mapping() {
        let mut d = vec![0u8; 7];
        for (kind, sub) in [(1u8, 0u8), (6, 0), (7, 2), (25, 0), (14, 0)] {
            let mut rec = vec![0, 0, kind, sub, 0, 0, 0, 24];
            rec.resize(32, b'x');
            d.extend(rec);
        }
        let kids = parse_children(&d);
        assert_eq!(kids, vec![(1, 0), (7, 2), (25, 0)]);
        let chans = map_link(&kids, &[1200, 0, 2400, 900]);
        assert_eq!(
            chans,
            vec![
                Chan {
                    name: "QX Fan 1".into(),
                    pump: false,
                    slot: 0
                },
                Chan {
                    name: "LINK AIO Pump 2".into(),
                    pump: true,
                    slot: 2
                },
                Chan {
                    name: "LINK XD6 Elite 3".into(),
                    pump: true,
                    slot: 3
                },
            ]
        );
        let unsure = map_link(&kids, &[1200, 0, 0, 0]);
        assert_eq!(unsure.len(), 4);
        assert!(unsure.iter().all(|c| c.pump));
        let sensors = map_link_sensors(&kids, &[Some(30.0), None, Some(28.5)]);
        assert_eq!(sensors.len(), 3);
        assert_eq!(sensors[0].1, 0);
    }

    fn record(kind: u8, model: u8, id: &[u8]) -> Vec<u8> {
        let mut r = vec![0, 0, kind, model, 0, 0, 0, id.len() as u8];
        r.extend_from_slice(id);
        r
    }

    #[test]
    fn link_children_map_to_their_own_channel() {
        let mut d = vec![0, 0, 0, 0, 0x21, 0, 6];
        d.extend(record(1, 0, b"QX0000000000000000000001"));
        d.extend([0u8; 8]);
        d.extend(record(7, 2, b"AIO000000000000000000002"));
        d.extend(record(9, 0, b"XC7000000000000000000003"));
        d.extend(record(26, 1, b"PUMP0000000000000000004"));
        d.extend(record(26, 8, b"LCD00000000000000000005"));
        let kids = parse_link(&d).unwrap();
        assert_eq!(
            kids,
            [(1, 1, 0), (3, 7, 2), (4, 9, 0), (5, 26, 1), (6, 26, 8)]
        );
        let chans = map_link_exact(&kids, &[0, 0, 1500, 0, 0, 2800, 0]).unwrap();
        let got: Vec<(&str, bool, u8)> = chans
            .iter()
            .map(|c| (c.name.as_str(), c.pump, c.slot))
            .collect();
        assert_eq!(
            got,
            [
                ("QX Fan 1", false, 1),
                ("LINK AIO Pump 2", true, 3),
                ("LINK AIO Pump 3", true, 5)
            ]
        );
        assert_eq!(map_link_exact(&kids, &[0, 0, 0]), None);
        let sensors = map_link_sensors_exact(&kids, &[None; 7]).unwrap();
        let slots: Vec<usize> = sensors.iter().map(|s| s.1).collect();
        assert_eq!(slots, [1, 3, 4, 5]);
        assert_eq!(parse_link(&d[..20]), None);
    }

    /// Answers each Core command once with success, and records what was written.
    #[derive(Clone, Default)]
    struct Echo {
        writes: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
        owed: std::sync::Arc<std::sync::Mutex<Option<u8>>>,
    }

    impl foxrgb_hid::port::Raw for Echo {
        fn write(&self, data: &[u8]) -> foxrgb_hid::port::HidResult<usize> {
            self.writes.lock().unwrap().push(data.to_vec());
            *self.owed.lock().unwrap() = data.get(2).copied();
            Ok(data.len())
        }
        fn read_timeout(&self, buf: &mut [u8], _: i32) -> foxrgb_hid::port::HidResult<usize> {
            let Some(cmd) = self.owed.lock().unwrap().take() else {
                return Ok(0);
            };
            buf.fill(0);
            buf[1] = cmd;
            Ok(buf.len())
        }
        fn send_feature_report(&self, _: &[u8]) -> foxrgb_hid::port::HidResult<()> {
            Ok(())
        }
        fn get_feature_report(&self, _: &mut [u8]) -> foxrgb_hid::port::HidResult<usize> {
            Ok(0)
        }
        fn get_input_report(&self, _: &mut [u8]) -> foxrgb_hid::port::HidResult<usize> {
            Ok(0)
        }
    }

    fn duty_writes(e: &Echo) -> Vec<Vec<u8>> {
        let start = [WRITE, BACKGROUND];
        let writes = e.writes.lock().unwrap();
        writes
            .iter()
            .filter_map(|w| {
                let t = &w[w.windows(2).position(|p| p == start)? + 6..];
                Some(t[..3 + 4 * usize::from(t[2])].to_vec())
            })
            .collect()
    }

    // Released fans follow the balanced curve while lighting holds the hub, then let go with it.
    #[test]
    fn released_fans_follow_the_liquid_while_lighting_holds_the_hub() {
        use foxrgb_hid::port::{LIGHTS, Port};
        let echo = Echo::default();
        let port = Port::alone(Box::new(echo.clone()));
        let device = port.device().clone();
        let mut hub = Hub {
            bus: Bus {
                io: Box::new(port),
                flavor: Flavor::Core,
                size: 97,
                handle: BACKGROUND,
            },
            id: "fan:c".into(),
            name: "C".into(),
            chans: vec![
                Chan {
                    name: "Pump".into(),
                    pump: true,
                    slot: 0,
                },
                Chan {
                    name: "Fan 1".into(),
                    pump: false,
                    slot: 1,
                },
            ],
            sensors: vec![("Liquid Temperature".into(), 0)],
            duty: vec![None; 2],
            rpm: Cached::new(60_000),
            temps: Cached::new(60_000),
            states: Cached::new(60_000),
            took: false,
            controlling: false,
            sent: None,
            mode_at: None,
            following: false,
        };
        hub.temps.get(|| Some(vec![Some(33.0)]));
        hub.rpm.get(|| Some(vec![0, 0]));
        hub.states.get(|| Some(vec![0, 0]));
        device.set_active(LIGHTS, true);
        hub.set_duty(1, 0.2).unwrap();
        hub.release(1).unwrap();
        let curve = super::super::platinum::balanced_pct(33.0);
        assert_eq!(curve, 68);
        let last = duty_writes(&echo).pop().unwrap();
        assert_eq!(last, duty_table(&[(0, 60), (1, curve)]));
        assert!(hub.following && !hub.controlling);

        device.set_active(LIGHTS, false);
        echo.writes.lock().unwrap().clear();
        hub.rpm(1);
        assert!(!hub.following);
        assert!(duty_writes(&echo).is_empty());
    }

    #[test]
    fn the_balanced_curve_runs_from_20_to_100_percent() {
        use super::super::platinum::balanced_pct;
        assert_eq!(balanced_pct(20.0), 20);
        assert_eq!(balanced_pct(29.5), 23);
        assert_eq!(balanced_pct(35.0), 100);
        assert_eq!(balanced_pct(60.0), 100);
    }

    #[test]
    fn sizes() {
        assert_eq!(first_size(CORE2), 65);
        assert_eq!(first_size(XT), 385);
        assert_eq!(first_size(DUO), 65);
        assert_eq!(Flavor::of(DUO), Some(Flavor::Xt));
        assert_eq!(size_after(CORE, 97, 1), 1025);
        assert_eq!(size_after(CORE, 97, 2), 97);
        assert_eq!(Flavor::Core.default_pct(true), 60);
        assert_eq!(Flavor::Xt.default_pct(false), 0x32);
    }
}
