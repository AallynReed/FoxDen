// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::Duration;

use foxden_core::{Backend, Color, DeviceKind, Hardware, HardwareMode, HardwareSetting};
use hidapi::HidApi;

use super::{Op, Protocol, any, device, open_with, rows, span};
use crate::registry::{Detector, Info};

const VID: u16 = 0x264A;
const CHANNELS: usize = 5;
/// Headers report no size; these are the maximums.
const RIING_LEDS: usize = 20;
const QUAD_LEDS: usize = 60;
const TRIO_LEDS: usize = 54;
const MODE_PER_LED: u8 = 0x18;
const MODE_DIRECT: u8 = 0x24;
const QUAD_SIZE: usize = 193;
const TRIO_CHUNK: usize = 19;

/// `SendInit` of every Riing controller, padded to its packet size.
fn init(size: usize) -> Vec<u8> {
    let mut b = vec![0u8; size];
    b[1] = 0xFE;
    b[2] = 0x33;
    b
}

fn grb(colors: &[Color]) -> Vec<u8> {
    colors.iter().flat_map(|c| [c.1, c.0, c.2]).collect()
}

/// `ThermaltakeRiingController::SendRGB`.
fn riing_rgb(port: u8, mode: u8, speed: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = 0x32;
    b[2] = 0x52;
    b[3] = port;
    b[4] = mode.wrapping_add(speed & 0x03);
    let data = grb(colors);
    b[5..5 + data.len()].copy_from_slice(&data);
    b
}

/// Mode id, name, mode byte, colours, speed; colours fill every LED.
const EFFECTS: [(&str, &str, u8, usize, bool); 7] = [
    ("static", "Static", 0x19, 1, false),
    ("rainbow", "Rainbow", 0x00, 0, true),
    ("color_cycle", "Colour cycle", 0x04, 0, true),
    ("ripple", "Ripple", 0x08, 1, true),
    ("flashing", "Flashing", 0x0C, 1, true),
    ("breathing", "Breathing", 0x10, 1, true),
    ("wave", "Wave", 0x14, 1, true),
];

fn riing_hardware() -> Hardware {
    let modes = EFFECTS.map(|(id, name, _, colors, speed)| HardwareMode {
        speed: speed.then_some([0, 3]),
        colors,
        ..HardwareMode::new(id, name)
    });
    Hardware {
        modes: modes.to_vec(),
    }
}

/// Init, then each port in the effect; the device counts speed down, 3 being the slowest.
fn riing_effect(counts: &[usize], s: &HardwareSetting) -> Result<Vec<Op>, String> {
    let (_, f) = riing_hardware().fit(s)?;
    let &(_, _, mode, _, timed) = EFFECTS
        .iter()
        .find(|e| e.0 == s.mode)
        .ok_or("unknown mode")?;
    let speed = if timed { 3 - f.speed } else { 0 };
    let c = f.colors.first().copied().unwrap_or(Color::BLACK);
    let mut ops = vec![Op::Write(init(65)), Op::Read(65, 100)];
    for (ch, &n) in counts.iter().enumerate() {
        ops.push(Op::Write(riing_rgb(ch as u8 + 1, mode, speed, &vec![c; n])));
        ops.push(Op::Read(65, 100));
    }
    Ok(ops)
}

/// `ThermaltakeRiingQuadController::SetChannelLEDs` in direct mode.
fn quad_channel(channel: u8, colors: &[Color]) -> Vec<u8> {
    let mut b = vec![0u8; QUAD_SIZE];
    b[1] = 0x32;
    b[2] = 0x52;
    b[3] = channel + 1;
    b[4] = MODE_DIRECT;
    let data = grb(colors);
    b[5..5 + data.len()].copy_from_slice(&data);
    b
}

