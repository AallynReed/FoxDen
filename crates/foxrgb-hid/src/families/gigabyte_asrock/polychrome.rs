//! Ported from OpenRGB Controllers/ASRockPolychromeUSBController (GPL-2.0-or-later).

use super::{Driver, Op, Plan, rows};
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

const VID: u16 = 0x26CE;

pub static DETECTORS: [Detector; 2] = [
    Detector {
        name: "ASRock Polychrome USB",
        vid: VID,
        pid: 0x01A2,
        interface: None,
        usage_page: None,
        usage: None,
        open,
    },
    Detector {
        name: "ASRock Deskmini Addressable LED Strip",
        vid: VID,
        pid: 0x01A6,
        interface: None,
        usage_page: None,
        usage: None,
        open,
    },
];

const READ_HEADER: u8 = 0x14;
const WRITE_HEADER: u8 = 0x15;
const SET_ZONE: u8 = 0x10;
const INITIAL_CHUNK: u8 = 0xE3;
const SEND_CHUNK: u8 = 0xE4;
const LEDCOUNT_CFG: u8 = 0x02;
const RGSWAP_CFG: u8 = 0x03;
const ZONE_UNAVAILABLE: u8 = 0x1E;
const MODE_DIRECT: u8 = 0x0F;
const ADDRESSABLE_MAX_LEDS: usize = 100;
const READ_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub zone_type: u8,
    pub leds: usize,
    pub leds_max: usize,
    pub rgswap: bool,
}

/// READ_HEADER request; OpenRGB writes 65 bytes except for header 1, which it writes as 64.
pub fn read_header(cfg: u8, len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[1] = READ_HEADER;
    b[3] = cfg;
    b
}

fn addressable(zone_type: u8) -> bool {
    matches!(zone_type, 2 | 3 | 7)
}

/// Each addressable header at the user's length; the LED-count table for SetZoneSize if one changed.
pub fn sized(id: &str, zones: &mut [Zone], counts: [u8; 8]) -> Option<[u8; 8]> {
    let mut table = counts;
    for (i, z) in zones.iter_mut().enumerate() {
        if addressable(z.zone_type) {
            let name = ZONE_NAMES[z.zone_type as usize];
            let n = crate::lengths::length(id, i, name, z.leds, 0, z.leds_max);
            // 30 in the table would mark the header unavailable
            if n != ZONE_UNAVAILABLE as usize {
                z.leds = n;
            }
            table[z.zone_type as usize] = z.leds as u8;
        }
    }
    (table != counts).then_some(table)
}

/// SetDeviceInfo: one zone per present channel, from the three config tables.
pub fn zones(max_leds: [u8; 8], rgswap: u8, header1: u8) -> Vec<Zone> {
    (0..8u8)
        .filter(|&z| max_leds[z as usize] != ZONE_UNAVAILABLE && (header1 >> z) & 1 == 1)
        .map(|z| {
            let leds = max_leds[z as usize] as usize;
            Zone {
                zone_type: z,
                leds,
                leds_max: if addressable(z) {
                    ADDRESSABLE_MAX_LEDS
                } else {
                    leds
                },
                rgswap: (rgswap >> z) & 1 == 1,
            }
        })
        .collect()
}

pub fn write_header(cfg: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = WRITE_HEADER;
    b[3] = cfg;
    b[4..4 + data.len()].copy_from_slice(data);
    b
}

fn grb(z: &Zone, c: Color) -> [u8; 3] {
    if z.rgswap {
        [c.0, c.1, c.2]
    } else {
        [c.1, c.0, c.2]
    }
}

pub fn write_zone(z: &Zone, mode: u8, speed: u8, c: Color, allzone: bool) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = SET_ZONE;
    b[3] = z.zone_type;
    b[4] = mode;
    b[5..8].copy_from_slice(&grb(z, c));
    b[8] = speed;
    b[9] = 0xFF;
    b[0x10] = allzone as u8;
    b
}

/// WriteAllZones: every zone's LEDs packed back to back, sent in 18- then 19-LED chunks.
pub fn write_all_zones(zones: &[Zone], colors: &[Color]) -> Vec<Vec<u8>> {
    let mut data = Vec::new();
    let mut at = 0;
    for z in zones {
        for c in &colors[at..at + z.leds] {
            data.extend_from_slice(&grb(z, *c));
        }
        at += z.leds;
    }
    let max: usize = zones.iter().map(|z| z.leds_max).sum();
    data.resize((max + 8) * 3, 0);
    let mut out = Vec::new();
    let (mut offset, mut capacity, mut cmd) = (9, 54, INITIAL_CHUNK);
    let mut rest = &data[..];
    while !rest.is_empty() {
        let mut b = vec![0u8; 65];
        b[1] = SET_ZONE;
        b[3] = 0xFF;
        b[4] = cmd;
        if cmd == INITIAL_CHUNK {
            b[7] = 0xFF;
        }
        b[0x40] = 0x65;
        let n = capacity.min(rest.len());
        b[offset..offset + n].copy_from_slice(&rest[..n]);
        rest = &rest[n..];
        out.push(b);
        (offset, capacity, cmd) = (5, 57, SEND_CHUNK);
    }
    out
}

