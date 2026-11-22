use crate::{Color, Device, DeviceKind, Press};
use serde::Serialize;

/// Key centres on a full-size ANSI board, in key widths: (Windows virtual-key code, x, row).
const KEYS: &[(u32, f32, u8)] = &[
    (0x1B, 0.5, 0),
    (0x70, 2.5, 0),
    (0x71, 3.5, 0),
    (0x72, 4.5, 0),
    (0x73, 5.5, 0),
    (0x74, 7.25, 0),
    (0x75, 8.25, 0),
    (0x76, 9.25, 0),
    (0x77, 10.25, 0),
    (0x78, 11.75, 0),
    (0x79, 12.75, 0),
    (0x7A, 13.75, 0),
    (0x7B, 14.75, 0),
    (0x2C, 15.75, 0),
    (0x91, 16.75, 0),
    (0x13, 17.75, 0),
    (0xC0, 0.5, 1),
    (0x31, 1.5, 1),
    (0x32, 2.5, 1),
    (0x33, 3.5, 1),
    (0x34, 4.5, 1),
    (0x35, 5.5, 1),
    (0x36, 6.5, 1),
    (0x37, 7.5, 1),
    (0x38, 8.5, 1),
    (0x39, 9.5, 1),
    (0x30, 10.5, 1),
    (0xBD, 11.5, 1),
    (0xBB, 12.5, 1),
    (0x08, 14.0, 1),
    (0x2D, 15.75, 1),
    (0x24, 16.75, 1),
    (0x21, 17.75, 1),
    (0x90, 19.25, 1),
    (0x6F, 20.25, 1),
    (0x6A, 21.25, 1),
    (0x6D, 22.25, 1),
    (0x09, 0.75, 2),
    (0x51, 2.0, 2),
    (0x57, 3.0, 2),
    (0x45, 4.0, 2),
    (0x52, 5.0, 2),
    (0x54, 6.0, 2),
    (0x59, 7.0, 2),
    (0x55, 8.0, 2),
    (0x49, 9.0, 2),
    (0x4F, 10.0, 2),
    (0x50, 11.0, 2),
    (0xDB, 12.0, 2),
    (0xDD, 13.0, 2),
    (0xDC, 14.25, 2),
    (0x2E, 15.75, 2),
    (0x23, 16.75, 2),
    (0x22, 17.75, 2),
    (0x67, 19.25, 2),
    (0x68, 20.25, 2),
    (0x69, 21.25, 2),
    (0x6B, 22.25, 2),
    (0x14, 0.875, 3),
    (0x41, 2.25, 3),
    (0x53, 3.25, 3),
    (0x44, 4.25, 3),
    (0x46, 5.25, 3),
    (0x47, 6.25, 3),
    (0x48, 7.25, 3),
    (0x4A, 8.25, 3),
    (0x4B, 9.25, 3),
    (0x4C, 10.25, 3),
    (0xBA, 11.25, 3),
    (0xDE, 12.25, 3),
    (0x0D, 13.875, 3),
    (0x64, 19.25, 3),
    (0x65, 20.25, 3),
    (0x66, 21.25, 3),
    (0xA0, 1.125, 4),
    (0x10, 1.125, 4),
    (0xE2, 1.75, 4),
    (0x5A, 2.75, 4),
    (0x58, 3.75, 4),
    (0x43, 4.75, 4),
    (0x56, 5.75, 4),
    (0x42, 6.75, 4),
    (0x4E, 7.75, 4),
    (0x4D, 8.75, 4),
    (0xBC, 9.75, 4),
    (0xBE, 10.75, 4),
    (0xBF, 11.75, 4),
    (0xA1, 13.625, 4),
    (0x26, 16.75, 4),
    (0x61, 19.25, 4),
    (0x62, 20.25, 4),
    (0x63, 21.25, 4),
    (0xA2, 0.625, 5),
    (0x11, 0.625, 5),
    (0x5B, 1.875, 5),
    (0xA4, 3.125, 5),
    (0x12, 3.125, 5),
    (0x20, 6.875, 5),
    (0xA5, 10.625, 5),
    (0x5C, 11.875, 5),
    (0x5D, 13.125, 5),
    (0xA3, 14.375, 5),
    (0x25, 15.75, 5),
    (0x28, 16.75, 5),
    (0x27, 17.75, 5),
    (0x60, 19.75, 5),
    (0x6E, 21.25, 5),
];

