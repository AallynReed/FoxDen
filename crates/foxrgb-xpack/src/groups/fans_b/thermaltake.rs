//! ThermalTake Riing, Riing Plus, Riing Trio and LEDBOX controllers (protocol learned from
//! SignalRGB's ThermalTake_Riing_Plus.js, ThermalTake_Ring_Controller.js,
//! ThermalTake_LEDBOX_Standard.js and ThermalTake_LEDBOX_0x213Cf.js).

use super::util::{Cached, ask, device_key, due, fan_id, u16le};
use crate::fan::{Channel, FanHub, backend_on};
use crate::hid::{Driver, Io, Proto, open_io};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_fan::FanBackend;
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;
use std::time::Instant;

pub const VID: u16 = 0x264A;
pub const CHANNELS: usize = 5;
const SHORT: usize = 65;
const LONG: usize = 193;
const INIT: [u8; 3] = [0x00, 0xFE, 0x33];
const SET: u8 = 0x32;
const GET: u8 = 0x33;
const FAN: u8 = 0x51;
const LED: u8 = 0x52;
const MODE_PER_LED: u8 = 0x18;
const MODE_DIRECT: u8 = 0x24;
/// Riing Plus packets carry 20 LEDs per channel, LEDBOX packets the plugin's 54.
const PLUS_LEDS: usize = 20;
const LEDBOX_LEDS: usize = 54;
/// `BurstFans`: the speed every channel starts at; also what a released channel gets.
const START_PCT: u8 = 75;
const RPM_AT: usize = 5;
const POLL_MS: u64 = 3000;
const RESEND_MS: u64 = 3000;

/// Every pid the four plugins list, with its name.
pub const IDS: &[(u16, &str)] = &[
    (0x1FA4, "Thermaltake Floe Riing Controller"),
    (0x1FA5, "Thermaltake Floe Riing Controller"),
    (0x1FA6, "Thermaltake Floe Riing Controller"),
    (0x1FA7, "Thermaltake Floe Riing Controller"),
    (0x2135, "Thermaltake Riing Controller"),
    (0x2136, "Thermaltake Riing Controller"),
    (0x2137, "Thermaltake Riing Controller"),
    (0x2143, "Thermaltake Riing Controller"),
    (0x2144, "Thermaltake Riing Controller"),
    (0x213C, "Thermaltake LEDBOX"),
    (0x2260, "Thermaltake LEDBOX"),
    (0x2261, "Thermaltake LEDBOX"),
    (0x2262, "Thermaltake LEDBOX"),
    (0x2263, "Thermaltake LEDBOX"),
    (0x2264, "Thermaltake LEDBOX"),
    (0x2265, "Thermaltake LEDBOX"),
    (0x2266, "Thermaltake LEDBOX"),
    (0x2267, "Thermaltake LEDBOX"),
    (0x2268, "Thermaltake LEDBOX"),
    (0x226F, "Thermaltake LEDBOX"),
    (0x232B, "Thermaltake LEDBOX"),
    (0x232C, "Thermaltake LEDBOX"),
    (0x232D, "Thermaltake LEDBOX"),
    (0x232E, "Thermaltake LEDBOX"),
    (0x233A, "Thermaltake LEDBOX"),
    (0x2332, "Thermaltake LEDBOX"),
    (0x2339, "Thermaltake LEDBOX"),
];

/// Report length: the Quad/LEDBOX family takes 193 bytes, the rest 65.
pub fn report_len(pid: u16) -> usize {
    if (0x2260..=0x2270).contains(&pid) || (0x2320..=0x233F).contains(&pid) {
        LONG
    } else {
        SHORT
    }
}

/// `Validate`: interface 0, or none.
pub fn wants(info: &Info) -> bool {
    info.vid == VID
        && IDS.iter().any(|i| i.0 == info.pid)
        && (info.interface == 0 || info.interface == -1)
}

pub fn set_fan(channel: usize, pct: u8) -> [u8; 6] {
    [0x00, SET, FAN, channel as u8 + 1, 0x01, pct.min(100)]
}

pub fn get_fan(channel: usize) -> [u8; 4] {
    [0x00, GET, FAN, channel as u8 + 1]
}

/// RPM from a `33 51` reply (little-endian after the speed byte).
pub fn parse_rpm(reply: &[u8]) -> Option<u32> {
    u16le(reply, RPM_AT).map(u32::from)
}

