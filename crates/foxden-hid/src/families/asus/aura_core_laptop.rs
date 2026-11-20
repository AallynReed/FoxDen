//! Ported from OpenRGB Controllers/AsusAuraCoreController/AsusAuraCoreLaptopController (GPL-2.0-or-later).

use super::common::*;
use super::dmi;
use super::klm::{Klm, Layout as KbLayout, Led};
use super::laptop_layouts::{LAPTOPS, Laptop};
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-core-laptop";
const VID: u16 = 0x0B05;
const ID: u8 = 0x5D;
const LEN: usize = 64;
const KEY_SET: u8 = 167;
const PER_PACKET: u8 = 16;
const DATA: usize = 9;
const LAYOUT_ISO: u8 = 0x02;
/// Direct's default brightness, ASUSAURACORELAPTOP_BRIGHTNESS_MAX.
const BRIGHTNESS: u8 = 3;

fn packet(head: &[u8]) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[..head.len()].copy_from_slice(head);
    b
}

/// SetPowerConfigFromJSON's flag packing, one state per zone (logo, keyboard, lightbar, lid) and power state.
pub fn power_flags(s: [bool; 16]) -> u32 {
    let bits = [
        s[0], s[4], s[1], s[5], !s[2], !s[6], !s[3], !s[7], false, s[8], s[9], !s[10], !s[11],
        false, false, false, s[12], s[13], !s[14], !s[15],
    ];
    bits.iter()
        .enumerate()
        .fold(0, |acc, (i, &b)| acc | ((b as u32) << i))
}

/// SendPowerConfig with every state on, the settings file's default.
pub fn power_config() -> [u8; LEN] {
    let f = power_flags([true; 16]);
    packet(&[ID, 0xBD, 0x01, f as u8, (f >> 8) as u8, (f >> 16) as u8])
}

/// SendInitDirectMode.
pub fn init_direct(single_zone: bool) -> [u8; LEN] {
    packet(&[ID, 0xBC, single_zone as u8])
}

/// SetLedsDirect: colours indexed by key value, 167 keys in packets of 16, then the rest.
pub fn direct(buffer: &[Color]) -> Vec<[u8; LEN]> {
    let count = buffer.len() as u8;
    let keys: Vec<u8> = buffer[..count as usize]
        .iter()
        .flat_map(|c| rgb(*c))
        .collect();
    if count == 1 {
        let mut b = packet(&[ID, 0xBC, 0x01, 0x01]);
        b[DATA..DATA + 3].copy_from_slice(&keys[..3]);
        return vec![b];
    }
    let mut b = packet(&[ID, 0xBC, 0x00, 0x01, 0x01, 0x01, 0x00, PER_PACKET, 0x00]);
    let mut out = Vec::new();
    let mut i = 0u8;
    while i < KEY_SET {
        if KEY_SET - i < PER_PACKET {
            b[7] = KEY_SET - i;
            b[DATA..].fill(0);
        }
        b[6] = i;
        for j in 0..3 * b[7] as usize {
            b[DATA + j] = keys.get(3 * i as usize + j).copied().unwrap_or(0);
        }
        out.push(b);
        i += PER_PACKET;
    }
    b[4..8].fill(0);
    b[4] = 0x04;
    b[DATA..].fill(0);
    let rest = keys.get(3 * KEY_SET as usize..).unwrap_or(&[]);
    let n = rest.len().min(LEN - DATA);
    b[DATA..DATA + n].copy_from_slice(&rest[..n]);
    out.push(b);
    out
}

/// The zones' keys as SetupZones lays them out, plus the length of the value-indexed buffer.
pub fn zones(laptop: &Laptop, iso: bool) -> (Vec<Vec<Led>>, usize) {
    let layout = if iso {
        KbLayout::IsoQwerty
    } else {
        KbLayout::AnsiQwerty
    };
    let mut max = 0u32;
    let mut out = Vec::new();
    for z in laptop.zones {
        let mut m = Klm::new(layout, z.size, z.values, &[]);
        let mut leds = Vec::new();
        if z.size != 0 || !z.edits.is_empty() {
            m.change_keys(z.edits);
            for k in &m.keys {
                max = max.max(k.value);
                leds.push(*k);
            }
        }
        max += 1;
        out.push(leds);
    }
    (out, max as usize)
}

fn single_zone(laptop: &Laptop) -> bool {
    let z = laptop.zones[0];
    z.size == 0 && z.edits.len() == 1
}

struct LaptopKb {
    values: Vec<u32>,
    buffer: usize,
    single: bool,
}

impl Proto for LaptopKb {
    /// DeviceUpdateMode(Direct): SendBrightness, then SendInitDirectMode.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        io.feature(&packet(&[ID, 0xBA, 0xC5, 0xC4, BRIGHTNESS]))?;
        io.feature(&init_direct(self.single))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut buffer = vec![Color::BLACK; self.buffer];
        for (v, c) in self.values.iter().zip(colors) {
            buffer[*v as usize] = *c;
        }
        for p in direct(&buffer) {
            io.feature(&p)?;
        }
        Ok(())
    }
}

/// GetKeyboardLayout: drain reads, ask, and take byte 12 of the answer.
fn keyboard_layout(io: &mut dyn Io) -> Result<u8, String> {
    let mut r = [0u8; LEN];
    while io.read(&mut r, 10)? > 0 {}
    io.feature(&packet(&[ID, 0x05, 0x20, 0x31, 0x00, 0x10]))?;
    let mut r = [0u8; LEN];
    r[0] = ID;
    Ok(match io.get_feature(&mut r) {
        Ok(n) if n > 0 => r[12],
        _ => 0x01,
    })
}

