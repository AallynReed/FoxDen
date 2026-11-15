//! Ported from OpenRGB Controllers/LightSaltController (GPL-2.0-or-later).

use super::common::*;
use crate::registry::{Detector, Info};
use foxrgb_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "lightsalt";
const LEN: usize = 33;
const BRIGHTNESS_DEFAULT: u8 = 0x80;

pub struct Table {
    sets: usize,
    rows: usize,
    columns: usize,
    indices: &'static [usize],
    deflections: &'static [u8],
    map: &'static [&'static [u32]],
    width: u32,
    kind: DeviceKind,
}

static KEYBOARD: Table = Table {
    sets: 4,
    rows: 9,
    columns: 10,
    indices: &[
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
        27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
        50, 51, 52, 53, 54, 55, 56, 57, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 73, 75, 76,
        77, 78, 79, 80, 81, 82, 83, 84, 85, 87, 88, 89,
    ],
    deflections: &[
        0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0xA, 0xA, 0xA, 0xA, 0xA,
        0xA, 0xA, 0xA, 0xA, 0xA, 0xA, 0xA, 0xA, 0x0, 0x0, 0x0, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6,
        0x6, 0x6, 0x6, 0xA, 0xA, 0xA, 0x0, 0x0, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0xA,
        0xA, 0x0, 0x0, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0x6, 0xA, 0xA, 0xA, 0x0, 0x0, 0x0, 0x0, 0x0,
        0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0,
    ],
    map: &[
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, NA, NA],
        &[13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
        &[28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42],
        &[43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, NA, NA],
        &[56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, NA, 68, NA],
        &[69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, NA, 80, 81, 82],
    ],
    width: 15,
    kind: DeviceKind::Keyboard,
};

static KEYPAD: Table = Table {
    sets: 1,
    rows: 5,
    columns: 10,
    indices: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 18],
    deflections: &[0; 18],
    map: &[
        &[0, 1, 2, 3],
        &[4, 5, 6, 7],
        &[8, 9, 10, 11],
        &[12, 13, 14, NA],
        &[15, NA, 16, 17],
    ],
    width: 4,
    kind: DeviceKind::Keyboard,
};

fn command(bytes: &[u8]) -> [u8; LEN] {
    let mut b = [0u8; LEN];
    b[1..1 + bytes.len()].copy_from_slice(bytes);
    b
}

/// DeflectColor: halfway to white.
fn deflect(on: bool, c: Color) -> Color {
    let f = |v: u8| (v as i32 + (255 - v as i32) * 50 / 100) as u8;
    if on { Color(f(c.0), f(c.1), f(c.2)) } else { c }
}

/// DeviceUpdateColors without save: filter, every set and row, then apply.
pub fn colour_packets(t: &Table, colors: &[Color]) -> Vec<[u8; LEN]> {
    // the colour filter tints the whole board; it stays at OpenRGB's default white
    let filter = Color(0xFF, 0xFF, 0xFF);
    let mut data = vec![Color::BLACK; t.sets * t.rows * t.columns];
    for (i, (&index, &defl)) in t.indices.iter().zip(t.deflections).enumerate() {
        for set in 0..t.sets {
            data[set * t.rows * t.columns + index] = deflect(defl & (1 << set) != 0, colors[i]);
        }
    }
    let mut out = vec![command(&[0xFE, 0x04, filter.0, filter.1, filter.2])];
    let mut it = data.iter();
    for set in 0..t.sets {
        for row in 0..t.rows {
            let mut b = [0u8; LEN];
            b[1] = 0x03 + set as u8;
            b[2] = 0x01 + (row * t.columns) as u8;
            for col in 0..t.columns {
                let c = it.next().unwrap();
                b[3 + col * 3..6 + col * 3].copy_from_slice(&rgb(*c));
            }
            out.push(b);
        }
    }
    let mut apply = vec![0xFE, 0x01];
    apply.extend((0..t.sets).map(|s| 0x03 + s as u8));
    out.push(command(&apply));
    out
}

struct LightSalt {
    table: &'static Table,
}

impl Proto for LightSalt {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        // DeviceUpdateMode(Custom): brightness, 25ms, then mode_map[0][0]
        io.write(&command(&[0xFE, 0x05, BRIGHTNESS_DEFAULT]))?;
        io.sleep(25);
        io.write(&command(&[0xFE, 0x03, 0, 0, 0]))
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        colour_packets(self.table, colors)
            .iter()
            .try_for_each(|p| io.write(p))
    }
}

