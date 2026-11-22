// SPDX-License-Identifier: GPL-2.0-or-later

use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

use super::{NA, Op, Protocol, any, at, device, matrix, open_with};
use crate::registry::{Detector, Info};

const VID: u16 = 0x1B1C;
const BLOCK_LEDS: usize = 16;
/// The fan header reports no size; this is its maximum.
const FAN_LEDS: usize = 32;
const MAGIC_1: u8 = 0b001;
const MAGIC_2: u8 = 0b010;
const MAGIC_3: u8 = 0b011;
const LIGHTING: [u8; 3] = [0b100, 0b101, 0b110];
/// Models whose fans carry no LEDs.
const NO_RGB_FAN: [u16; 5] = [0x0C35, 0x0C36, 0x0C37, 0x0C40, 0x0C41];

#[rustfmt::skip]
const MAP: [u32; 25] = [
    NA, 11, 12, 13, NA,
    10, NA, 1, NA, 14,
    9, 0, NA, 2, 15,
    8, NA, 3, NA, 4,
    NA, 7, 6, 5, NA,
];

const MAGIC: [[u8; 61]; 3] = {
    let mut m = [[0xFFu8; 61]; 3];
    m[0][0] = 0x01;
    m[0][1] = 0x01;
    let mut i = 15;
    while i < 19 {
        m[0][i] = 0x7F;
        i += 1;
    }
    m[0][20] = 0x00;
    m[0][25] = 0x00;
    m[0][30] = 0x00;
    m[0][35] = 0x00;
    m[0][40] = 0x00;
    m[0][45] = 0x00;
    let mut i = 0;
    while i < 40 {
        m[1][i] = i as u8;
        m[2][i] = 0x28 + i as u8;
        i += 1;
    }
    m
};

#[rustfmt::skip]
const HARDWARE: [[u8; 61]; 4] = [
    [
        0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
        0x7F, 0x7F, 0x7F, 0x7F, 0x09, 0x20, 0x07, 0x00, 0x0B, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF,
        0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ],
    [
        0x0A, 0x01, 0x04, 0x07, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x00, 0x01, 0x02, 0x03, 0x04,
        0x01, 0x0A, 0x07, 0x04, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00,
        0x01, 0x0A, 0x07, 0x04, 0x01, 0x0A, 0x09, 0x08, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ],
    [
        0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
        0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C,
        0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ],
    [
        0x00, 0x00, 0xFF, 0x00, 0x4A, 0xFF, 0x00, 0x94, 0xFF, 0x00, 0xDF, 0xFF, 0x00, 0xFF, 0xAA, 0x00,
        0xFF, 0x15, 0x7F, 0x7F, 0x00, 0xFA, 0x00, 0x06, 0xDB, 0x00, 0x32, 0xD7, 0x00, 0x58, 0xF6, 0x00,
        0x76, 0x94, 0x00, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ],
];

/// The PEC byte: CRC-8 with polynomial 0x07, as `CRC_TABLE` holds it.
fn pec(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |crc, &b| {
        (0..8).fold(crc ^ b, |c, _| {
            if c & 0x80 != 0 {
                (c << 1) ^ 0x07
            } else {
                c << 1
            }
        })
    })
}

fn packet(seq: u8, command: u8, body: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 65];
    b[1] = 0x3F;
    b[2] = seq | command;
    b[3..3 + body.len()].copy_from_slice(body);
    b[64] = pec(&b[2..64]);
    b
}

struct Platinum {
    leds: usize,
    sequence: u8,
    /// The fan driver's device, whose sequence both drivers step.
    shared: Option<std::sync::Arc<crate::port::Device>>,
}

impl Platinum {
    /// `GetSequenceNumber`: 1..=31, shifted into the top five bits.
    fn next(&mut self) -> u8 {
        if let Some(d) = &self.shared {
            return d.next_seq() << 3;
        }
        self.sequence = if self.sequence < 31 {
            self.sequence + 1
        } else {
            1
        };
        self.sequence << 3
    }

    fn magic(&mut self, magic: &[u8; 61], command: u8) -> Vec<Op> {
        let seq = self.next();
        vec![
            Op::Write(packet(seq, command, magic)),
            Op::Read(65, 100),
            Op::Sleep(5),
        ]
    }
}

impl Protocol for Platinum {
    fn setup(&mut self, _: &[Color]) -> Vec<Op> {
        let mut ops = self.magic(&MAGIC[0], MAGIC_1);
        ops.extend(self.magic(&MAGIC[1], MAGIC_2));
        ops.extend(self.magic(&MAGIC[2], MAGIC_3));
        ops
    }