fn with_reads(packets: Vec<Vec<u8>>) -> Vec<Op> {
    packets
        .into_iter()
        .flat_map(|p| [Op::Write(p), Op::Read(READ_LEN)])
        .collect()
}

pub struct Board {
    zones: Vec<Zone>,
    resize: Option<[u8; 8]>,
}

impl Plan for Board {
    /// SetZoneSize if a header was resized, then DeviceUpdateMode for Direct: RGSwap off, each zone into mode 0x0F.
    fn first(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut packets: Vec<Vec<u8>> = self
            .resize
            .iter()
            .map(|t| write_header(LEDCOUNT_CFG, t))
            .collect();
        packets.push(write_header(RGSWAP_CFG, &[0x00]));
        let mut at = 0;
        for z in &self.zones {
            if z.leds > 0 {
                packets.push(write_zone(z, MODE_DIRECT, 0, colors[at], false));
            }
            at += z.leds;
        }
        with_reads(packets)
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        with_reads(write_all_zones(&self.zones, colors))
    }
}

fn query(hid: &HidDevice, cfg: u8, len: usize) -> Result<[u8; READ_LEN], String> {
    hid.write(&read_header(cfg, len))
        .map_err(|e| e.to_string())?;
    let mut r = [0u8; READ_LEN];
    hid.read_timeout(&mut r, 1000).map_err(|e| e.to_string())?;
    Ok(r)
}

