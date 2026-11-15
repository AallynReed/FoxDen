//! Ported from OpenRGB Controllers/CorsairCommanderCoreController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use crate::port::Port;

use super::{NA, Op, Protocol, at, device, matrix, on_ipu, open_with};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1B1C;
const CORE: u16 = 0x0C1C;
const CORE2: u16 = 0x0C32;
const XT: u16 = 0x0C2A;
const FANS: usize = 6;
/// OpenRGB starts each fan header at 0 LEDs; 34 is its maximum and the stride of every header.
const FAN_LEDS: usize = 34;
const REPLY_MS: i32 = 500;

#[rustfmt::skip]
const MAP29: [u32; 49] = [
    28, NA, 27, NA, 26, NA, 25,
    NA, 16, NA, 15, NA, 14, NA,
    17, NA, 0, 5, 3, NA, 24,
    NA, 9, 4, 8, 6, 13, NA,
    18, NA, 1, 7, 2, NA, 23,
    NA, 10, NA, 11, NA, 12, NA,
    19, NA, 20, NA, 21, NA, 22,
];

#[rustfmt::skip]
const MAP24: [u32; 121] = [
    NA, NA, NA, NA, NA, 6, NA, NA, NA, NA, NA,
    NA, NA, NA, 4, 5, NA, 7, 8, NA, NA, NA,
    NA, NA, 3, NA, NA, NA, NA, NA, 9, NA, NA,
    NA, 2, NA, NA, NA, NA, NA, NA, NA, 10, NA,
    NA, 1, NA, NA, NA, NA, NA, NA, NA, 11, NA,
    0, NA, NA, NA, NA, NA, NA, NA, NA, NA, 12,
    NA, 23, NA, NA, NA, NA, NA, NA, NA, 13, NA,
    NA, 22, NA, NA, NA, NA, NA, NA, NA, 14, NA,
    NA, NA, 21, NA, NA, NA, NA, NA, 15, NA, NA,
    NA, NA, NA, 20, 19, NA, 17, 16, NA, NA, NA,
    NA, NA, NA, NA, NA, 18, NA, NA, NA, NA, NA,
];

/// Packet size before the firmware is known.
fn first_size(pid: u16) -> usize {
    match pid {
        CORE2 => 65,
        XT => 385,
        _ => 97,
    }
}

/// `SendCommand`'s buffer.
fn command(size: usize, cmd: [u8; 2], data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; size];
    b[1] = 0x08;
    b[2..4].copy_from_slice(&cmd);
    b[4..4 + data.len()].copy_from_slice(data);
    b
}

/// `WriteData`: open the endpoint, `06 00` with the length header, `07 00` for the rest, close.
fn write_data(size: usize, endpoint: [u8; 2], data_type: [u8; 2], data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![command(size, [0x0D, 0x00], &endpoint)];
    let first = data.len().min(size - 10);
    let len = (data.len() + 2) as u16;
    let mut head = vec![
        len as u8,
        (len >> 8) as u8,
        0,
        0,
        data_type[0],
        data_type[1],
    ];
    head.extend_from_slice(&data[..first]);
    out.push(command(size, [0x06, 0x00], &head));
    for chunk in data[first..].chunks(size - 4) {
        out.push(command(size, [0x07, 0x00], chunk));
    }
    out.push(command(size, [0x05, 0x01], &[]));
    out
}

/// `SetFanMode`'s 15-byte record.
fn fan_mode(pid: u16, external_rgb_port: bool) -> [u8; 15] {
    let mut b = [0u8; 15];
    b[0] = 0x07;
    let mut from = 3;
    if pid == XT {
        if external_rgb_port {
            b[1] = 0x01;
            b[2] = 0x01;
        } else {
            from = 2;
        }
    } else {
        b[1] = 0x01;
        b[2] = 0x08;
    }
    let to = if from == 2 { 14 } else { 15 };
    for pair in b[from..to].chunks_mut(2) {
        pair.copy_from_slice(&[0x01, 0x06]);
    }
    b
}

/// `GetLedCounts`, from the reply after its first three bytes.
fn led_counts(res: &[u8]) -> Vec<usize> {
    let n = res.get(2).copied().unwrap_or(0) as usize;
    (0..n)
        .map(|i| {
            let lo = res.get(i * 4 + 5).copied().unwrap_or(0) as usize;
            let hi = res.get(i * 4 + 6).copied().unwrap_or(0) as usize;
            hi << 8 | lo
        })
        .collect()
}