    fn frame(&mut self, colors: &[Color]) -> Vec<Op> {
        let mut ops = Vec::new();
        for (i, (from, to)) in [(0, 20), (20, 40), (40, 48)].into_iter().enumerate() {
            if from > 0 && self.leds <= from {
                break;
            }
            let body: Vec<u8> = (from..to.min(self.leds))
                .flat_map(|led| {
                    let c = at(colors, led);
                    [c.2, c.1, c.0]
                })
                .collect();
            let seq = self.next();
            ops.push(Op::Write(packet(seq, LIGHTING[i], &body)));
            ops.push(Op::Sleep(5));
        }
        ops
    }

    fn release(&mut self) -> Vec<Op> {
        let mut ops = self.magic(&HARDWARE[1], MAGIC_2);
        ops.extend(self.magic(&HARDWARE[2], MAGIC_3));
        ops.extend(self.magic(&HARDWARE[3], LIGHTING[0]));
        ops.extend(self.magic(&HARDWARE[0], MAGIC_1));
        ops
    }
}

/// The fan header at the user's length; models without RGB fans have none.
fn fan_leds(id: &str, pid: u16) -> usize {
    if NO_RGB_FAN.contains(&pid) {
        return 0;
    }
    crate::lengths::length(id, 1, "Fans", FAN_LEDS, 0, FAN_LEDS)
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    open_with(api, info, "corsair hydro platinum", |hid| {
        let id = info.device_id("corsair-hydro-platinum");
        let fan = fan_leds(&id, info.pid);
        let leds_n = BLOCK_LEDS + fan;
        let mut leds: Vec<[f32; 2]> = matrix(&MAP, 5, BLOCK_LEDS, |_| [0.5, 0.5])
            .into_iter()
            .map(|[x, y]| [x, y * 0.8])
            .collect();
        leds.extend((0..fan).map(|i| [(i as f32 + 0.5) / fan as f32, 0.9]));
        let dev = device(id, d.name, DeviceKind::Other, [5, 6], leds);
        Ok((
            dev,
            Platinum {
                leds: leds_n,
                sequence: 0,
                shared: crate::port::is_shared(info.vid, info.pid).then(|| hid.device().clone()),
            },
        ))
    })
}

pub(super) static DETECTORS: [Detector; 13] = [
    any("Corsair Hydro H100i Platinum", VID, 0x0C18, open),
    any("Corsair Hydro H100i Platinum SE", VID, 0x0C19, open),
    any("Corsair Hydro H115i Platinum", VID, 0x0C17, open),
    any("Corsair Hydro H60i Pro XT", VID, 0x0C29, open),
    any("Corsair Hydro H100i Pro XT", VID, 0x0C20, open),
    any("Corsair Hydro H100i Pro XT v2", VID, 0x0C2D, open),
    any("Corsair Hydro H115i Pro XT", VID, 0x0C21, open),
    any("Corsair Hydro H150i Pro XT", VID, 0x0C22, open),
    any("Corsair Hydro H100i Elite", VID, 0x0C35, open),
    any("Corsair Hydro H115i Elite", VID, 0x0C36, open),
    any("Corsair Hydro H150i Elite", VID, 0x0C37, open),
    any("Corsair Hydro H100i Elite White", VID, 0x0C40, open),
    any("Corsair Hydro H150i Elite White", VID, 0x0C41, open),
];

#[cfg(test)]
mod tests {
    use super::super::test_util::bytes;
    use super::*;

    // CRC table
    #[test]
    fn pec_matches_crc_table() {
        assert_eq!(pec(&[0x01]), 0x07);
        assert_eq!(pec(&[0x08]), 0x38);
        assert_eq!(pec(&[0x80]), 0x89);
        assert_eq!(pec(&[0xFF]), 0xF3);
        // CRC_TABLE[CRC_TABLE[0x01] ^ 0x01] = CRC_TABLE[0x06]
        assert_eq!(pec(&[0x01, 0x01]), 0x12);
    }

