//! Elgato Stream Deck original, V2 and XL (protocol learned from SignalRGB's pre-release Streamdeck_0x0060.js, Streamdeck.js and Streamdeck-XL_0x006c.js).

use super::jpeg;
use crate::hid::{Driver, Io, Proto, any, open_io};
use foxden_core::{Backend, Color, Device, DeviceKind};
use foxden_hid::registry::{Detector, Info};
use hidapi::HidApi;

const FAMILY: &str = "streamdeck";
const VID: u16 = 0x0FD9;
const GEN2_REPORT: usize = 1024;
const GEN2_HEADER: usize = 8;
const GEN2_IMAGE: u8 = 0x07;
const V1_REPORT: usize = 8191;
const V1_HEADER: usize = 16;
const V1_SPLIT: usize = 7804;
const V1_KEY_PX: u32 = 72;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Model {
    Original,
    V2,
    Xl,
}

impl Model {
    fn cols(self) -> u32 {
        if self == Model::Xl { 8 } else { 5 }
    }
    fn rows(self) -> u32 {
        if self == Model::Xl { 4 } else { 3 }
    }
    fn key_px(self) -> u16 {
        if self == Model::Xl { 96 } else { 72 }
    }

    /// The firmware key for LED `i` (row-major, left to right); the original numbers keys right to left.
    fn key(self, i: usize) -> u8 {
        let c = self.cols() as usize;
        match self {
            Model::Original => ((i / c) * c + (c - 1 - i % c)) as u8,
            _ => i as u8,
        }
    }

    fn reports(self, key: u8, c: Color) -> Vec<(Vec<u8>, usize)> {
        match self {
            Model::Original => v1_reports(key, &bmp(c))
                .into_iter()
                .map(|r| (r, V1_REPORT))
                .collect(),
            _ => gen2_reports(
                key,
                &jpeg::solid(self.key_px(), self.key_px(), c.0, c.1, c.2),
            )
            .into_iter()
            .map(|r| (r, GEN2_REPORT))
            .collect(),
        }
    }
}

/// sendZone (V2/XL): `[0x02, 0x07, key, last, length LE, page LE]` then up to 1016 image bytes.
fn gen2_reports(key: u8, image: &[u8]) -> Vec<Vec<u8>> {
    let chunks: Vec<_> = image.chunks(GEN2_REPORT - GEN2_HEADER).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(page, c)| {
            let last = (page + 1 == chunks.len()) as u8;
            let [l0, l1] = (c.len() as u16).to_le_bytes();
            let [p0, p1] = (page as u16).to_le_bytes();
            let mut r = vec![0x02, GEN2_IMAGE, key, last, l0, l1, p0, p1];
            r.extend_from_slice(c);
            r
        })
        .collect()
}

/// grabColors (original): a 72x72 24-bit bottom-up BMP of one colour, BGR.
fn bmp(c: Color) -> Vec<u8> {
    let pixels = (V1_KEY_PX * V1_KEY_PX * 3) as usize;
    let mut b = Vec::with_capacity(54 + pixels);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(54 + pixels as u32).to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0]);
    b.extend_from_slice(&V1_KEY_PX.to_le_bytes());
    b.extend_from_slice(&V1_KEY_PX.to_le_bytes());
    b.extend_from_slice(&[1, 0, 24, 0, 0, 0, 0, 0]);
    b.extend_from_slice(&(pixels as u32).to_le_bytes());
    b.extend_from_slice(&[0x13, 0x0E, 0, 0, 0x13, 0x0E, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for _ in 0..V1_KEY_PX * V1_KEY_PX {
        b.extend_from_slice(&[c.2, c.1, c.0]);
    }
    b
}

/// sendZone (original): the image in two pages, `[0x02, 0x01, page, 0, last, key + 1, ...]`.
fn v1_reports(key: u8, image: &[u8]) -> Vec<Vec<u8>> {
    let (a, b) = image.split_at(V1_SPLIT.min(image.len()));
    [(1u8, 0u8, a), (2, 1, b)]
        .into_iter()
        .map(|(page, last, part)| {
            let mut r = vec![0u8; V1_HEADER];
            r[..6].copy_from_slice(&[0x02, 0x01, page, 0x00, last, key + 1]);
            r.extend_from_slice(part);
            r
        })
        .collect()
}

struct Deck {
    model: Model,
    sent: Vec<Option<Color>>,
}

impl Proto for Deck {
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for (i, &c) in colors.iter().enumerate() {
            if self.sent[i] == Some(c) {
                continue;
            }
            self.sent[i] = None;
            for (r, len) in self.model.reports(self.model.key(i), c) {
                io.write(&r, len)?;
            }
            self.sent[i] = Some(c);
        }
        Ok(())
    }
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let model = match d.pid {
        0x0060 => Model::Original,
        0x006C => Model::Xl,
        _ => Model::V2,
    };
    let dev = Device::grid(
        &info.device_id(FAMILY),
        d.name,
        DeviceKind::Other,
        [0.34, 0.12, 0.20, 0.12],
        model.cols(),
        model.rows(),
    );
    let deck = Deck {
        model,
        sent: vec![None; dev.leds.len()],
    };
    Ok(Driver::new(open_io(api, info)?, dev, FAMILY, deck).boxed())
}

pub static DETECTORS: &[Detector] = &[
    any("Elgato Stream Deck", VID, 0x0060, open),
    any("Elgato Stream Deck V2", VID, 0x006D, open),
    any("Elgato Stream Deck XL", VID, 0x006C, open),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_pages() {
        let img = bmp(Color(1, 2, 3));
        assert_eq!(img.len(), 15606);
        assert_eq!(img[54..57], [3, 2, 1]);
        let r = v1_reports(Model::Original.key(0), &img);
        assert_eq!(r[0][..6], [0x02, 0x01, 0x01, 0x00, 0x00, 5]);
        assert_eq!(r[1][..6], [0x02, 0x01, 0x02, 0x00, 0x01, 5]);
        assert_eq!(r[0].len(), V1_HEADER + V1_SPLIT);
        assert!(r[1].len() <= V1_REPORT);
    }

    #[test]
    fn gen2_header() {
        let img = vec![7u8; 1100];
        let r = gen2_reports(31, &img);
        assert_eq!(r[0][..8], [0x02, 0x07, 31, 0, 0xF8, 0x03, 0, 0]);
        assert_eq!(r[1][..8], [0x02, 0x07, 31, 1, 84, 0, 1, 0]);
        assert_eq!(Model::Xl.key(9), 9);
        assert_eq!(Model::Original.key(6), 8);
    }
}