const FULL: f32 = 22.75;
const TKL: f32 = 18.25;

/// Keys wider than one key width.
fn key_width(vk: u32) -> f32 {
    match vk {
        0x08 | 0x60 => 2.0,
        0x09 | 0xDC => 1.5,
        0x14 => 1.75,
        0x0D | 0xA0 | 0x10 => 2.25,
        0xA1 => 2.75,
        0xA2 | 0x11 | 0x5B | 0xA4 | 0x12 | 0xA5 | 0x5C | 0x5D | 0xA3 => 1.25,
        0x20 => 6.25,
        _ => 1.0,
    }
}

/// One key of the full-size board, for drawing it: centre and width in key widths.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct KeyInfo {
    pub vk: u32,
    pub x: f32,
    pub row: u8,
    pub w: f32,
}

/// Not the left-or-right-neutral codes or the ISO key that shares a spot.
fn listed(vk: u32) -> bool {
    !matches!(vk, 0x10 | 0x11 | 0x12 | 0xE2)
}

/// Every key once, without the left-or-right-neutral codes or the ISO key that shares a spot.
pub fn key_layout() -> Vec<KeyInfo> {
    KEYS.iter()
        .filter(|k| listed(k.0))
        .map(|&(vk, x, row)| KeyInfo {
            vk,
            x,
            row,
            w: key_width(vk),
        })
        .collect()
}

/// Whether canvas point `x`, `y` falls on key `vk` of any keyboard.
pub fn key_at(vk: u32, x: f32, y: f32, keyboards: &[([f32; 4], bool)]) -> bool {
    keyboards.iter().any(|&([bx, by, w, h], full)| {
        let width = if full { FULL } else { TKL };
        KEYS.iter()
            .find(|k| k.0 == vk)
            .is_some_and(|&(_, kx, row)| {
                let hw = key_width(vk) / 2.0;
                kx + hw <= width
                    && (x - (bx + kx / width * w)).abs() <= hw / width * w
                    && (y - (by + (row as f32 + 0.5) / 6.0 * h)).abs() <= h / 12.0
            })
    })
}

/// Where a key sits on a keyboard, each 0..=1. `full` includes the number pad.
pub fn key_spot(vk: u32, full: bool) -> Option<(f32, f32)> {
    let &(_, x, row) = KEYS.iter().find(|k| k.0 == vk)?;
    let width = if full { FULL } else { TKL };
    Some(((x / width).min(1.0), (row as f32 + 0.5) / 6.0))
}

/// The key nearest point `x`, `y` of a board (each 0..=1) in its row, if within half a key.
pub fn key_under(x: f32, y: f32, full: bool) -> Option<u32> {
    let width = if full { FULL } else { TKL };
    let row = (y * 6.0).floor();
    if !(0.0..6.0).contains(&row) || !x.is_finite() {
        return None;
    }
    let at = x * width;
    KEYS.iter()
        .filter(|&&(vk, kx, r)| listed(vk) && r as f32 == row && kx + key_width(vk) / 2.0 <= width)
        .map(|&(vk, kx, _)| (vk, (at - kx).abs() - key_width(vk) / 2.0))
        .filter(|&(_, gap)| gap <= 0.5)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(vk, _)| vk)
}

/// What one LED of a keyboard shows of a key colour map.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    Key(u32),
    /// A lighting zone and the keys nearest it.
    Zone(Vec<u32>),
    None,
}

/// Boards with one row of lights drive zones, not keys.
pub fn zoned(device: &Device) -> bool {
    device.kind == DeviceKind::Keyboard && device.grid[1] <= 1
}