    // MAGIC_1..3 tables
    #[test]
    fn magic_tables() {
        let m1: [u8; 61] = [
            0x01, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0x7F, 0x7F, 0x7F, 0x7F, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF,
            0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF,
            0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        assert_eq!(MAGIC[0], m1);
        assert_eq!(
            MAGIC[1][..41],
            (0..40).chain([0xFF]).collect::<Vec<u8>>()[..]
        );
        assert_eq!(MAGIC[2][39..41], [0x4F, 0xFF]);
        assert!(MAGIC[1][40..].iter().all(|&b| b == 0xFF));
    }

    // Constructor: SendMagic(MAGIC_1..3) with sequence numbers 1, 2, 3
    #[test]
    fn setup_sends_three_magics() {
        let mut p = Platinum {
            leds: 48,
            sequence: 0,
            shared: None,
        };
        let ops = p.setup(&[]);
        assert_eq!(ops.len(), 9);
        let first = bytes(&ops[0]);
        assert_eq!(first.len(), 65);
        assert_eq!(first[..3], [0x00, 0x3F, 0x08 | 0x01]);
        assert_eq!(first[3..64], MAGIC[0]);
        assert_eq!(first[64], pec(&first[2..64]));
        assert_eq!(ops[1], Op::Read(65, 100));
        assert_eq!(ops[2], Op::Sleep(5));
        assert_eq!(bytes(&ops[3])[2], 0x10 | 0x02);
        assert_eq!(bytes(&ops[6])[2], 0x18 | 0x03);
    }

    // SetupColors -> SendColors: BGR, 20 LEDs per packet, commands 4/5/6
    #[test]
    fn frame_sends_bgr_in_three_packets() {
        let colors: Vec<Color> = (0..48).map(|i| Color(i, 100 + i, 200 + i / 2)).collect();
        let mut p = Platinum {
            leds: 48,
            sequence: 3,
            shared: None,
        };
        let ops = p.frame(&colors);
        assert_eq!(ops.len(), 6);
        let a = bytes(&ops[0]);
        assert_eq!(a[..6], [0x00, 0x3F, 0x20 | 0x04, 200, 100, 0]);
        assert_eq!(a[60..63], [209, 119, 19]);
        assert_eq!(a[63], 0);
        assert_eq!(a[64], pec(&a[2..64]));
        let b = bytes(&ops[2]);
        assert_eq!(b[..6], [0x00, 0x3F, 0x28 | 0x05, 210, 120, 20]);
        let c = bytes(&ops[4]);
        assert_eq!(c[..6], [0x00, 0x3F, 0x30 | 0x06, 220, 140, 40]);
        assert_eq!(c[24..27], [223, 147, 47]);
        assert!(c[27..64].iter().all(|&x| x == 0));
        assert_eq!(ops[5], Op::Sleep(5));
    }

    // ~CorsairHydroPlatinumController: HARDWARE_MAGIC_2, 3, 4 (as SET_LIGHTING_1), then 1
    #[test]
    fn release_hands_back_to_hardware() {
        let mut p = Platinum {
            leds: 48,
            sequence: 10,
            shared: None,
        };
        let ops = p.release();
        assert_eq!(ops.len(), 12);
        let heads: Vec<u8> = ops.iter().step_by(3).map(|o| bytes(o)[2]).collect();
        assert_eq!(heads, [0x58 | 0x02, 0x60 | 0x03, 0x68 | 0x04, 0x70 | 0x01]);
        assert_eq!(bytes(&ops[6])[3..8], [0x00, 0x00, 0xFF, 0x00, 0x4A]);
        assert_eq!(bytes(&ops[9])[23..26], [0x09, 0x20, 0x07]);
    }

    #[test]
    fn a_set_length_shortens_the_fans() {
        crate::lengths::set_one("hid:hydro-platinum-test", 1, 4);
        assert_eq!(fan_leds("hid:hydro-platinum-test", 0x0C18), 4);
        assert_eq!(fan_leds("hid:hydro-platinum-test", 0x0C35), 0);
        let mut p = Platinum {
            leds: 20,
            sequence: 0,
            shared: None,
        };
        assert_eq!(p.frame(&[Color(1, 2, 3); 20]).len(), 2);
    }

    // GetSequenceNumber wraps from 31 back to 1; Elite models send only the block packet
    #[test]
    fn sequence_wraps_and_block_only() {
        let mut p = Platinum {
            leds: 16,
            sequence: 31,
            shared: None,
        };
        let ops = p.frame(&[Color(1, 2, 3); 16]);
        assert_eq!(ops.len(), 2);
        assert_eq!(bytes(&ops[0])[2], 0x08 | 0x04);
        assert_eq!(bytes(&ops[0])[3..6], [3, 2, 1]);
        assert!(bytes(&ops[0])[51..64].iter().all(|&x| x == 0));
    }
}
