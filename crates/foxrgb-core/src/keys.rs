use crate::Press;

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

/// Where a key sits on a keyboard, each 0..=1. `full` includes the number pad.
pub fn key_spot(vk: u32, full: bool) -> Option<(f32, f32)> {
    let &(_, x, row) = KEYS.iter().find(|k| k.0 == vk)?;
    let width = if full { FULL } else { TKL };
    Some(((x / width).min(1.0), (row as f32 + 0.5) / 6.0))
}

/// Places key presses `(virtual-key code, age)` on each keyboard `(rect, full)` of the canvas.
pub fn presses_on(keys: &[(u32, f32)], keyboards: &[([f32; 4], bool)]) -> Vec<Press> {
    keys.iter()
        .flat_map(|&(vk, age)| {
            keyboards.iter().filter_map(move |&([x, y, w, h], full)| {
                key_spot(vk, full).map(|(kx, ky)| Press {
                    x: x + kx * w,
                    y: y + ky * h,
                    age,
                    hw: w / if full { FULL } else { TKL } / 2.0,
                    hh: h / 12.0,
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
        let p = presses_on(&[(0x1B, 0.2), (0xFF, 0.1)], &boards);
        assert_eq!(p.len(), 2);
        assert!(p[0].x < 0.05 && p[1].x >= 0.5 && p[1].y >= 0.5);
        assert_eq!(p[0].age, 0.2);
    }

    #[test]
    fn no_key_is_listed_twice() {
        for (i, a) in KEYS.iter().enumerate() {
            assert!(KEYS[i + 1..].iter().all(|b| b.0 != a.0), "{:#x}", a.0);
        }
    }
}