/// `SetDirectColor`'s data: the first zone packed, every later zone padded to 34 LEDs.
fn color_data(colors: &[Color], zones: &[usize]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut led = 0;
    for (z, &n) in zones.iter().enumerate() {
        for i in led..led + n {
            let c = at(colors, i);
            out.extend_from_slice(&[c.0, c.1, c.2]);
        }
        led += n;
        if z != 0 {
            out.resize(out.len() + 3 * FAN_LEDS.saturating_sub(n), 0);
        }
    }
    out
}

fn with_replies(size: usize, packets: Vec<Vec<u8>>) -> Vec<Op> {
    packets
        .into_iter()
        .flat_map(|p| [Op::Write(p), Op::Read(size, REPLY_MS)])
        .collect()
}

/// The pump, then each fan header at the user's length.
fn sized(id: &str, pump: usize) -> Vec<usize> {
    let fans = (1..=FANS).map(|i| {
        let name = format!("Fan {i}");
        crate::lengths::length(id, i, &name, FAN_LEDS, 0, FAN_LEDS)
    });
    std::iter::once(pump).chain(fans).collect()
}

struct Core {
    size: usize,
    zones: Vec<usize>,
}

impl Protocol for Core {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let data = color_data(colors, &self.zones);
        with_replies(
            self.size,
            write_data(self.size, [0x22, 0x00], [0x12, 0x00], &data),
        )
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    fn min_gap(&self) -> Duration {
        Duration::from_millis(33)
    }

    fn release(&mut self) -> Vec<Op> {
        with_replies(
            self.size,
            vec![command(self.size, [0x01, 0x03], &[0x00, 0x01])],
        )
    }

    fn release_waits(&self) -> bool {
        true
    }
}

/// Writes one command and reads until a reply starts with 0, as `SendCommand` does.
fn exchange(hid: &Port, packet: &[u8], size: usize) -> Result<Vec<u8>, String> {
    hid.write(packet).map_err(|e| e.to_string())?;
    for _ in 0..10 {
        let mut buf = vec![0u8; size];
        if hid
            .read_timeout(&mut buf, REPLY_MS)
            .map_err(|e| e.to_string())?
            > 0
            && buf[0] == 0
        {
            return Ok(buf);
        }
    }
    Err("no reply".into())
}

/// `InitController`'s firmware request.
fn firmware_request(size: usize) -> Vec<u8> {
    command(size, [0x02, 0x13], &[])
}

/// Packet size once the firmware is known.
fn size_after(pid: u16, size: usize, firmware_major: u8) -> usize {
    if pid == CORE && firmware_major == 1 {
        1025
    } else {
        size
    }
}

/// `SetFanMode(false)`, then `ReadData(20 00)` for `GetLedCounts` (its middle packet answers).
fn init_packets(size: usize, pid: u16) -> (Vec<Vec<u8>>, [Vec<u8>; 3]) {
    let mut fan = write_data(size, [0x1E, 0x00], [0x0D, 0x00], &fan_mode(pid, false));
    fan.push(command(size, [0x01, 0x03], &[0x00, 0x02]));
    let counts = [
        command(size, [0x0D, 0x00], &[0x20, 0x00]),
        command(size, [0x08, 0x00], &[]),
        command(size, [0x05, 0x01], &[]),
    ];
    (fan, counts)
}

fn handshake(hid: &Port, pid: u16) -> Result<(usize, Vec<usize>), String> {
    let first = first_size(pid);
    let firmware = exchange(hid, &firmware_request(first), first)?;
    let size = size_after(pid, first, firmware[3]);
    let (fan, [open, read, close]) = init_packets(size, pid);
    for p in fan {
        exchange(hid, &p, size)?;
    }
    exchange(hid, &open, size)?;
    let reply = exchange(hid, &read, size)?;
    exchange(hid, &close, size)?;
    Ok((size, led_counts(&reply[3..])))
}