/// `sendDirectPacket`: one channel's GRB colours after the mode byte.
pub fn led_packet(channel: usize, mode: u8, colors: &[Color]) -> Vec<u8> {
    let mut p = vec![0x00, SET, LED, channel as u8 + 1, mode];
    p.extend(colors.iter().flat_map(|c| [c.1, c.0, c.2]));
    p
}

struct Hub {
    io: Box<dyn Io>,
    id: String,
    name: String,
    len: usize,
    ready: bool,
    rpm: Vec<Cached<u32>>,
    duty: [Option<u8>; CHANNELS],
    sent: [Option<(u8, Instant)>; CHANNELS],
    /// The device's epoch at the last send: the lighting init may reset the fans.
    epoch: u32,
}

impl Hub {
    fn init(&mut self) -> Result<(), String> {
        if !self.ready {
            self.io.write(&INIT, self.len)?;
            let _ = self.io.read(self.len, 250);
            self.ready = true;
        }
        Ok(())
    }

    fn send(&mut self, channel: usize, pct: u8) -> Result<(), String> {
        let epoch = self.io.port().map_or(0, |p| p.device().epoch());
        if std::mem::replace(&mut self.epoch, epoch) != epoch {
            self.sent = [None; CHANNELS];
        }
        self.init()?;
        let fresh = self.sent[channel].is_some_and(|(p, at)| p == pct && !due(Some(at), RESEND_MS));
        if !fresh {
            ask(
                self.io.as_mut(),
                &set_fan(channel, pct),
                self.len,
                self.len,
                30,
                |_| true,
            )
            .ok();
            self.sent[channel] = Some((pct, Instant::now()));
        }
        Ok(())
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
        (1..=CHANNELS)
            .map(|i| Channel::fan(format!("Fan {i}")))
            .collect()
    }
    fn rpm(&mut self, channel: usize) -> Option<u32> {
        if channel >= CHANNELS || self.init().is_err() {
            return None;
        }
        let (io, len) = (self.io.as_mut(), self.len);
        let port = channel as u8 + 1;
        let rpm = self.rpm[channel].get(|| {
            let r = ask(io, &get_fan(channel), len, len, 60, |r| {
                r.len() > 6 && r[0] == GET && r[1] == FAN && r[2] == port
            })
            .ok()?;
            parse_rpm(&r)
        });
        if let Some(pct) = self.duty[channel] {
            self.send(channel, pct).ok();
        }
        rpm.filter(|&r| r > 0)
    }
    fn set_duty(&mut self, channel: usize, duty: f32) -> Result<(), String> {
        if channel >= CHANNELS {
            return Err("no such channel".into());
        }
        let pct = crate::fan::percent(duty);
        self.duty[channel] = Some(pct);
        self.send(channel, pct)
    }
    fn release(&mut self, channel: usize) -> Result<(), String> {
        if channel >= CHANNELS {
            return Ok(());
        }
        self.duty[channel] = None;
        self.sent[channel] = None;
        self.send(channel, START_PCT)
    }
}

pub fn hub(api: &HidApi, info: &Info) -> Option<Box<dyn FanBackend>> {
    let name = IDS.iter().find(|i| i.0 == info.pid)?.1;
    let io = open_io(api, info).ok()?;
    Some(backend_on(
        info,
        Hub {
            io,
            id: fan_id(info, "thermaltake"),
            name: name.into(),
            len: report_len(info.pid),
            ready: false,
            rpm: (0..CHANNELS).map(|_| Cached::new(POLL_MS / 3)).collect(),
            duty: [None; CHANNELS],
            sent: [None; CHANNELS],
            epoch: 0,
        },
    ))
}

pub fn key(info: &Info) -> String {
    device_key(info)
}

/// Lighting for the ids FoxRGB doesn't drive: Riing Plus 1FA4 and the newer LEDBOX pids.
struct Lights {
    len: usize,
    mode: u8,
    per: usize,
}

impl Proto for Lights {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.write(&INIT, self.len)?;
        let _ = io.read(self.len, 250);
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for ch in 0..CHANNELS {
            let p = led_packet(ch, self.mode, &colors[ch * self.per..(ch + 1) * self.per]);
            io.write(&p, self.len)?;
            let _ = io.read(self.len, 10);
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(1000)
    }
}

