// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Device, DeviceKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    InsertShiftRight,
    SwapOnly,
    RemoveShiftLeft,
    InsertRow,
}

/// One key edit as a `keyboard_led` gives it; an empty name is KEY_EN_UNUSED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub row: u8,
    pub col: u8,
    pub value: u32,
    pub name: &'static str,
    pub op: Op,
}

pub const fn k(row: u8, col: u8, value: u32, name: &'static str, op: Op) -> Key {
    Key {
        row,
        col,
        value,
        name,
        op,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Default,
    AnsiQwerty,
    IsoQwerty,
    Jis,
}

pub const MAIN: u8 = 1;
pub const FN_ROW: u8 = 2;
pub const EXTRA: u8 = 4;
pub const NUMPAD: u8 = 8;
pub const FULL: u8 = MAIN | FN_ROW | EXTRA | NUMPAD;
pub const TKL: u8 = MAIN | FN_ROW | EXTRA;
pub const SIXTY: u8 = MAIN;

use Op::*;

const fn ins(row: u8, col: u8, name: &'static str) -> Key {
    k(row, col, 0, name, InsertShiftRight)
}

const ZONE_MAIN: [Key; 63] = [
    ins(1, 0, "`"),
    ins(1, 1, "1"),
    ins(1, 2, "2"),
    ins(1, 3, "3"),
    ins(1, 4, "4"),
    ins(1, 5, "5"),
    ins(1, 6, "6"),
    ins(1, 7, "7"),
    ins(1, 8, "8"),
    ins(1, 9, "9"),
    ins(1, 10, "0"),
    ins(1, 11, "-"),
    ins(1, 12, "="),
    ins(1, 13, "Backspace"),
    ins(2, 0, "Tab"),
    ins(2, 1, "Q"),
    ins(2, 2, "W"),
    ins(2, 3, "E"),
    ins(2, 4, "R"),
    ins(2, 5, "T"),
    ins(2, 6, "Y"),
    ins(2, 7, "U"),
    ins(2, 8, "I"),
    ins(2, 9, "O"),
    ins(2, 10, "P"),
    ins(2, 11, "["),
    ins(2, 12, "]"),
    ins(2, 13, "\\ (ANSI)"),
    ins(3, 0, "Caps Lock"),
    ins(3, 1, "A"),
    ins(3, 2, "S"),
    ins(3, 3, "D"),
    ins(3, 4, "F"),
    ins(3, 5, "G"),
    ins(3, 6, "H"),
    ins(3, 7, "J"),
    ins(3, 8, "K"),
    ins(3, 9, "L"),
    ins(3, 10, ";"),
    ins(3, 11, "'"),
    ins(3, 12, "#"),
    ins(3, 13, "Enter"),
    ins(4, 0, "Left Shift"),
    ins(4, 1, "\\ (ISO)"),
    ins(4, 2, "Z"),
    ins(4, 3, "X"),
    ins(4, 4, "C"),
    ins(4, 5, "V"),
    ins(4, 6, "B"),
    ins(4, 7, "N"),
    ins(4, 8, "M"),
    ins(4, 9, ","),
    ins(4, 10, "."),
    ins(4, 11, "/"),
    ins(4, 13, "Right Shift"),
    ins(5, 0, "Left Control"),
    ins(5, 1, "Left Windows"),
    ins(5, 2, "Left Alt"),
    ins(5, 6, "Space"),
    ins(5, 10, "Right Alt"),
    ins(5, 11, "Right Fn"),
    ins(5, 12, "Menu"),
    ins(5, 13, "Right Control"),
];

pub const ESCAPE: Key = ins(0, 0, "Escape");

const ZONE_FN_ROW: [Key; 13] = [
    ESCAPE,
    ins(0, 2, "F1"),
    ins(0, 3, "F2"),
    ins(0, 4, "F3"),
    ins(0, 5, "F4"),
    ins(0, 6, "F5"),
    ins(0, 7, "F6"),
    ins(0, 8, "F7"),
    ins(0, 9, "F8"),
    ins(0, 10, "F9"),
    ins(0, 11, "F10"),
    ins(0, 12, "F11"),
    ins(0, 13, "F12"),
];

const ZONE_EXTRAS: [Key; 13] = [
    ins(0, 14, "Print Screen"),
    ins(0, 15, "Scroll Lock"),
    ins(0, 16, "Pause/Break"),
    ins(1, 14, "Insert"),
    ins(1, 15, "Home"),
    ins(1, 16, "Page Up"),
    ins(2, 14, "Delete"),
    ins(2, 15, "End"),
    ins(2, 16, "Page Down"),
    ins(4, 15, "Up Arrow"),
    ins(5, 14, "Left Arrow"),
    ins(5, 15, "Down Arrow"),
    ins(5, 16, "Right Arrow"),
];

const ZONE_NUMPAD: [Key; 17] = [
    ins(1, 17, "Num Lock"),
    ins(1, 18, "Number Pad /"),
    ins(1, 19, "Number Pad *"),
    ins(1, 20, "Number Pad -"),
    ins(2, 17, "Number Pad 7"),
    ins(2, 18, "Number Pad 8"),
    ins(2, 19, "Number Pad 9"),
    ins(2, 20, "Number Pad +"),
    ins(3, 17, "Number Pad 4"),
    ins(3, 18, "Number Pad 5"),
    ins(3, 19, "Number Pad 6"),
    ins(4, 17, "Number Pad 1"),
    ins(4, 18, "Number Pad 2"),
    ins(4, 19, "Number Pad 3"),
    ins(4, 20, "Number Pad Enter"),
    ins(5, 18, "Number Pad 0"),
    ins(5, 19, "Number Pad ."),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Led {
    pub row: u8,
    pub col: u8,
    pub value: u32,
    pub name: &'static str,
}

pub struct Klm {
    pub keys: Vec<Led>,
    rows: u8,
    cols: u8,
}

impl Klm {
    /// KeyboardLayoutManager(layout, size, values) with an optional regional overlay for `layout`.
    pub fn new(layout: Layout, size: u8, values: &[u32], overlay: &[Key]) -> Klm {
        let mut m = Klm {
            keys: Vec::new(),
            rows: 0,
            cols: 0,
        };
        if size == 0 {
            return m;
        }
        if size & MAIN != 0 {
            m.insert_keys(&ZONE_MAIN);
        }
        if size & FN_ROW != 0 {
            m.insert_keys(&ZONE_FN_ROW);
        }
        if size & EXTRA != 0 {
            m.insert_keys(&ZONE_EXTRAS);
        }
        if size & NUMPAD != 0 {
            m.insert_keys(&ZONE_NUMPAD);
        }
        for (key, &v) in m.keys.iter_mut().zip(values) {
            key.value = v;
        }
        let swap = |row, col| k(row, col, 0, "", SwapOnly);
        match layout {
            Layout::AnsiQwerty => m.change_keys(&[swap(3, 12), swap(4, 1)]),
            Layout::IsoQwerty => m.change_keys(&[swap(2, 13)]),
            Layout::Jis => m.update_dimensions(),
            Layout::Default => {}
        }
        for key in overlay {
            m.swap_key(*key);
        }
        if size == SIXTY {
            m.remove_row(0);
            m.swap_key(ESCAPE);
        }
        m.update_dimensions();
        m
    }

    pub fn change_keys(&mut self, edits: &[Key]) {
        for e in edits {
            match e.op {
                InsertShiftRight => self.insert_key(*e),
                SwapOnly => self.swap_key(*e),
                RemoveShiftLeft => self.remove_key(*e),
                InsertRow => {
                    if self.insert_row(e.row) {
                        self.swap_key(*e)
                    }
                }
            }
        }
        self.update_dimensions();
    }

    fn led(key: Key) -> Led {
        Led {
            row: key.row,
            col: key.col,
            value: key.value,
            name: key.name,
        }
    }

    fn insert_key(&mut self, key: Key) {
        let (r, c) = (key.row, key.col);
        let mut i = self
            .keys
            .iter()
            .position(|k| r < k.row || (r == k.row && c <= k.col))
            .unwrap_or(self.keys.len());
        if i == self.keys.len() {
            self.keys.push(Self::led(key));
            return;
        }
        if !key.name.is_empty() {
            self.keys.insert(i, Self::led(key));
            i += 1;
        }
        while i < self.keys.len() {
            if self.keys[i].row == r && self.keys[i].col >= c {
                self.keys[i].col += 1;
            }
            if self.keys[i].row > r {
                break;
            }
            i += 1;
        }
    }

    fn insert_keys(&mut self, keys: &[Key]) {
        for key in keys {
            self.insert_key(*key);
        }
        self.update_dimensions();
    }

    fn swap_key(&mut self, key: Key) {
        if self.keys.is_empty() {
            self.keys.push(Self::led(key));
            return;
        }
        for i in 0..self.keys.len() {
            let here = self.keys[i];
            if here.row == key.row && here.col == key.col {
                if key.name.is_empty() {
                    self.keys.remove(i);
                } else {
                    self.keys[i].name = key.name;
                    self.keys[i].value = key.value;
                }
                break;
            }
            if here.row > key.row || (here.row == key.row && here.col > key.col) {
                if !key.name.is_empty() {
                    self.keys.insert(i.saturating_sub(1), Self::led(key));
                }
                break;
            }
        }
    }

    fn remove_key(&mut self, key: Key) {
        let r = key.row;
        let shift_rest = |keys: &mut Vec<Led>, mut i: usize| {
            while i < keys.len() && keys[i].row == r {
                keys[i].col -= 1;
                i += 1;
            }
        };
        for i in 0..self.keys.len() {
            if self.keys[i].row == r && self.keys[i].col == key.col {
                self.keys.remove(i);
                shift_rest(&mut self.keys, i);
                break;
            }
            if self.keys[i].row == r && key.col < self.keys[i].col {
                shift_rest(&mut self.keys, i);
                break;
            }
        }
    }

    fn insert_row(&mut self, row: u8) -> bool {
        if row >= self.rows {
            return false;
        }
        let i = self
            .keys
            .iter()
            .position(|k| row <= k.row)
            .unwrap_or(self.keys.len());
        if i < self.keys.len() {
            for k in &mut self.keys[i..] {
                k.row += 1;
            }
        }
        true
    }

    fn remove_row(&mut self, row: u8) {
        if row >= self.rows {
            return;
        }
        let mut i = 0;
        while i < self.keys.len() && row >= self.keys[i].row {
            if self.keys[i].row == row {
                self.keys.remove(i);
            } else {
                i += 1;
            }
        }
        for k in self.keys.iter_mut().skip(i) {
            k.row -= 1;
        }
    }

    fn update_dimensions(&mut self) {
        self.rows = self.keys.iter().map(|k| k.row).max().unwrap_or(0) + 1;
        self.cols = self.keys.iter().map(|k| k.col).max().unwrap_or(0) + 1;
    }

    pub fn values(&self) -> Vec<u32> {
        self.keys.iter().map(|k| k.value).collect()
    }

    /// Each key at its cell centre, and the grid size.
    pub fn positions(&self) -> (Vec<[f32; 2]>, [u32; 2]) {
        let (w, h) = (self.cols.max(1) as f32, self.rows.max(1) as f32);
        let at = self
            .keys
            .iter()
            .map(|k| [(k.col as f32 + 0.5) / w, (k.row as f32 + 0.5) / h])
            .collect();
        (at, [self.cols.max(1) as u32, self.rows.max(1) as u32])
    }

    /// A device with one LED per key, placed at its row and column.
    pub fn device(&self, info: &Info, family: &str, d: &Detector) -> Device {
        let at: Vec<(u32, u32)> = self
            .keys
            .iter()
            .map(|k| (k.col as u32, k.row as u32))
            .collect();
        cells(
            info,
            family,
            d,
            DeviceKind::Keyboard,
            self.cols.max(1) as u32,
            self.rows.max(1) as u32,
            &at,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_ansi_has_104_keys() {
        let m = Klm::new(Layout::AnsiQwerty, FULL, &[], &[]);
        assert_eq!(m.keys.len(), 104);
        assert_eq!((m.rows, m.cols), (6, 21));
        assert_eq!(m.keys[0].name, "Escape");
        assert!(!m.keys.iter().any(|k| k.name == "#" || k.name == "\\ (ISO)"));
    }

    #[test]
    fn values_follow_the_full_iso_order() {
        let values: Vec<u32> = (0..106).collect();
        let m = Klm::new(Layout::AnsiQwerty, FULL, &values, &[]);
        let find = |n: &str| m.keys.iter().find(|k| k.name == n).unwrap().value;
        assert_eq!(find("Escape"), 0);
        assert_eq!(find("Print Screen"), 13);
        assert_eq!(find("`"), 16);
        assert_eq!(find("Number Pad ."), 105);
    }

    #[test]
    fn sixty_drops_the_fn_row() {
        let m = Klm::new(Layout::AnsiQwerty, SIXTY, &[], &[]);
        assert_eq!(m.keys.len(), 61);
        assert_eq!(m.keys[0].name, "Escape");
        assert_eq!(m.rows, 5);
    }
}