fn layout(zones: &[usize]) -> Vec<[f32; 2]> {
    let pump = zones[0];
    let mut leds: Vec<[f32; 2]> = match pump {
        24 => matrix(&MAP24, 11, 24, |_| [0.5, 0.5]),
        _ => matrix(&MAP29, 7, pump, |_| [0.5, 0.5]),
    }
    .into_iter()
    .map(|[x, y]| [x * 0.3, y])
    .collect();
    for (f, &n) in zones[1..].iter().enumerate() {
        leds.extend((0..n).map(|i| {
            [
                0.35 + 0.65 * (i as f32 + 0.5) / n as f32,
                (f as f32 + 0.5) / FANS as f32,
            ]
        }));
    }
    leds
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "corsair commander core", |hid| {
        let (size, counts) = handshake(hid, info.pid)?;
        // The XT's first zone is the external header, which OpenRGB leaves at 0 LEDs.
        let pump = if info.pid == XT {
            0
        } else {
            *counts.first().ok_or("no pump")?
        };
        let id = info.device_id("corsair-commander-core");
        let zones = sized(&id, pump);
        let dev = device(
            id,
            d.name,
            DeviceKind::Other,
            [FAN_LEDS as u32, FANS as u32],
            layout(&zones),
        );
        Ok((dev, Core { size, zones }))
    })
}