/// `ThermaltakeRiingTrioController::SetChannelLEDs`: at least two chunks of 19, none for no LEDs.
fn trio_channel(channel: u8, colors: &[Color]) -> Vec<Vec<u8>> {
    if colors.is_empty() {
        return Vec::new();
    }
    let data = grb(colors);
    let mut out = Vec::new();
    let mut i = 0;
    while colors.len() > TRIO_CHUNK * i || i < 2 {
        let mut b = vec![0u8; 65];
        b[1] = 0x32;
        b[2] = 0x52;
        b[3] = channel + 1;
        b[4] = 0x24;
        b[5] = 0x03;
        b[6] = i as u8 + 1;
        let from = (TRIO_CHUNK * i * 3).min(data.len());
        let to = (from + TRIO_CHUNK * 3).min(data.len());
        b[8..8 + to - from].copy_from_slice(&data[from..to]);
        out.push(b);
        i += 1;
    }
    out
}

#[derive(Clone, Copy)]
enum Kind {
    Riing,
    Quad,
    Trio,
}

impl Kind {
    fn leds(self) -> usize {
        match self {
            Kind::Riing => RIING_LEDS,
            Kind::Quad => QUAD_LEDS,
            Kind::Trio => TRIO_LEDS,
        }
    }
}

/// Every header at the user's length, up to the controller's maximum.
fn sized(id: &str, kind: Kind) -> [usize; CHANNELS] {
    std::array::from_fn(|i| {
        let name = format!("Header {}", i + 1);
        crate::lengths::length(id, i, &name, kind.leds(), 0, kind.leds())
    })
}

struct Riing {
    kind: Kind,
    counts: [usize; CHANNELS],
}

impl Protocol for Riing {
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        match self.kind {
            Kind::Riing => vec![Op::Write(init(65)), Op::Read(65, 100)],
            Kind::Quad => vec![Op::Write(init(QUAD_SIZE)), Op::Read(QUAD_SIZE, 250)],
            Kind::Trio => vec![Op::Write(init(65)), Op::Read(65, 250)],
        }
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        let mut from = 0;
        for (ch, &n) in self.counts.iter().enumerate() {
            let zone = span(colors, from, n);
            from += n;
            match self.kind {
                Kind::Riing => {
                    ops.push(Op::Write(riing_rgb(ch as u8 + 1, MODE_PER_LED, 0, &zone)));
                    ops.push(Op::Read(65, 100));
                }
                Kind::Quad => ops.push(Op::Write(quad_channel(ch as u8, &zone))),
                Kind::Trio => ops.extend(trio_channel(ch as u8, &zone).into_iter().map(Op::Write)),
            }
        }
        ops
    }

    fn keepalive(&self) -> Option<Duration> {
        matches!(self.kind, Kind::Quad).then_some(Duration::from_secs(3))
    }

    fn hardware(&self) -> Option<Hardware> {
        matches!(self.kind, Kind::Riing).then(riing_hardware)
    }

    fn hardware_ops(&mut self, s: &HardwareSetting) -> Result<Vec<Op>, String> {
        match self.kind {
            Kind::Riing => riing_effect(&self.counts, s),
            _ => Err("no hardware effects".into()),
        }
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector, kind: Kind) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "thermaltake riing", |_| {
        let id = info.device_id("thermaltake-riing");
        let counts = sized(&id, kind);
        let (leds, grid) = rows(&counts);
        let dev = device(id, d.name, DeviceKind::Other, grid, leds);
        Ok((dev, Riing { kind, counts }))
    })
}

fn open_riing(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Kind::Riing)
}

fn open_quad(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Kind::Quad)
}

fn open_trio(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open(api, info, d, Kind::Trio)
}

macro_rules! riing {
    ($($name:literal $pid:literal $open:ident),* $(,)?) => {
        pub(super) static DETECTORS: [Detector; 51] = [$(any($name, VID, $pid, $open)),*];
    };
}