/// The key or zone under each LED of `device`; empty when it is not a keyboard.
pub fn spots(device: &Device) -> Vec<Spot> {
    if device.kind != DeviceKind::Keyboard {
        return Vec::new();
    }
    if !zoned(device) {
        let full = device.number_pad();
        return device
            .leds
            .iter()
            .map(|&[x, y]| key_under(x, y, full).map_or(Spot::None, Spot::Key))
            .collect();
    }
    // Zone boards are taken as full size, so the number pad goes to the zone at the right.
    let (w, h) = (
        device.rect[2].abs().max(1e-6),
        device.rect[3].abs().max(1e-6),
    );
    let mut zones = vec![Vec::new(); device.leds.len()];
    for k in key_layout() {
        let (kx, ky) = (k.x / FULL, (k.row as f32 + 0.5) / 6.0);
        let near = device
            .leds
            .iter()
            .enumerate()
            .map(|(i, [lx, ly])| (i, ((lx - kx) * w).hypot((ly - ky) * h)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = near {
            zones[i].push(k.vk);
        }
    }
    zones.into_iter().map(Spot::Zone).collect()
}

/// A key colour map as a table by virtual-key code.
pub type KeyTable = [Option<Color>; 256];

pub fn key_table(map: &[(u32, Color)]) -> KeyTable {
    let mut table = [None; 256];
    for &(vk, c) in map {
        if let Some(slot) = table.get_mut(vk as usize) {
            *slot = Some(c);
        }
    }
    table
}

/// The colour a spot shows: its key's, or the colour most of a zone's painted keys have.
pub fn paint(spot: &Spot, table: &KeyTable) -> Option<Color> {
    let at = |vk: u32| table.get(vk as usize).copied().flatten();
    match spot {
        Spot::Key(vk) => at(*vk),
        Spot::None => None,
        Spot::Zone(keys) => {
            let mut counts: Vec<(Color, usize)> = Vec::new();
            for c in keys.iter().filter_map(|&vk| at(vk)) {
                match counts.iter_mut().find(|(seen, _)| *seen == c) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((c, 1)),
                }
            }
            let most = counts.iter().map(|(_, n)| *n).max()?;
            counts.into_iter().find(|(_, n)| *n == most).map(|(c, _)| c)
        }
    }
}

/// Known keys only, each once (the last colour wins), in key-code order, at most one per key.
pub fn clean_key_map(map: Vec<(u32, Color)>) -> Vec<(u32, Color)> {
    let mut table = [None; 256];
    for (vk, c) in map {
        if KEYS.iter().any(|k| k.0 == vk && listed(vk)) {
            table[vk as usize] = Some(c);
        }
    }
    (0..256u32)
        .filter_map(|vk| table[vk as usize].map(|c| (vk, c)))
        .collect()
}

/// Places key presses `(virtual-key code, age, seed)` on each keyboard `(rect, full)` of the canvas.
pub fn presses_on(keys: &[(u32, f32, u32)], keyboards: &[([f32; 4], bool)]) -> Vec<Press> {
    keys.iter()
        .flat_map(|&(vk, age, seed)| {
            keyboards.iter().filter_map(move |&([x, y, w, h], full)| {
                key_spot(vk, full).map(|(kx, ky)| Press {
                    x: x + kx * w,
                    y: y + ky * h,
                    age,
                    hw: w / if full { FULL } else { TKL } / 2.0,
                    hh: h / 12.0,
                    seed,
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_land_where_they_are_on_the_board() {
        let (ex, ey) = key_spot(0x1B, true).unwrap();
        assert!(ex < 0.05 && ey < 0.2);
        let (sx, sy) = key_spot(0x20, false).unwrap();
        assert!((0.3..0.45).contains(&sx) && sy > 0.8);
        assert!(key_spot(0x60, false).unwrap().0 <= 1.0);
        assert_eq!(key_spot(0xFF, true), None);
    }

    #[test]
    fn a_press_lands_on_every_keyboard() {
        let boards = [([0.0, 0.0, 0.5, 0.5], true), ([0.5, 0.5, 0.5, 0.5], false)];
        let p = presses_on(&[(0x1B, 0.2, 1), (0xFF, 0.1, 2)], &boards);
        assert_eq!(p.len(), 2);
        assert!(p[0].x < 0.05 && p[1].x >= 0.5 && p[1].y >= 0.5);
        assert_eq!(p[0].age, 0.2);
    }

    #[test]
    fn wide_keys_cover_their_width() {
        let board = [([0.0, 0.0, 1.0, 1.0], false)];
        let (sx, sy) = key_spot(0x20, false).unwrap();
        assert!(key_at(0x20, sx + 2.5 / TKL, sy, &board));
        assert!(!key_at(0x20, sx + 3.5 / TKL, sy, &board));
        assert!(!key_at(0x20, sx, sy - 0.1, &board));
        assert!(!key_at(0x41, sx, sy, &board));
        assert!(!key_at(0x6B, 0.99, 0.4, &board));
        let layout = key_layout();
        assert!(layout.iter().all(|k| k.vk != 0x10) && layout.len() == KEYS.len() - 4);
    }

    const RED: Color = Color(255, 0, 0);
    const BLUE: Color = Color(0, 0, 255);

    /// A per-key board with an LED on every key it has.
    fn per_key(full: bool) -> Device {
        let width = if full { FULL } else { TKL };
        let keys: Vec<_> = key_layout()
            .into_iter()
            .filter(|k| k.x + k.w / 2.0 <= width)
            .collect();
        Device {
            id: "kb".into(),
            name: "Board".into(),
            kind: DeviceKind::Keyboard,
            rect: [0.1, 0.5, 0.5, 0.3],
            grid: [if full { 22 } else { 18 }, 6],
            leds: keys
                .iter()
                .map(|k| key_spot(k.vk, full).unwrap().into())
                .collect(),
        }
    }

    fn shown(device: &Device, map: &[(u32, Color)]) -> Vec<Option<Color>> {
        let table = key_table(map);
        spots(device).iter().map(|s| paint(s, &table)).collect()
    }

    #[test]
    fn painted_keys_light_their_own_leds() {
        let board = per_key(false);
        let lit = shown(&board, &[(0x41, RED), (0x57, BLUE), (0x20, RED)]);
        let at = |vk| {
            let spot = key_spot(vk, false).unwrap();
            lit[board
                .leds
                .iter()
                .position(|l| (l[0], l[1]) == spot)
                .unwrap()]
        };
        assert_eq!(
            (at(0x41), at(0x57), at(0x20)),
            (Some(RED), Some(BLUE), Some(RED))
        );
        assert_eq!(at(0x53), None);
        assert_eq!(lit.iter().flatten().count(), 3);
        assert!(spots(&board).iter().all(|s| matches!(s, Spot::Key(_))));
    }

    #[test]
    fn number_pad_keys_only_light_on_boards_with_one() {
        let map = [(0x67, RED), (0x41, BLUE)];
        let tkl = shown(&per_key(false), &map);
        assert!(!tkl.contains(&Some(RED)) && tkl.contains(&Some(BLUE)));
        assert!(shown(&per_key(true), &map).contains(&Some(RED)));
        assert_eq!(key_under(0.99, 0.4, false), Some(0x22));
        assert_eq!(key_under(f32::NAN, 0.4, true), None);
        assert_eq!(key_under(0.5, 1.5, true), None);
    }

    #[test]
    fn zone_boards_take_the_colour_most_painted_keys_in_a_zone_have() {
        let board = Device::grid("z", "Z", DeviceKind::Keyboard, [0.0, 0.0, 0.5, 0.2], 4, 1);
        assert!(zoned(&board) && !zoned(&per_key(false)));
        let lit = shown(
            &board,
            &[(0x57, RED), (0x41, RED), (0x53, BLUE), (0x67, BLUE)],
        );
        assert_eq!(lit, [Some(RED), None, None, Some(BLUE)]);
        let zones = spots(&board);
        let total: usize = zones
            .iter()
            .map(|s| match s {
                Spot::Zone(keys) => keys.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(total, key_layout().len());
        let mouse = Device::grid("m", "M", DeviceKind::Mouse, [0.0; 4], 2, 1);
        assert!(spots(&mouse).is_empty());
    }

    #[test]
    fn key_maps_keep_known_keys_once() {
        let map = clean_key_map(vec![
            (0x41, RED),
            (0x10, RED),
            (0x41, BLUE),
            (999, RED),
            (0x1B, RED),
        ]);
        assert_eq!(map, [(0x1B, RED), (0x41, BLUE)]);
    }

    #[test]
    fn no_key_is_listed_twice() {
        for (i, a) in KEYS.iter().enumerate() {
            assert!(KEYS[i + 1..].iter().all(|b| b.0 != a.0), "{:#x}", a.0);
        }
    }
}
