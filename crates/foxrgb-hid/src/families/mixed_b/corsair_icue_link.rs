//! Ported from OpenRGB Controllers/CorsairICueLinkController (GPL-2.0-or-later).

use std::time::Duration;

use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::{HidApi, HidDevice};

use super::{Op, Protocol, at, device, on_ipu, open_with, rows};
use crate::registry::{Detector, Info};

const WRITE_LEN: usize = 513;
const READ_LEN: usize = 512;
const CHUNK: usize = 508;
const READ: Op = Op::Read(READ_LEN, 1000);
const CLOSE: [u8; 3] = [0x05, 0x01, 0x01];
const COLOR_ENDPOINT: u8 = 0x22;
const DEVICES_ENDPOINT: u8 = 0x36;

/// (type, model, LEDs, internal) from `known_devices`.
const KNOWN: [(u8, u8, usize, bool); 30] = [
    (0x05, 0x02, 160, false),
    (0x05, 0x01, 22, false),
    (0x05, 0x00, 0, false),
    (0x06, 0x00, 24, false),
    (0x11, 0x00, 20, false),
    (0x11, 0x04, 20, false),
    (0x07, 0x00, 20, false),
    (0x07, 0x04, 20, false),
    (0x11, 0x01, 20, false),
    (0x07, 0x01, 20, false),
    (0x11, 0x02, 20, false),
    (0x11, 0x05, 20, false),
    (0x07, 0x02, 20, false),
    (0x07, 0x05, 20, false),
    (0x11, 0x03, 20, false),
    (0x07, 0x03, 20, false),
    (0x10, 0x00, 0, false),
    (0x02, 0x00, 18, false),
    (0x14, 0x00, 0, false),
    (0x01, 0x00, 34, false),
    (0x13, 0x00, 0, false),
    (0x04, 0x00, 0, false),
    (0x0F, 0x00, 8, false),
    (0x03, 0x00, 8, false),
    (0x09, 0x00, 24, false),
    (0x0C, 0x00, 22, false),
    (0x0E, 0x00, 22, true),
    (0x19, 0x00, 22, false),
    (0x0A, 0x00, 0, false),
    (0x0D, 0x00, 16, false),
];

/// `SendCommand`'s 513-byte buffer.
fn command(cmd: &[u8], data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; WRITE_LEN];
    b[2] = 0x01;
    b[3..3 + cmd.len()].copy_from_slice(cmd);
    b[3 + cmd.len()..3 + cmd.len() + data.len()].copy_from_slice(data);
    b
}

/// `GetControllerDevices`: (type, LEDs) of every lit endpoint, from the `08 01` read.
fn endpoints(reply: &[u8]) -> Vec<(u8, usize)> {
    let channels = reply.get(6).copied().unwrap_or(0) as usize;
    let index = reply.get(7..).unwrap_or(&[]);
    let byte = |i: usize| index.get(i).copied().unwrap_or(0);
    let mut pos = 0;
    let mut out = Vec::new();
    for _ in 0..channels {
        let id_len = byte(pos + 7) as usize;
        if id_len == 0 {
            pos += 8;
            continue;
        }
        let (kind, model) = (byte(pos + 2), byte(pos + 3));
        if let Some(&(_, _, leds, internal)) = KNOWN.iter().find(|k| k.0 == kind && k.1 == model)
            && !internal
            && leds > 0
        {
            out.push((kind, leds));
        }
        pos += 8 + id_len;
    }
    out
}

/// `RGBController_CorsairICueLink::SetupZones`: pump LCD rings follow each AIO.
fn zones(endpoints: &[(u8, usize)]) -> Vec<usize> {
    let mut out = Vec::new();
    for &(kind, leds) in endpoints {
        if kind == 0x06 {
            continue;
        }
        out.push(leds);
        if kind == 0x07 || kind == 0x11 {
            out.extend(endpoints.iter().filter(|e| e.0 == 0x06).map(|e| e.1));
        }
    }
    out
}

/// `UpdateLights` -> `Write` on the colour endpoint.
fn update_lights(colors: &[Color], leds: usize) -> Vec<Op> {
    let data: Vec<u8> = (0..leds)
        .flat_map(|i| {
            let c = at(colors, i);
            [c.0, c.1, c.2]
        })
        .collect();
    let len = (data.len() + 2) as u16;
    let mut buf = vec![len as u8, (len >> 8) as u8, 0, 0, 0x12, 0x00];
    buf.extend(data);
    let mut packets = vec![
        command(&CLOSE, &[COLOR_ENDPOINT]),
        command(&[0x0D, 0x00], &[COLOR_ENDPOINT]),
    ];
    for (i, chunk) in buf.chunks(CHUNK).enumerate() {
        let cmd: [u8; 2] = if i == 0 { [0x06, 0x00] } else { [0x07, 0x00] };
        packets.push(command(&cmd, chunk));
    }
    packets.push(command(&CLOSE, &[COLOR_ENDPOINT]));
    packets
        .into_iter()
        .flat_map(|p| [Op::Write(p), READ])
        .collect()
}

struct Link {
    leds: usize,
}

impl Protocol for Link {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        update_lights(colors, self.leds)
    }

    fn keepalive(&self) -> Option<Duration> {
        Some(Duration::from_secs(5))
    }

    fn release(&mut self) -> Vec<Op> {
        vec![Op::Write(command(&[0x01, 0x03, 0x00, 0x01], &[])), READ]
    }
}

fn send(hid: &HidDevice, packet: &[u8]) -> Result<Vec<u8>, String> {
    hid.write(packet).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; READ_LEN];
    hid.read_timeout(&mut buf, 1000)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// The constructor: firmware, software mode, then `Read` of the device list (index 4 answers).