/// QueryDeviceClass: the class string names the table.
pub fn query_class(io: &mut dyn Io) -> Result<Option<&'static Table>, String> {
    io.write(&command(&[0xFE, 0xF5, 0x06]))?;
    let mut model = [0u8; 32];
    io.read(&mut model, 100)?;
    io.write(&command(&[0xFE, 0xFF]))?;
    let mut reply = [0u8; 32];
    if io.read(&mut reply, 100)? != 32 {
        return Ok(None);
    }
    let end = reply.iter().position(|&b| b == 0).unwrap_or(32);
    Ok(match &reply[..end] {
        b"Light&Salt_keyboard1" => Some(&KEYBOARD),
        b"Light&Salt_KEY_PAD" => Some(&KEYPAD),
        _ => None,
    })
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let manufacturer = api
        .device_list()
        .find(|x| x.path().to_string_lossy() == info.path)
        .and_then(|x| x.manufacturer_string().map(str::to_string))
        .unwrap_or_default();
    if !manufacturer.contains("Light&Salt") || !info.product.contains("Light&Salt") {
        return Err("not a Light&Salt device".into());
    }
    let mut io = opened(api, info)?;
    let table = query_class(io.as_mut())?.ok_or("unknown Light&Salt class")?;
    let mut dev = mapped(info, FAMILY, d, table.kind, table.map, table.indices.len());
    dev.grid[0] = table.width;
    Ok(Box::new(Driver::new(io, dev, FAMILY, LightSalt { table })))
}

pub static DETECTORS: &[Detector] = &[det("LightSalt Peripherals", 0x0483, 0x5750, 1, 1, 0, open)];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    #[test]
    fn class_lookup() {
        let mut reply = b"Light&Salt_KEY_PAD".to_vec();
        reply.resize(32, 0);
        let mut rec = Rec::with(vec![vec![0; 32], reply]);
        assert!(std::ptr::eq(
            query_class(&mut rec).unwrap().unwrap(),
            &KEYPAD
        ));
        let ops = rec.take();
        assert_eq!(ops[0], w(command(&[0xFE, 0xF5, 0x06]).to_vec()));
        assert_eq!(ops[2], w(command(&[0xFE, 0xFF]).to_vec()));
    }

    // DeviceUpdateMode(Custom) then DeviceUpdateColors on the keypad
    #[test]
    fn keypad_sequence() {
        let mut a = vec![Color::BLACK; 18];
        a[0] = Color(1, 2, 3);
        a[17] = Color(4, 5, 6); // enter, index 18: row 1 column 8
        let [first, _, _] = pushes(LightSalt { table: &KEYPAD }, 18, vec![], &a, &[]);
        assert_eq!(first[0], w(pad(&[0, 0xFE, 0x05, 0x80], 33)));
        assert_eq!(first[1], Op::Sleep(25));
        assert_eq!(first[2], w(pad(&[0, 0xFE, 0x03, 0, 0, 0], 33)));
        assert_eq!(first[3], w(pad(&[0, 0xFE, 0x04, 0xFF, 0xFF, 0xFF], 33)));
        assert_eq!(first[4], w(pad(&[0, 0x03, 0x01, 1, 2, 3], 33)));
        let mut row1 = pad(&[0, 0x03, 0x0B], 33);
        row1[3 + 24..3 + 27].copy_from_slice(&[4, 5, 6]);
        assert_eq!(first[5], w(row1));
        assert_eq!(first.len(), 3 + 1 + 5 + 1);
        assert_eq!(first[9], w(pad(&[0, 0xFE, 0x01, 0x03], 33)));
    }

    #[test]
    fn deflection_is_halfway_to_white() {
        assert_eq!(deflect(true, Color(0, 100, 255)), Color(127, 177, 255));
        let mut a = vec![Color(0, 0, 0); 83];
        a[13] = Color(0, 0, 0); // backtick, deflected in sets 1 and 3
        let p = colour_packets(&KEYBOARD, &a);
        assert_eq!(p.len(), 1 + 4 * 9 + 1);
        // set 1 (packet 1 + 9), row 1, column 5: index 15
        assert_eq!(p[1 + 9 + 1][3 + 15..3 + 18], [127, 127, 127]);
        assert_eq!(p[1 + 1][3 + 15..3 + 18], [0, 0, 0]);
    }
}
