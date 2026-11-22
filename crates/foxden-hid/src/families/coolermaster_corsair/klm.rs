// SPDX-License-Identifier: GPL-2.0-or-later

/// One key: matrix position, the value the device indexes it by, and whether it has a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub row: u8,
    pub col: u8,
    pub value: u32,
    pub named: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Insert,
    Swap,
    InsertRow,
}

#[derive(Clone, Copy, Debug)]
pub struct Edit {
    pub key: Key,
    pub op: Op,
}

const fn edit(row: u8, col: u8, value: u32, named: bool, op: Op) -> Edit {
    Edit {
        key: Key {
            row,
            col,
            value,
            named,
        },
        op,
    }
}

/// KEYBOARD_OPCODE_INSERT_SHIFT_RIGHT
pub const fn ins(row: u8, col: u8, value: u32) -> Edit {
    edit(row, col, value, true, Op::Insert)
}

/// KEYBOARD_OPCODE_SWAP_ONLY
pub const fn swap(row: u8, col: u8, value: u32) -> Edit {
    edit(row, col, value, true, Op::Swap)
}

/// KEYBOARD_OPCODE_SWAP_ONLY with KEY_EN_UNUSED, which removes the key.
pub const fn unused(row: u8, col: u8) -> Edit {
    edit(row, col, 0, false, Op::Swap)
}

/// KEYBOARD_OPCODE_INSERT_ROW
pub const fn new_row(row: u8, col: u8, value: u32) -> Edit {
    edit(row, col, value, true, Op::InsertRow)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Full,
    Tkl,
    Sixty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Ansi,
    Iso,
    Jis,
}

/// A device's base size, default values and edit keys (keyboard_keymap_overlay_values).
pub struct Overlay {
    pub size: Size,
    pub values: &'static [u32],
    pub edits: &'static [Edit],
}

const MAIN: &[(u8, u8)] = &[
    (1, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (1, 5),
    (1, 6),
    (1, 7),
    (1, 8),
    (1, 9),
    (1, 10),
    (1, 11),
    (1, 12),
    (1, 13),
    (2, 0),
    (2, 1),
    (2, 2),
    (2, 3),
    (2, 4),
    (2, 5),
    (2, 6),
    (2, 7),
    (2, 8),
    (2, 9),
    (2, 10),
    (2, 11),
    (2, 12),
    (2, 13),
    (3, 0),
    (3, 1),
    (3, 2),
    (3, 3),
    (3, 4),
    (3, 5),
    (3, 6),
    (3, 7),
    (3, 8),
    (3, 9),
    (3, 10),
    (3, 11),
    (3, 12),
    (3, 13),
    (4, 0),
    (4, 1),
    (4, 2),
    (4, 3),
    (4, 4),
    (4, 5),
    (4, 6),
    (4, 7),
    (4, 8),
    (4, 9),
    (4, 10),
    (4, 11),
    (4, 13),
    (5, 0),
    (5, 1),
    (5, 2),
    (5, 6),
    (5, 10),
    (5, 11),
    (5, 12),
    (5, 13),
];

const FN_ROW: &[(u8, u8)] = &[
    (0, 0),
    (0, 2),
    (0, 3),
    (0, 4),
    (0, 5),
    (0, 6),
    (0, 7),
    (0, 8),
    (0, 9),
    (0, 10),
    (0, 11),
    (0, 12),
    (0, 13),
];

const EXTRAS: &[(u8, u8)] = &[
    (0, 14),
    (0, 15),
    (0, 16),
    (1, 14),
    (1, 15),
    (1, 16),
    (2, 14),
    (2, 15),
    (2, 16),
    (4, 15),
    (5, 14),
    (5, 15),
    (5, 16),
];

const NUMPAD: &[(u8, u8)] = &[
    (1, 17),
    (1, 18),
    (1, 19),
    (1, 20),
    (2, 17),
    (2, 18),
    (2, 19),
    (2, 20),
    (3, 17),
    (3, 18),
    (3, 19),
    (4, 17),
    (4, 18),
    (4, 19),
    (4, 20),
    (5, 18),
    (5, 19),
];

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    pub keys: Vec<Key>,
    pub rows: u8,
    pub cols: u8,
}

fn base(row: u8, col: u8) -> Key {
    Key {
        row,
        col,
        value: 0,
        named: true,
    }
}

impl Keymap {
    pub fn new(layout: Layout, size: Size, values: &[u32]) -> Self {
        let mut k = Keymap::default();
        let mut zones = vec![MAIN];
        if size != Size::Sixty {
            zones.push(FN_ROW);
            zones.push(EXTRAS);
        }
        if size == Size::Full {
            zones.push(NUMPAD);
        }
        for zone in zones {
            for &(r, c) in zone {
                k.insert(base(r, c));
            }
            k.update();
        }
        for (key, v) in k.keys.iter_mut().zip(values) {
            key.value = *v;
        }
        match layout {
            Layout::Ansi => k.change(&[unused(3, 12), unused(4, 1)]),
            Layout::Iso => k.change(&[unused(2, 13)]),
            Layout::Jis => k.update(),
        }
        if size == Size::Sixty {
            k.remove_row(0);
            k.swap(base(0, 0));
        }
        k.update();
        k
    }

    /// The constructor followed by ChangeKeys with the device's edit keys.
    pub fn build(layout: Layout, overlay: &Overlay) -> Self {
        let mut k = Keymap::new(layout, overlay.size, overlay.values);
        k.change(overlay.edits);
        k
    }