fn handshake_packets() -> [Vec<u8>; 6] {
    [
        command(&[0x02, 0x13], &[]),
        command(&[0x01, 0x03, 0x00, 0x02], &[]),
        command(&CLOSE, &[DEVICES_ENDPOINT]),
        command(&[0x0D, 0x01], &[DEVICES_ENDPOINT]),
        command(&[0x08, 0x01], &[]),
        command(&CLOSE, &[DEVICES_ENDPOINT]),
    ]
}

fn handshake(hid: &HidDevice) -> Result<Vec<usize>, String> {
    let [firmware, software, close, open, read, close_again] = handshake_packets();
    for p in [firmware, software, close, open] {
        send(hid, &p)?;
    }
    let mut reply = send(hid, &read)?;
    for _ in 0..5 {
        if reply[4] == 0x21 {
            break;
        }
        reply.fill(0);
        hid.read_timeout(&mut reply, 1000)
            .map_err(|e| e.to_string())?;
    }
    send(hid, &close_again)?;
    Ok(zones(&endpoints(&reply)))
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "corsair icue link", |hid| {
        let zones = handshake(hid)?;
        let leds: usize = zones.iter().sum();
        if leds == 0 {
            return Err("no lit devices".into());
        }
        let (pos, grid) = rows(&zones);
        let dev = device(
            info.device_id("corsair-icue-link"),
            d.name,
            DeviceKind::Other,
            grid,
            pos,
        );
        Ok((dev, Link { leds }))
    })
}

pub(super) static DETECTORS: [Detector; 1] = [on_ipu(
    "Corsair iCUE Link System Hub",
    0x1B1C,
    0x0C3F,
    0x00,
    0xFF42,
    0x01,
    open,
)];

#[cfg(test)]
mod tests {
    use super::super::test_util::{buf, bytes};
    use super::*;

    fn entry(kind: u8, model: u8, id: &[u8]) -> Vec<u8> {
        let mut e = vec![0, 0, kind, model, 0, 0, 0, id.len() as u8];
        e.extend_from_slice(id);
        e
    }

    // GetControllerFirmware, SetControllerSoftwareMode, Read(GET_DEVICES)
    #[test]
    fn handshake_requests() {
        let p = handshake_packets();
        assert_eq!(p[0], buf(513, &[0, 0, 1, 0x02, 0x13]));
        assert_eq!(p[1], buf(513, &[0, 0, 1, 0x01, 0x03, 0x00, 0x02]));
        assert_eq!(p[2], buf(513, &[0, 0, 1, 0x05, 0x01, 0x01, 0x36]));
        assert_eq!(p[3], buf(513, &[0, 0, 1, 0x0D, 0x01, 0x36]));
        assert_eq!(p[4], buf(513, &[0, 0, 1, 0x08, 0x01]));
        assert_eq!(p[5], p[2]);
    }

    // GetControllerDevices + SetupZones
    #[test]
    fn device_list_to_zones() {
        let mut reply = vec![0u8; 7];
        reply[4] = 0x21;
        reply[6] = 6;
        reply.extend(entry(0x06, 0x00, b"lcd"));
        reply.extend(entry(0x01, 0x00, b"qx-fan-1"));
        reply.extend([0u8; 8]); // empty port
        reply.extend(entry(0x07, 0x02, b"h150i"));
        reply.extend(entry(0x0E, 0x00, b"xd5")); // internal
        reply.extend(entry(0x7F, 0x7F, b"?")); // unknown
        let eps = endpoints(&reply);
        assert_eq!(eps, vec![(0x06, 24), (0x01, 34), (0x07, 20)]);
        assert_eq!(zones(&eps), vec![34, 20, 24]);
    }

    // UpdateLights -> Write(SET_COLOR, CORSAIR_ICUE_ENDPOINT_TYPE_COLOR)
    #[test]
    fn frame_writes_colour_endpoint() {
        let colors: Vec<Color> = (0..200).map(|i| Color(i as u8, 7, 9)).collect();
        let ops = Link { leds: 200 }.frame(&colors);
        // 6 + 600 bytes -> chunks of 508 and 98
        assert_eq!(ops.len(), 2 * 5);
        assert!(
            ops.iter()
                .skip(1)
                .step_by(2)
                .all(|o| *o == Op::Read(512, 1000))
        );
        assert_eq!(
            bytes(&ops[0]),
            buf(513, &[0, 0, 1, 0x05, 0x01, 0x01, 0x22]).as_slice()
        );
        assert_eq!(
            bytes(&ops[2]),
            buf(513, &[0, 0, 1, 0x0D, 0x00, 0x22]).as_slice()
        );
        let first = bytes(&ops[4]);
        assert_eq!(
            first[..14],
            [0, 0, 1, 0x06, 0x00, 0x5A, 0x02, 0, 0, 0x12, 0x00, 0, 7, 9]
        );
        assert_eq!(first[510..513], [7, 9, 167]);
        let second = bytes(&ops[6]);
        assert_eq!(second[..5], [0, 0, 1, 0x07, 0x00]);
        assert_eq!(second[5..8], [7, 9, 168]);
        assert_eq!(second[5 + 98 - 3..5 + 98], [199, 7, 9]);
        assert!(second[5 + 98..].iter().all(|&b| b == 0));
        assert_eq!(bytes(&ops[8]), bytes(&ops[0]));
    }

    // ~CorsairICueLinkController: SetControllerHardwareMode
    #[test]
    fn release_restores_hardware_mode() {
        let ops = Link { leds: 1 }.release();
        assert_eq!(
            ops,
            vec![
                Op::Write(buf(513, &[0, 0, 1, 0x01, 0x03, 0x00, 0x01])),
                READ
            ]
        );
    }
}
