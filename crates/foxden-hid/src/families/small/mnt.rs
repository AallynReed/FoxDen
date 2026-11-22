// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "mnt-reform";
const ROWS: usize = 6;

/// SendColorMatrix: "xXRGB", the row number, then BGR for each column.
pub fn row_packets(cols: usize, colors: &[Color]) -> Vec<Vec<u8>> {
    (0..ROWS)
        .map(|r| {
            let mut p = b"xXRGB".to_vec();
            p.push(r as u8);
            for c in &colors[r * cols..(r + 1) * cols] {
                p.extend([c.2, c.1, c.0]);
            }
            p
        })
        .collect()
}

struct Mnt {
    cols: usize,
}

impl Proto for Mnt {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        row_packets(self.cols, colors)
            .iter()
            .try_for_each(|p| io.write(p))
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let cols = if info.pid == 0x6D06 { 12 } else { 14 };
    let at: Vec<(u32, u32)> = (0..ROWS * cols)
        .map(|i| ((i % cols) as u32, (i / cols) as u32))
        .collect();
    let dev = cells(
        info,
        FAMILY,
        d,
        DeviceKind::Keyboard,
        cols as u32,
        ROWS as u32,
        &at,
    );
    boxed(api, info, FAMILY, dev, Mnt { cols })
}

pub static DETECTORS: &[Detector] = &[
    det("MNT Reform Keyboard", 0x1209, 0x6D02, 0, 0x01, 0x06, open),
    det(
        "MNT Pocket Reform Keyboard",
        0x1209,
        0x6D06,
        0,
        0x01,
        0x06,
        open,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    // MNTKeyboardController::SendColorMatrix with DeviceUpdateLEDs' BGR order
    #[test]
    fn six_rows() {
        let mut a = vec![Color::BLACK; 72];
        a[12] = Color(1, 2, 3);
        let [first, _, _] = pushes(Mnt { cols: 12 }, 72, vec![], &a, &[]);
        assert_eq!(first.len(), 6);
        let mut row1 = b"xXRGB".to_vec();
        row1.push(1);
        row1.extend([3, 2, 1]);
        row1.resize(6 + 36, 0);
        assert_eq!(first[1], w(row1));
    }
}