    pub fn change(&mut self, edits: &[Edit]) {
        for e in edits {
            match e.op {
                Op::Insert => self.insert(e.key),
                Op::Swap => self.swap(e.key),
                Op::InsertRow => {
                    if self.insert_row(e.key.row) {
                        self.swap(e.key);
                    }
                }
            }
        }
        self.update();
    }

    fn insert(&mut self, k: Key) {
        let idx = self
            .keys
            .iter()
            .position(|x| k.row < x.row || (k.row == x.row && k.col <= x.col))
            .unwrap_or(self.keys.len());
        if idx == self.keys.len() {
            self.keys.push(k);
            return;
        }
        let mut idx = idx;
        if k.named {
            self.keys.insert(idx, k);
            idx += 1;
        }
        for x in &mut self.keys[idx..] {
            if x.row == k.row && x.col >= k.col {
                x.col += 1;
            }
            if x.row > k.row {
                break;
            }
        }
    }

    fn swap(&mut self, k: Key) {
        if self.keys.is_empty() {
            self.keys.push(k);
            return;
        }
        for i in 0..self.keys.len() {
            let x = self.keys[i];
            if x.row == k.row && x.col == k.col {
                if k.named {
                    self.keys[i].value = k.value;
                    self.keys[i].named = true;
                } else {
                    self.keys.remove(i);
                }
                return;
            }
            if x.row > k.row || (x.row == k.row && x.col > k.col) {
                if k.named {
                    // Inserted one place early when it is not at the front.
                    self.keys.insert(i.saturating_sub(1), k);
                }
                return;
            }
        }
    }

    fn insert_row(&mut self, row: u8) -> bool {
        if row >= self.rows {
            return false;
        }
        if let Some(idx) = self.keys.iter().position(|x| row <= x.row) {
            for x in &mut self.keys[idx..] {
                x.row += 1;
            }
        }
        true
    }

    fn remove_row(&mut self, row: u8) {
        if row >= self.rows {
            return;
        }
        let mut idx = 0;
        while idx < self.keys.len() && row >= self.keys[idx].row {
            if self.keys[idx].row == row {
                self.keys.remove(idx);
            } else {
                idx += 1;
            }
        }
        if self.keys.get(idx).is_some_and(|x| row < x.row) {
            for x in &mut self.keys[idx..] {
                x.row -= 1;
            }
        }
    }

    fn update(&mut self) {
        self.rows = self.keys.iter().map(|k| k.row).max().unwrap_or(0) + 1;
        self.cols = self.keys.iter().map(|k| k.col).max().unwrap_or(0) + 1;
    }

    /// LED positions inside the device rect, in LED order.
    pub fn positions(&self) -> Vec<[f32; 2]> {
        self.keys
            .iter()
            .map(|k| {
                [
                    (k.col as f32 + 0.5) / self.cols as f32,
                    (k.row as f32 + 0.5) / self.rows as f32,
                ]
            })
            .collect()
    }

    pub fn values(&self) -> Vec<u32> {
        self.keys.iter().map(|k| k.value).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // KeyboardLayoutManager constructor: full ANSI is 104 keys, ISO 105, JIS keeps all 106.
    #[test]
    fn base_sizes() {
        assert_eq!(Keymap::new(Layout::Ansi, Size::Full, &[]).keys.len(), 104);
        assert_eq!(Keymap::new(Layout::Iso, Size::Full, &[]).keys.len(), 105);
        assert_eq!(Keymap::new(Layout::Jis, Size::Full, &[]).keys.len(), 106);
        assert_eq!(Keymap::new(Layout::Ansi, Size::Tkl, &[]).keys.len(), 87);
        assert_eq!(Keymap::new(Layout::Ansi, Size::Sixty, &[]).keys.len(), 61);
    }

    // KeyboardLayoutManager constructor: keys are ordered row first, then column.
    #[test]
    fn order_is_row_major() {
        let k = Keymap::new(Layout::Jis, Size::Full, &[]);
        let first: Vec<(u8, u8)> = k.keys.iter().take(17).map(|k| (k.row, k.col)).collect();
        assert_eq!(first[0], (0, 0));
        assert_eq!(first[1], (0, 2));
        assert_eq!(first[15], (0, 16));
        assert_eq!(first[16], (1, 0));
        assert_eq!((k.rows, k.cols), (6, 21));
    }

    // KeyboardLayoutManager size fix for KEYBOARD_SIZE_SIXTY: row 0 removed, Esc swapped onto backtick.
    #[test]
    fn sixty_moves_escape() {
        let k = Keymap::new(Layout::Jis, Size::Sixty, &[41, 30]);
        assert_eq!(k.keys[0], base(0, 0));
        assert_eq!(k.keys[1].value, 30);
        assert_eq!(k.rows, 5);
    }

    // InsertKey: an insert in the middle of a row shifts the rest of that row right.
    #[test]
    fn insert_shifts_row() {
        let mut k = Keymap::new(Layout::Jis, Size::Sixty, &[]);
        k.change(&[ins(1, 0, 7)]);
        assert_eq!(
            k.keys[14],
            Key {
                row: 1,
                col: 0,
                value: 7,
                named: true
            }
        );
        assert_eq!((k.keys[15].row, k.keys[15].col), (1, 1));
        assert_eq!(k.cols, 15);
    }

    // InsertRow checks the row count from before the current edit batch.
    #[test]
    fn insert_row_uses_stale_rows() {
        let mut k = Keymap::new(Layout::Ansi, Size::Sixty, &[]);
        let n = k.keys.len();
        k.change(&[new_row(0, 0, 9), new_row(6, 0, 8)]);
        assert_eq!(k.keys.len(), n + 1);
        assert_eq!(k.keys[0].value, 9);
    }
}