const ZONE_NAMES: [&str; 8] = [
    "RGB LED 1 Header",
    "RGB LED 2 Header",
    "Addressable Header 1",
    "Addressable Header 2",
    "PCH",
    "IO Cover",
    "PCB",
    "Addressable Header 3/Audio",
];

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let hid = info.open(api)?;
    let counts = query(&hid, LEDCOUNT_CFG, 65)?;
    let swap = query(&hid, RGSWAP_CFG, 65)?;
    let header1 = query(&hid, 0x01, 64)?;
    let mut max_leds = [0u8; 8];
    max_leds.copy_from_slice(&counts[4..12]);
    let mut zones = zones(max_leds, swap[4], header1[4]);
    let id = info.device_id("asrock-polychrome");
    let resize = sized(&id, &mut zones, max_leds);
    if zones.iter().all(|z| z.leds == 0) {
        return Err("no lighting zones".into());
    }
    let (leds, grid) = rows(&zones.iter().map(|z| z.leds).collect::<Vec<_>>());
    let names: Vec<&str> = zones
        .iter()
        .map(|z| ZONE_NAMES[z.zone_type as usize])
        .collect();
    let device = Device {
        id,
        name: format!("{} ({})", d.name, names.join(", ")),
        kind: DeviceKind::Motherboard,
        rect: [0.06, 0.12, 0.52, 0.03 * zones.len() as f32],
        grid,
        leds,
    };
    Ok(Driver::boxed(
        "asrock polychrome usb",
        device,
        hid,
        Board { zones, resize },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::families::gigabyte_asrock::Session;

    fn zone(zone_type: u8, leds: usize, leds_max: usize, rgswap: bool) -> Zone {
        Zone {
            zone_type,
            leds,
            leds_max,
            rgswap,
        }
    }

    // PolychromeUSBController::ReadConfigTables request packets.
    #[test]
    fn read_header_requests() {
        let mut a = vec![0u8; 65];
        a[1] = 0x14;
        a[3] = 0x02;
        assert_eq!(read_header(LEDCOUNT_CFG, 65), a);
        let mut b = vec![0u8; 64];
        b[1] = 0x14;
        b[3] = 0x01;
        assert_eq!(read_header(0x01, 64), b);
    }

    // PolychromeUSBController::SetDeviceInfo zone selection and types.
    #[test]
    fn zones_follow_config_tables() {
        let z = zones([1, 0x1E, 31, 12, 1, 1, 1, 80], 0b0101_0000, 0b1101_1101);
        assert_eq!(
            z,
            vec![
                zone(0, 1, 1, false),
                zone(2, 31, 100, false),
                zone(3, 12, 100, false),
                zone(4, 1, 1, true),
                zone(6, 1, 1, true),
                zone(7, 80, 100, false),
            ]
        );
    }

    // RGBController_PolychromeUSB::DeviceUpdateMode (Direct) -> SetRGSwap(false), WriteZone.
    #[test]
    fn first_push_disables_rgswap_and_selects_direct() {
        let zones = vec![
            zone(0, 1, 1, false),
            zone(4, 1, 1, true),
            zone(2, 0, 100, false),
        ];
        let mut b = Board {
            zones,
            resize: None,
        };
        let ops = b.first(&[Color(0x11, 0x22, 0x33), Color(0x44, 0x55, 0x66)]);
        let mut rg = vec![0u8; 65];
        rg[1] = 0x15;
        rg[3] = 0x03;
        let mut z0 = vec![0u8; 65];
        z0[1] = 0x10;
        z0[3] = 0x00;
        z0[4] = 0x0F;
        z0[5..10].copy_from_slice(&[0x22, 0x11, 0x33, 0x00, 0xFF]);
        let mut z4 = vec![0u8; 65];
        z4[1] = 0x10;
        z4[3] = 0x04;
        z4[4] = 0x0F;
        z4[5..10].copy_from_slice(&[0x44, 0x55, 0x66, 0x00, 0xFF]);
        assert_eq!(
            ops,
            vec![
                Op::Write(rg),
                Op::Read(64),
                Op::Write(z0),
                Op::Read(64),
                Op::Write(z4),
                Op::Read(64),
            ]
        );
    }

    // PolychromeUSBController::WriteAllZones chunking and padding.
    #[test]
    fn direct_frame_matches_write_all_zones() {
        let zones = vec![zone(0, 1, 1, false), zone(2, 20, 100, true)];
        let mut colors = vec![Color(0x01, 0x02, 0x03)];
        colors.extend((0..20u8).map(|i| Color(i, 0x80, 0xF0)));
        let packets = write_all_zones(&zones, &colors);
        // (1 + 100 + 8) * 3 = 327 bytes: 54 in the first packet, then 57 each
        assert_eq!(packets.len(), 1 + (327usize - 54).div_ceil(57));
        let p0 = &packets[0];
        assert_eq!(p0.len(), 65);
        assert_eq!(
            p0[..9],
            [0x00, 0x10, 0x00, 0xFF, 0xE3, 0x00, 0x00, 0xFF, 0x00]
        );
        assert_eq!(p0[9..12], [0x02, 0x01, 0x03]);
        assert_eq!(p0[12..15], [0x00, 0x80, 0xF0]);
        assert_eq!(p0[60..63], [0x10, 0x80, 0xF0]);
        assert_eq!(p0[63], 0x00);
        assert_eq!(p0[64], 0x65);
        let p1 = &packets[1];
        assert_eq!(p1[..5], [0x00, 0x10, 0x00, 0xFF, 0xE4]);
        assert_eq!(p1[5..8], [0x11, 0x80, 0xF0]);
        assert_eq!(p1[11..14], [0x13, 0x80, 0xF0]);
        assert!(p1[14..62].iter().all(|&b| b == 0));
        assert_eq!(p1[62..], [0x00, 0x00, 0x65]);
        let last = packets.last().unwrap();
        assert!(last[5..64].iter().all(|&b| b == 0));
        assert_eq!(last[64], 0x65);
    }

    #[test]
    fn session_sends_setup_then_frames() {
        let zones = vec![zone(0, 1, 1, false)];
        let board = Board {
            zones,
            resize: None,
        };
        let mut s = Session::new(board, 1);
        let first = s.step(&[Color(1, 2, 3)]);
        // rgswap + zone + one chunk of (1 + 8) * 3 bytes, each with a read
        assert_eq!(first.len(), 6);
        assert_eq!(s.step(&[Color(3, 2, 1)]).len(), 2);
    }

    // SetZoneSize: the LED-count table with the resized header, before Direct mode.
    #[test]
    fn a_set_length_resizes_a_header() {
        let counts = [1, 0x1E, 31, 12, 1, 1, 1, 80];
        let mut z = zones(counts, 0, 0b1000_1101);
        crate::lengths::set_one("hid:polychrome-test", 2, 20);
        let resize = sized("hid:polychrome-test", &mut z, counts);
        assert_eq!(
            z.iter().map(|z| z.leds).collect::<Vec<_>>(),
            [1, 31, 20, 80]
        );
        let table = [1, 0x1E, 31, 20, 1, 1, 1, 80];
        assert_eq!(resize, Some(table));
        let mut fresh = zones(counts, 0, 0b1000_1101);
        assert_eq!(sized("hid:polychrome-test-2", &mut fresh, counts), None);
        crate::lengths::set_one("hid:polychrome-test-3", 1, 30);
        let mut kept = zones(counts, 0, 0b1000_1101);
        assert_eq!(sized("hid:polychrome-test-3", &mut kept, counts), None);
        assert_eq!(kept[1].leds, 31);
        let mut b = Board { zones: z, resize };
        let ops = b.first(&[Color::BLACK; 132]);
        assert_eq!(ops[0], Op::Write(write_header(LEDCOUNT_CFG, &table)));
    }
}