fn laptop() -> Option<&'static Laptop> {
    let board = dmi::board()?;
    LAPTOPS.iter().find(|l| l.dmi == board)
}

pub fn known() -> bool {
    laptop().is_some()
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let laptop = laptop().ok_or("laptop not in the Aura Core table")?;
    let mut io = opened(api, info)?;
    let single = single_zone(laptop);
    io.feature(&power_config())?;
    io.feature(&init_direct(single))?;
    let iso = keyboard_layout(io.as_mut())? == LAYOUT_ISO;
    let (zones, buffer) = zones(laptop, iso);
    let mut at = Vec::new();
    let mut values = Vec::new();
    let mut top = 0u32;
    let mut cols = 1u32;
    for z in &zones {
        let rows = z.iter().map(|k| k.row as u32 + 1).max().unwrap_or(0);
        for k in z {
            at.push((k.col as u32, top + k.row as u32));
            values.push(k.value);
            cols = cols.max(k.col as u32 + 1);
        }
        top += rows;
    }
    if values.is_empty() {
        return Err("no Aura Core laptop LEDs".into());
    }
    let dev = cells(info, FAMILY, d, DeviceKind::Keyboard, cols, top.max(1), &at);
    let proto = LaptopKb {
        values,
        buffer,
        single,
    };
    Ok(Box::new(Driver::new(io, dev, FAMILY, proto)))
}

pub static DETECTORS: &[Detector] = &[
    det(
        "ASUS ROG Strix SCAR 15",
        VID,
        0x19B6,
        ANY,
        0xFF31,
        0x79,
        open,
    ),
    det(
        "ASUS ROG Strix SCAR 17",
        VID,
        0x1866,
        ANY,
        0xFF31,
        0x79,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // SetPowerConfigFromJSON with the default settings, then SendInitDirectMode
    #[test]
    fn init_reports() {
        assert_eq!(
            power_config().to_vec(),
            pad(&[0x5D, 0xBD, 0x01, 0x0F, 0x06, 0x03], 64)
        );
        let gu603 = LAPTOPS.iter().find(|l| l.dmi == "GU603VV").unwrap();
        assert!(single_zone(gu603));
        assert_eq!(init_direct(true).to_vec(), pad(&[0x5D, 0xBC, 1], 64));
    }

    // SetupZones for the G533ZW: keyboard, lid and lightbar values set the buffer length
    #[test]
    fn g533zw_zones() {
        let l = LAPTOPS.iter().find(|l| l.dmi == "G533ZW").unwrap();
        let (z, buffer) = zones(l, false);
        assert_eq!(z.len(), 3);
        assert_eq!(
            z[1].iter().map(|k| k.value).collect::<Vec<_>>(),
            [167, 176, 177]
        );
        assert_eq!(z[0][0].value, 2);
        assert_eq!(buffer, 179);
    }

    // GetKeyboardLayout: drain, ask, byte 12 of the answer
    #[test]
    fn layout_query() {
        let mut reply = pad(&[0x5D], 64);
        reply[12] = LAYOUT_ISO;
        let mut rec = Rec::with(vec![vec![], reply]);
        assert_eq!(keyboard_layout(&mut rec).unwrap(), LAYOUT_ISO);
        assert_eq!(
            rec.take(),
            vec![
                Op::Read,
                Op::F(pad(&[0x5D, 0x05, 0x20, 0x31, 0, 0x10], 64)),
                Op::GetF(0x5D),
            ]
        );
        assert_eq!(keyboard_layout(&mut Rec::default()).unwrap(), 0x01);
    }

    // DeviceUpdateMode(Direct), then SetLedsDirect over the value-indexed buffer
    #[test]
    fn direct_frame() {
        let proto = LaptopKb {
            values: vec![0, 16, 170],
            buffer: 171,
            single: false,
        };
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let [first, _, _] = pushes(proto, 3, vec![], &a, &[]);
        assert_eq!(first.len(), 2 + 11 + 1);
        assert_eq!(first[0], Op::F(pad(&[0x5D, 0xBA, 0xC5, 0xC4, 3], 64)));
        assert_eq!(first[1], Op::F(pad(&[0x5D, 0xBC, 0], 64)));
        assert_eq!(
            first[2],
            Op::F(pad(&[0x5D, 0xBC, 0, 1, 1, 1, 0, 16, 0, 1, 2, 3], 64))
        );
        assert_eq!(
            first[3],
            Op::F(pad(&[0x5D, 0xBC, 0, 1, 1, 1, 16, 16, 0, 4, 5, 6], 64))
        );
        assert_eq!(
            first[12],
            Op::F(pad(&[0x5D, 0xBC, 0, 1, 1, 1, 160, 7, 0], 64))
        );
        let mut tail = pad(&[0x5D, 0xBC, 0, 1, 4, 0, 0, 0, 0], 64);
        tail[9 + 9..9 + 12].copy_from_slice(&[7, 8, 9]);
        assert_eq!(first[13], Op::F(tail));
        let one = direct(&[Color(1, 2, 3)]);
        assert_eq!(
            one[0].to_vec(),
            pad(&[0x5D, 0xBC, 1, 1, 0, 0, 0, 0, 0, 1, 2, 3], 64)
        );
    }
}