pub(super) static DETECTORS: [Detector; 7] = [
    on_ipu(
        "Corsair Commander Core",
        VID,
        CORE,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
    on_ipu(
        "Corsair Commander Core",
        VID,
        CORE2,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
    on_ipu(
        "Corsair Commander Core",
        VID,
        0x0C1D,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
    on_ipu(
        "Corsair Commander Core",
        VID,
        0x0C3C,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
    on_ipu(
        "Corsair Commander Core",
        VID,
        0x0C3D,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
    on_ipu(
        "Corsair Commander Core",
        VID,
        0x0C3E,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
    on_ipu(
        "Corsair Commander Core XT",
        VID,
        XT,
        0x00,
        0xFF42,
        0x01,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    // SetFanMode(false) for the Core and the XT, SetFanMode(true) for the XT
    #[test]
    fn fan_mode_records() {
        assert_eq!(
            fan_mode(CORE, false),
            [7, 1, 8, 1, 6, 1, 6, 1, 6, 1, 6, 1, 6, 1, 6]
        );
        assert_eq!(
            fan_mode(XT, false),
            [7, 0, 1, 6, 1, 6, 1, 6, 1, 6, 1, 6, 1, 6, 0]
        );
        assert_eq!(
            fan_mode(XT, true),
            [7, 1, 1, 1, 6, 1, 6, 1, 6, 1, 6, 1, 6, 1, 6]
        );
    }

    // WriteData(1E 00, 0D 00, fan mode) on a 97-byte device
    #[test]
    fn write_data_short() {
        let packets = write_data(97, [0x1E, 0x00], [0x0D, 0x00], &fan_mode(CORE, false));
        assert_eq!(packets.len(), 3);
        assert_eq!(packets[0], buf(97, &[0x00, 0x08, 0x0D, 0x00, 0x1E, 0x00]));
        let mut second = buf(97, &[0x00, 0x08, 0x06, 0x00, 17, 0, 0, 0, 0x0D, 0x00]);
        second[10..25].copy_from_slice(&fan_mode(CORE, false));
        assert_eq!(packets[1], second);
        assert_eq!(packets[2], buf(97, &[0x00, 0x08, 0x05, 0x01]));
    }

    // InitController (firmware, size by PID and version), SetFanMode(false), GetLedCounts
    #[test]
    fn handshake_packets() {
        assert_eq!(firmware_request(97), buf(97, &[0x00, 0x08, 0x02, 0x13]));
        assert_eq!(first_size(CORE2), 65);
        assert_eq!(first_size(XT), 385);
        assert_eq!(size_after(CORE, 97, 1), 1025);
        assert_eq!(size_after(CORE, 97, 2), 97);
        assert_eq!(size_after(0x0C1D, 97, 1), 97);
        let (fan, counts) = init_packets(97, CORE);
        assert_eq!(
            fan[..3],
            write_data(97, [0x1E, 0x00], [0x0D, 0x00], &fan_mode(CORE, false))[..]
        );
        assert_eq!(fan[3], buf(97, &[0x00, 0x08, 0x01, 0x03, 0x00, 0x02]));
        assert_eq!(
            counts,
            [
                buf(97, &[0x00, 0x08, 0x0D, 0x00, 0x20, 0x00]),
                buf(97, &[0x00, 0x08, 0x08, 0x00]),
                buf(97, &[0x00, 0x08, 0x05, 0x01]),
            ]
        );
        let (fan_xt, _) = init_packets(385, XT);
        assert_eq!(fan_xt[1][10..25], fan_mode(XT, false));
        assert_eq!(fan_xt[1].len(), 385);
    }

    // GetLedCounts
    #[test]
    fn led_counts_from_reply() {
        let mut res = vec![0u8; 40];
        res[2] = 3;
        res[5] = 29;
        res[9] = 34;
        res[13] = 0x2C;
        res[14] = 0x01;
        assert_eq!(led_counts(&res), vec![29, 34, 300]);
    }

    // SetDirectColor: pump packed, fans padded to 34 with zeros (OpenRGB leaves them uninitialised)
    #[test]
    fn color_data_pads_fans() {
        let colors: Vec<Color> = (0..5).map(|i| Color(i, 10 + i, 20 + i)).collect();
        let data = color_data(&colors, &[2, 1, 2]);
        assert_eq!(data.len(), 3 * (2 + 34 + 34));
        assert_eq!(data[..6], [0, 10, 20, 1, 11, 21]);
        assert_eq!(data[6..9], [2, 12, 22]);
        assert!(data[9..6 + 102].iter().all(|&b| b == 0));
        assert_eq!(data[108..114], [3, 13, 23, 4, 14, 24]);
    }

    #[test]
    fn a_set_length_shortens_a_fan() {
        crate::lengths::set_one("hid:commander-core-test", 2, 8);
        crate::lengths::set_one("hid:commander-core-test", 6, 50);
        let zones = sized("hid:commander-core-test", 29);
        assert_eq!(zones, [29, 34, 8, 34, 34, 34, 34]);
        let colors = vec![Color(1, 1, 1); 207];
        assert_eq!(color_data(&colors, &zones).len(), 3 * (29 + 6 * 34));
    }

    // SetDirectColor -> WriteData(22 00, 12 00) for a 29-LED pump and six full fans
    #[test]
    fn frame_matches_set_direct_color() {
        let zones = vec![29, 34, 34, 34, 34, 34, 34];
        let colors: Vec<Color> = (0..233).map(|i| Color(i as u8, 1, 2)).collect();
        let mut core = Core { size: 97, zones };
        let ops = core.frame(&colors);
        // 699 bytes: 87 in the first data packet, then 93 per packet -> 7 more, plus open and close
        assert_eq!(ops.len(), 2 * (1 + 1 + 7 + 1));
        assert!(
            ops.iter()
                .skip(1)
                .step_by(2)
                .all(|o| *o == Op::Read(97, 500))
        );
        assert_eq!(
            bytes(&ops[0]),
            buf(97, &[0x00, 0x08, 0x0D, 0x00, 0x22, 0x00]).as_slice()
        );
        let first = bytes(&ops[2]);
        assert_eq!(
            first[..10],
            [0x00, 0x08, 0x06, 0x00, 0xBD, 0x02, 0, 0, 0x12, 0x00]
        );
        assert_eq!(first[10..16], [0, 1, 2, 1, 1, 2]);
        let next = bytes(&ops[4]);
        assert_eq!(next[..4], [0x00, 0x08, 0x07, 0x00]);
        assert_eq!(next[4..7], [29, 1, 2]);
        let last = bytes(&ops[16]);
        assert_eq!(last[..4], [0x00, 0x08, 0x07, 0x00]);
        assert_eq!(last[4..4 + 54], data_tail(&colors)[..]);
        assert!(last[58..].iter().all(|&b| b == 0));
        assert_eq!(
            bytes(&ops[18]),
            buf(97, &[0x00, 0x08, 0x05, 0x01]).as_slice()
        );
        assert_eq!(core.min_gap(), Duration::from_millis(33));
        assert_eq!(core.keepalive(), Some(Duration::from_secs(10)));
    }

    fn data_tail(colors: &[Color]) -> Vec<u8> {
        let data = color_data(colors, &[29, 34, 34, 34, 34, 34, 34]);
        data[699 - 54..].to_vec()
    }

    // ~CorsairCommanderCoreController: back to hardware mode
    #[test]
    fn release_sends_hardware_mode() {
        let ops = Core {
            size: 65,
            zones: vec![],
        }
        .release();
        assert_eq!(
            ops,
            vec![
                Op::Write(buf(65, &[0x00, 0x08, 0x01, 0x03, 0x00, 0x01])),
                Op::Read(65, 500)
            ]
        );
    }
}