riing! {
    "Thermaltake Riing (PID 0x1FA5)" 0x1FA5 open_riing,
    "Thermaltake Riing (PID 0x1FA6)" 0x1FA6 open_riing,
    "Thermaltake Riing (PID 0x1FA7)" 0x1FA7 open_riing,
    "Thermaltake Riing (PID 0x1FA8)" 0x1FA8 open_riing,
    "Thermaltake Riing (PID 0x1FA9)" 0x1FA9 open_riing,
    "Thermaltake Riing (PID 0x1FAA)" 0x1FAA open_riing,
    "Thermaltake Riing (PID 0x1FAB)" 0x1FAB open_riing,
    "Thermaltake Riing (PID 0x1FAC)" 0x1FAC open_riing,
    "Thermaltake Riing (PID 0x1FAD)" 0x1FAD open_riing,
    "Thermaltake Riing (PID 0x1FAE)" 0x1FAE open_riing,
    "Thermaltake Riing (PID 0x1FAF)" 0x1FAF open_riing,
    "Thermaltake Riing (PID 0x1FB0)" 0x1FB0 open_riing,
    "Thermaltake Riing (PID 0x1FB1)" 0x1FB1 open_riing,
    "Thermaltake Riing (PID 0x1FB2)" 0x1FB2 open_riing,
    "Thermaltake Riing (PID 0x1FB3)" 0x1FB3 open_riing,
    "Thermaltake Riing (PID 0x1FB4)" 0x1FB4 open_riing,
    "Thermaltake Riing (PID 0x1FB5)" 0x1FB5 open_riing,
    "Thermaltake Riing Quad (PID 0x2260)" 0x2260 open_quad,
    "Thermaltake Riing Quad (PID 0x2261)" 0x2261 open_quad,
    "Thermaltake Riing Quad (PID 0x2262)" 0x2262 open_quad,
    "Thermaltake Riing Quad (PID 0x2263)" 0x2263 open_quad,
    "Thermaltake Riing Quad (PID 0x2264)" 0x2264 open_quad,
    "Thermaltake Riing Quad (PID 0x2265)" 0x2265 open_quad,
    "Thermaltake Riing Quad (PID 0x2266)" 0x2266 open_quad,
    "Thermaltake Riing Quad (PID 0x2267)" 0x2267 open_quad,
    "Thermaltake Riing Quad (PID 0x2268)" 0x2268 open_quad,
    "Thermaltake Riing Quad (PID 0x2269)" 0x2269 open_quad,
    "Thermaltake Riing Quad (PID 0x226A)" 0x226A open_quad,
    "Thermaltake Riing Quad (PID 0x226B)" 0x226B open_quad,
    "Thermaltake Riing Quad (PID 0x226C)" 0x226C open_quad,
    "Thermaltake Riing Quad (PID 0x226D)" 0x226D open_quad,
    "Thermaltake Riing Quad (PID 0x226E)" 0x226E open_quad,
    "Thermaltake Riing Quad (PID 0x226F)" 0x226F open_quad,
    "Thermaltake Riing Quad (PID 0x2270)" 0x2270 open_quad,
    "Thermaltake Riing Quad (PID 0x232B)" 0x232B open_quad,
    "Thermaltake Riing Trio (PID 0x2135)" 0x2135 open_trio,
    "Thermaltake Riing Trio (PID 0x2136)" 0x2136 open_trio,
    "Thermaltake Riing Trio (PID 0x2137)" 0x2137 open_trio,
    "Thermaltake Riing Trio (PID 0x2138)" 0x2138 open_trio,
    "Thermaltake Riing Trio (PID 0x2139)" 0x2139 open_trio,
    "Thermaltake Riing Trio (PID 0x213A)" 0x213A open_trio,
    "Thermaltake Riing Trio (PID 0x213B)" 0x213B open_trio,
    "Thermaltake Riing Trio (PID 0x213C)" 0x213C open_trio,
    "Thermaltake Riing Trio (PID 0x213D)" 0x213D open_trio,
    "Thermaltake Riing Trio (PID 0x213E)" 0x213E open_trio,
    "Thermaltake Riing Trio (PID 0x213F)" 0x213F open_trio,
    "Thermaltake Riing Trio (PID 0x2141)" 0x2141 open_trio,
    "Thermaltake Riing Trio (PID 0x2142)" 0x2142 open_trio,
    "Thermaltake Riing Trio (PID 0x2143)" 0x2143 open_trio,
    "Thermaltake Riing Trio (PID 0x2144)" 0x2144 open_trio,
    "Thermaltake Riing Trio (PID 0x2145)" 0x2145 open_trio,
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    fn ramp(n: usize) -> Vec<Color> {
        (0..n)
            .map(|i| Color(i as u8, (100 + i) as u8, (300 - i) as u8))
            .collect()
    }

    fn full(kind: Kind) -> Riing {
        Riing {
            kind,
            counts: [kind.leds(); CHANNELS],
        }
    }

    #[test]
    fn a_set_length_shortens_a_header() {
        crate::lengths::set_one("hid:riing-test", 4, 7);
        let counts = sized("hid:riing-test", Kind::Quad);
        assert_eq!(counts, [60, 60, 60, 60, 7]);
        let colors = ramp(247);
        let ops = Riing {
            kind: Kind::Quad,
            counts,
        }
        .frame(&colors);
        let p = bytes(&ops[4]);
        assert_eq!(p[5..8], [colors[240].1, colors[240].0, colors[240].2]);
        assert!(p[5 + 7 * 3..].iter().all(|&b| b == 0));
    }

    // ThermaltakeRiingController::SendInit
    #[test]
    fn riing_setup_is_init_then_read() {
        let ops = full(Kind::Riing).setup(&[]);
        assert_eq!(
            ops,
            vec![Op::Write(buf(65, &[0x00, 0xFE, 0x33])), Op::Read(65, 100)]
        );
    }

    // ThermaltakeRiingController::SetChannelLEDs + SendRGB with the Direct mode (0x18, speed 0)
    #[test]
    fn riing_frame_sends_grb_per_port() {
        let colors = ramp(CHANNELS * RIING_LEDS);
        let ops = full(Kind::Riing).frame(&colors);
        assert_eq!(ops.len(), 10);
        for ch in 0..CHANNELS {
            assert_eq!(ops[ch * 2 + 1], Op::Read(65, 100));
            let mut want = buf(65, &[0x00, 0x32, 0x52, ch as u8 + 1, 0x18]);
            for i in 0..RIING_LEDS {
                let c = colors[ch * RIING_LEDS + i];
                want[5 + i * 3..8 + i * 3].copy_from_slice(&[c.1, c.0, c.2]);
            }
            assert_eq!(bytes(&ops[ch * 2]), want.as_slice());
        }
    }

    // ThermaltakeRiingController::SendRGB: mode + (speed & 3)
    #[test]
    fn riing_mode_byte_adds_speed() {
        assert_eq!(riing_rgb(2, 0x14, 0x07, &[])[4], 0x17);
    }

    // ThermaltakeRiingQuadController::SendInit
    #[test]
    fn quad_setup() {
        let ops = full(Kind::Quad).setup(&[]);
        assert_eq!(
            ops,
            vec![Op::Write(buf(193, &[0x00, 0xFE, 0x33])), Op::Read(193, 250)]
        );
    }

    // ThermaltakeRiingQuadController::SetChannelLEDs with Direct (0x24), 193-byte buffers
    #[test]
    fn quad_frame_and_keepalive() {
        let colors = ramp(CHANNELS * QUAD_LEDS);
        let mut q = full(Kind::Quad);
        let ops = q.frame(&colors);
        assert_eq!(ops.len(), 5);
        let p = bytes(&ops[4]);
        assert_eq!(p.len(), 193);
        assert_eq!(p[..5], [0x00, 0x32, 0x52, 0x05, 0x24]);
        let first = colors[4 * QUAD_LEDS];
        assert_eq!(p[5..8], [first.1, first.0, first.2]);
        let last = colors[5 * QUAD_LEDS - 1];
        assert_eq!(p[5 + 59 * 3..5 + 60 * 3], [last.1, last.0, last.2]);
        assert!(p[185..].iter().all(|&b| b == 0));
        // KeepaliveThread resends all five buffers every 3 s
        assert_eq!(q.keepalive(), Some(Duration::from_secs(3)));
        assert_eq!(q.refresh(&colors), ops);
    }

    // ThermaltakeRiingTrioController::SetChannelLEDs: 54 LEDs make three chunks of 19
    #[test]
    fn trio_chunks() {
        let colors = ramp(TRIO_LEDS);
        let packets = trio_channel(2, &colors);
        assert_eq!(packets.len(), 3);
        for (i, p) in packets.iter().enumerate() {
            assert_eq!(p.len(), 65);
            assert_eq!(
                p[..8],
                [0x00, 0x32, 0x52, 0x03, 0x24, 0x03, i as u8 + 1, 0x00]
            );
        }
        let c = colors[19];
        assert_eq!(packets[1][8..11], [c.1, c.0, c.2]);
        let c = colors[53];
        assert_eq!(packets[2][8 + 15 * 3..8 + 16 * 3], [c.1, c.0, c.2]);
        assert!(packets[2][8 + 16 * 3..].iter().all(|&b| b == 0));
    }

    // ThermaltakeRiingTrioController::SetChannelLEDs: `|| i < 2` and the early return
    #[test]
    fn trio_small_and_empty() {
        let packets = trio_channel(0, &[Color(1, 2, 3)]);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0][8..11], [2, 1, 3]);
        assert_eq!(
            packets[1],
            buf(65, &[0x00, 0x32, 0x52, 0x01, 0x24, 0x03, 0x02])
        );
        assert!(trio_channel(0, &[]).is_empty());
        assert_eq!(full(Kind::Trio).frame(&ramp(270)).len(), 15);
    }

    #[test]
    fn riing_effects() {
        let mut r = full(Kind::Riing);
        r.counts = [2, 0, 1, 1, 1];
        let mut s = HardwareSetting::new("breathing");
        s.speed = Some(3);
        s.colors = vec![Color(1, 2, 3)];
        let ops = r.hardware_ops(&s).unwrap();
        assert_eq!(ops.len(), 12);
        assert_eq!(ops[..2], r.setup(&[]));
        assert_eq!(
            bytes(&ops[2]),
            buf(65, &[0x00, 0x32, 0x52, 0x01, 0x10, 2, 1, 3, 2, 1, 3]).as_slice()
        );
        assert_eq!(ops[3], Op::Read(65, 100));
        assert_eq!(
            bytes(&ops[4]),
            buf(65, &[0x00, 0x32, 0x52, 0x02, 0x10]).as_slice()
        );
        s.speed = Some(0);
        assert_eq!(bytes(&r.hardware_ops(&s).unwrap()[2])[4], 0x13);
        let mut st = HardwareSetting::new("static");
        st.speed = Some(2);
        st.colors = vec![Color(9, 8, 7)];
        assert_eq!(
            bytes(&r.hardware_ops(&st).unwrap()[2])[4..8],
            [0x19, 8, 9, 7]
        );
        let flow = r.hardware_ops(&HardwareSetting::new("rainbow")).unwrap();
        assert_eq!(bytes(&flow[2])[4..8], [0x02, 0, 0, 0]);
        let modes: Vec<u8> = EFFECTS.iter().map(|e| e.2).collect();
        assert_eq!(modes, [0x19, 0x00, 0x04, 0x08, 0x0C, 0x10, 0x14]);
        assert!(r.hardware_ops(&HardwareSetting::new("direct")).is_err());
        assert!(full(Kind::Quad).hardware().is_none());
        assert!(full(Kind::Trio).hardware_ops(&st).is_err());
        assert_eq!(
            r.frame(&ramp(5))[0],
            Op::Write(riing_rgb(1, 0x18, 0, &ramp(2)))
        );
    }
}