fn open_lights(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let io = open_io(api, info)?;
    let (mode, per) = if info.pid == 0x1FA4 {
        (MODE_PER_LED, PLUS_LEDS)
    } else {
        (MODE_DIRECT, LEDBOX_LEDS)
    };
    let dev = Device::grid(
        &info.device_id("thermaltake-xpack"),
        d.name,
        DeviceKind::Other,
        [0.34, 0.12, 0.24, 0.24],
        per as u32,
        CHANNELS as u32,
    );
    let len = report_len(info.pid);
    Ok(Driver::new(io, dev, "thermaltake", Lights { len, mode, per }).boxed())
}

const fn det(name: &'static str, pid: u16, interface: i32) -> Detector {
    crate::hid::on_i(name, VID, pid, interface, open_lights)
}

pub static DETECTORS: [Detector; 14] = [
    det("Thermaltake Floe Riing Controller", 0x1FA4, 0),
    det("Thermaltake Floe Riing Controller", 0x1FA4, -1),
    det("Thermaltake LEDBOX", 0x232C, 0),
    det("Thermaltake LEDBOX", 0x232C, -1),
    det("Thermaltake LEDBOX", 0x232D, 0),
    det("Thermaltake LEDBOX", 0x232D, -1),
    det("Thermaltake LEDBOX", 0x232E, 0),
    det("Thermaltake LEDBOX", 0x232E, -1),
    det("Thermaltake LEDBOX", 0x233A, 0),
    det("Thermaltake LEDBOX", 0x233A, -1),
    det("Thermaltake LEDBOX", 0x2332, 0),
    det("Thermaltake LEDBOX", 0x2332, -1),
    det("Thermaltake LEDBOX", 0x2339, 0),
    det("Thermaltake LEDBOX", 0x2339, -1),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_packets() {
        assert_eq!(set_fan(2, 40), [0x00, 0x32, 0x51, 0x03, 0x01, 40]);
        assert_eq!(set_fan(0, 250)[5], 100);
        assert_eq!(get_fan(4), [0x00, 0x33, 0x51, 0x05]);
    }

    #[test]
    fn rpm_from_reply() {
        let reply = [0x33, 0x51, 0x01, 0x00, 0x32, 0xDC, 0x05];
        assert_eq!(parse_rpm(&reply), Some(1500));
        assert_eq!(parse_rpm(&reply[..6]), None);
    }

    #[test]
    fn sizes_by_pid() {
        assert_eq!(report_len(0x2260), 193);
        assert_eq!(report_len(0x2339), 193);
        assert_eq!(report_len(0x1FA4), 65);
        assert_eq!(report_len(0x213C), 65);
        assert_eq!(report_len(0x2144), 65);
    }

    // The lighting's init can reset the fans, so a held speed goes out again after it.
    #[test]
    fn fans_resend_their_speed_after_the_lighting_init() {
        use crate::hid::wire::Wire;
        let wire = Wire::default();
        let port = wire.port();
        let mut hub = Hub {
            io: Box::new(port.clone()),
            id: "fan:t".into(),
            name: "T".into(),
            len: SHORT,
            ready: false,
            rpm: (0..CHANNELS).map(|_| Cached::new(60_000)).collect(),
            duty: [None; CHANNELS],
            sent: [None; CHANNELS],
            epoch: 0,
        };
        hub.set_duty(0, 0.4).unwrap();
        let dev = Device::grid("t", "T", DeviceKind::Other, [0.0; 4], 20, 5);
        let lights = Lights {
            len: SHORT,
            mode: MODE_PER_LED,
            per: PLUS_LEDS,
        };
        let mut lit = Driver::new(Box::new(port.clone()), dev, "t", lights);
        lit.push("t", &[Color(1, 2, 3); 100]).unwrap();
        hub.rpm[0].get(|| None);
        hub.rpm(0);
        let sent: Vec<Vec<u8>> = wire.take().iter().map(|p| p[..3].to_vec()).collect();
        let fan = vec![0x00, SET, FAN];
        let init = INIT.to_vec();
        assert_eq!(sent[..3], [init.clone(), fan.clone(), init]);
        assert_eq!(sent.last(), Some(&fan));
    }

    #[test]
    fn led_packet_is_grb() {
        let p = led_packet(1, MODE_DIRECT, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(p, vec![0x00, 0x32, 0x52, 0x02, 0x24, 2, 1, 3, 5, 4, 6]);
        const { assert!(5 + 3 * PLUS_LEDS <= SHORT) };
        const { assert!(5 + 3 * LEDBOX_LEDS <= LONG) };
    }
}
