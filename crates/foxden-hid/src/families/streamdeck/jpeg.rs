// SPDX-License-Identifier: GPL-2.0-or-later
//!
//! A baseline JPEG of one colour, byte for byte what OpenRGB's `stbi_write_jpg_to_func` writes at
//! quality 95 (4:4:4, standard Huffman tables): every 8x8 block has only a DC value.

const ZIGZAG: [usize; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11,
    18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20, 22, 33, 38, 46, 51, 55, 60, 21, 34,
    37, 47, 50, 56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];

const YQT: [u32; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

const UVQT: [u32; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

const DC_LUM_COUNTS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_CHR_COUNTS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUM_COUNTS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUM_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
const AC_CHR_COUNTS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHR_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

/// (code, length) per DC size category.
const YDC_HT: [(u32, u32); 12] = [
    (0, 2),
    (2, 3),
    (3, 3),
    (4, 3),
    (5, 3),
    (6, 3),
    (14, 4),
    (30, 5),
    (62, 6),
    (126, 7),
    (254, 8),
    (510, 9),
];
const UVDC_HT: [(u32, u32); 12] = [
    (0, 2),
    (1, 2),
    (2, 2),
    (6, 3),
    (14, 4),
    (30, 5),
    (62, 6),
    (126, 7),
    (254, 8),
    (510, 9),
    (1022, 10),
    (2046, 11),
];
const Y_EOB: (u32, u32) = (10, 4);
const UV_EOB: (u32, u32) = (0, 2);

/// stb's quality 95 scale: `200 - 2 * 95`.
const SCALE: u32 = 10;

fn table(base: &[u32; 64]) -> [u8; 64] {
    let mut t = [0u8; 64];
    for (i, &b) in base.iter().enumerate() {
        t[ZIGZAG[i]] = ((b * SCALE + 50) / 100).clamp(1, 255) as u8;
    }
    t
}

struct Bits {
    out: Vec<u8>,
    buf: u32,
    cnt: u32,
}

impl Bits {
    fn put(&mut self, (code, len): (u32, u32)) {
        self.cnt += len;
        self.buf |= code << (24 - self.cnt);
        while self.cnt >= 8 {
            let c = (self.buf >> 16) as u8;
            self.out.push(c);
            if c == 0xFF {
                self.out.push(0);
            }
            self.buf <<= 8;
            self.cnt -= 8;
        }
    }
}

/// `stbiw__jpg_calcBits`: (value bits, size category).
fn calc_bits(val: i32) -> (u32, u32) {
    let mut mag = val.unsigned_abs();
    let v = if val < 0 { val - 1 } else { val };
    let mut len = 1;
    while {
        mag >>= 1;
        mag != 0
    } {
        len += 1;
    }
    ((v & ((1 << len) - 1)) as u32, len)
}

/// The quantised DC of a block of `v` (already level-shifted), in stb's f32 arithmetic.
fn dc(v: f32, q: u8) -> i32 {
    #[allow(clippy::excessive_precision)]
    let aasf0 = 2.828427125f32;
    let fd = 1.0f32 / (q as f32 * aasf0 * aasf0);
    // Both DCT passes just sum eight equal values.
    let x = v * 64.0 * fd;
    (if x < 0.0 { x - 0.5 } else { x + 0.5 }) as i32
}

/// `stbiw__jpg_processDU` for a flat block.
fn block(bits: &mut Bits, value: i32, pred: i32, dc_ht: &[(u32, u32); 12], eob: (u32, u32)) {
    let diff = value - pred;
    if diff == 0 {
        bits.put(dc_ht[0]);
    } else {
        let (v, len) = calc_bits(diff);
        bits.put(dc_ht[len as usize]);
        bits.put((v, len));
    }
    bits.put(eob);
}

/// A `width` x `height` JPEG filled with `(r, g, b)`; both sides multiples of 8.
pub fn solid(width: u16, height: u16, r: u8, g: u8, b: u8) -> Vec<u8> {
    let yt = table(&YQT);
    let uvt = table(&UVQT);
    let mut out = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
        0xFF, 0xDB, 0, 0x84, 0,
    ];
    out.extend_from_slice(&yt);
    out.push(1);
    out.extend_from_slice(&uvt);
    let [hh, hl] = height.to_be_bytes();
    let [wh, wl] = width.to_be_bytes();
    out.extend_from_slice(&[
        0xFF, 0xC0, 0, 0x11, 8, hh, hl, wh, wl, 3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xC4,
        0x01, 0xA2, 0,
    ]);
    out.extend_from_slice(&DC_LUM_COUNTS);
    out.extend_from_slice(&DC_VALUES);
    out.push(0x10);
    out.extend_from_slice(&AC_LUM_COUNTS);
    out.extend_from_slice(&AC_LUM_VALUES);
    out.push(1);
    out.extend_from_slice(&DC_CHR_COUNTS);
    out.extend_from_slice(&DC_VALUES);
    out.push(0x11);
    out.extend_from_slice(&AC_CHR_COUNTS);
    out.extend_from_slice(&AC_CHR_VALUES);
    out.extend_from_slice(&[0xFF, 0xDA, 0, 0xC, 3, 1, 0, 2, 0x11, 3, 0x11, 0, 0x3F, 0]);

    let (r, g, b) = (r as f32, g as f32, b as f32);
    let y = dc(
        0.29900f32 * r + 0.58700f32 * g + 0.11400f32 * b - 128.0,
        yt[0],
    );
    let u = dc(-0.16874f32 * r - 0.33126f32 * g + 0.50000f32 * b, uvt[0]);
    let v = dc(0.50000f32 * r - 0.41869f32 * g - 0.08131f32 * b, uvt[0]);

    let mut bits = Bits {
        out,
        buf: 0,
        cnt: 0,
    };
    let blocks = (width as usize / 8) * (height as usize / 8);
    for i in 0..blocks {
        let first = i == 0;
        let pred = |x: i32| if first { 0 } else { x };
        block(&mut bits, y, pred(y), &YDC_HT, Y_EOB);
        block(&mut bits, u, pred(u), &UVDC_HT, UV_EOB);
        block(&mut bits, v, pred(v), &UVDC_HT, UV_EOB);
    }
    bits.put((0x7F, 7));
    let mut out = bits.out;
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Huff(HashMap<(u32, u32), u8>);

    impl Huff {
        fn new(counts: &[u8], values: &[u8]) -> Huff {
            let mut map = HashMap::new();
            let (mut code, mut k) = (0u32, 0);
            for (len, &n) in counts.iter().enumerate() {
                for _ in 0..n {
                    map.insert((len as u32 + 1, code), values[k]);
                    code += 1;
                    k += 1;
                }
                code <<= 1;
            }
            Huff(map)
        }
    }

    struct Reader<'a> {
        data: &'a [u8],
        pos: usize,
        bit: u32,
    }

    impl Reader<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.data[self.pos] >> (7 - self.bit)) & 1;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
            b as u32
        }

        fn bits(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, _| (v << 1) | self.bit())
        }

        fn symbol(&mut self, h: &Huff) -> u8 {
            let mut code = 0;
            for len in 1..=16 {
                code = (code << 1) | self.bit();
                if let Some(&v) = h.0.get(&(len, code)) {
                    return v;
                }
            }
            panic!("bad Huffman code");
        }
    }

    struct Decoded {
        markers: Vec<u8>,
        width: u16,
        height: u16,
        pixel: [f32; 3],
        first_dc: [i32; 3],
    }

    /// A baseline decoder for flat images: walks the segments, then decodes every block of the scan.
    fn decode(jpeg: &[u8]) -> Decoded {
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        let mut markers = vec![0xD8];
        let mut qt: HashMap<u8, Vec<u8>> = HashMap::new();
        let mut ht: HashMap<u8, Huff> = HashMap::new();
        let mut comps: Vec<(u8, u8)> = Vec::new();
        let mut sel: Vec<u8> = Vec::new();
        let (mut width, mut height) = (0, 0);
        let mut p = 2;
        loop {
            assert_eq!(jpeg[p], 0xFF);
            let m = jpeg[p + 1];
            markers.push(m);
            let len = u16::from_be_bytes([jpeg[p + 2], jpeg[p + 3]]) as usize;
            let seg = &jpeg[p + 4..p + 2 + len];
            match m {
                0xDB => {
                    for t in seg.chunks(65) {
                        assert_eq!(t.len(), 65);
                        qt.insert(t[0], t[1..].to_vec());
                    }
                }
                0xC0 => {
                    assert_eq!(seg[0], 8);
                    height = u16::from_be_bytes([seg[1], seg[2]]);
                    width = u16::from_be_bytes([seg[3], seg[4]]);
                    for c in seg[6..].chunks(3).take(seg[5] as usize) {
                        assert_eq!(c[1], 0x11);
                        comps.push((c[0], c[2]));
                    }
                }
                0xC4 => {
                    let mut s = seg;
                    while !s.is_empty() {
                        let n: usize = s[1..17].iter().map(|&c| c as usize).sum();
                        ht.insert(s[0], Huff::new(&s[1..17], &s[17..17 + n]));
                        s = &s[17 + n..];
                    }
                }
                0xDA => {
                    for c in seg[1..].chunks(2).take(seg[0] as usize) {
                        sel.push(c[1]);
                    }
                    assert_eq!(&seg[seg.len() - 3..], &[0, 63, 0]);
                }
                _ => {}
            }
            p += 2 + len;
            if m == 0xDA {
                break;
            }
        }
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9]);
        let mut data = Vec::new();
        let mut i = p;
        while i < jpeg.len() - 2 {
            data.push(jpeg[i]);
            if jpeg[i] == 0xFF {
                assert_eq!(jpeg[i + 1], 0, "unstuffed 0xFF in scan");
                i += 1;
            }
            i += 1;
        }
        markers.push(0xD9);

        let mut r = Reader {
            data: &data,
            pos: 0,
            bit: 0,
        };
        let mut pred = [0i32; 3];
        let mut first_dc = [0i32; 3];
        let blocks = (width as usize / 8) * (height as usize / 8);
        for b in 0..blocks {
            for c in 0..3 {
                let s = r.symbol(&ht[&(sel[c] >> 4)]) as u32;
                let mut diff = r.bits(s) as i32;
                if s > 0 && diff < 1 << (s - 1) {
                    diff -= (1 << s) - 1;
                }
                if b > 0 {
                    assert_eq!(diff, 0, "block {b} component {c}");
                }
                pred[c] += diff;
                if b == 0 {
                    first_dc[c] = pred[c];
                }
                assert_eq!(r.symbol(&ht[&(0x10 | (sel[c] & 15))]), 0, "AC not EOB");
            }
        }
        let tail = data.len() * 8 - (r.pos * 8 + r.bit as usize);
        assert!(tail < 8);
        assert_eq!(r.bits(tail as u32), (1 << tail) - 1, "fill bits");

        let level = |c: usize| pred[c] as f32 * qt[&comps[c].1][0] as f32 / 8.0;
        let (y, cb, cr) = (level(0) + 128.0, level(1), level(2));
        let pixel = [
            y + 1.402 * cr,
            y - 0.344_136 * cb - 0.714_136 * cr,
            y + 1.772 * cb,
        ];
        Decoded {
            markers,
            width,
            height,
            pixel,
            first_dc,
        }
    }

    #[test]
    fn markers_and_dimensions() {
        let d = decode(&solid(72, 72, 10, 20, 30));
        assert_eq!(d.markers, vec![0xD8, 0xE0, 0xDB, 0xC0, 0xC4, 0xDA, 0xD9]);
        assert_eq!((d.width, d.height), (72, 72));
    }

    #[test]
    fn headers_match_stb_quality_95() {
        let j = solid(72, 72, 0, 0, 0);
        assert_eq!(
            &j[..25],
            &[
                0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0,
                0, 0xFF, 0xDB, 0, 0x84, 0
            ]
        );
        // (base * 10 + 50) / 100, stored in zigzag order.
        assert_eq!(&j[25..33], &[2, 1, 1, 1, 1, 1, 2, 1]);
        assert_eq!(j[89], 1);
        assert_eq!(&j[90..94], &[2, 2, 2, 2]);
        assert_eq!(j[153], 10);
        assert_eq!(
            &j[154..178],
            &[
                0xFF, 0xC0, 0, 0x11, 8, 0, 72, 0, 72, 3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF,
                0xC4, 0x01, 0xA2, 0
            ]
        );
        let sos = 178 + 0x1A2 - 3;
        assert_eq!(
            &j[sos..sos + 14],
            &[0xFF, 0xDA, 0, 0xC, 3, 1, 0, 2, 0x11, 3, 0x11, 0, 0x3F, 0]
        );
    }

    #[test]
    fn black_scan_golden() {
        // Y DC -512: category 10 {254,8} + bits 0111111111, EOB 1010; U and V: 00, EOB 00.
        let j = solid(72, 72, 0, 0, 0);
        let scan = 178 + 0x1A2 - 3 + 14;
        assert_eq!(&j[scan..scan + 3], &[0xFE, 0x7F, 0xE8]);
        assert_eq!(decode(&j).first_dc, [-512, 0, 0]);
    }

    #[test]
    fn decodes_back_to_the_colour() {
        for rgb in [
            [0u8, 0, 0],
            [255, 255, 255],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [12, 200, 99],
            [128, 64, 250],
        ] {
            let d = decode(&solid(72, 72, rgb[0], rgb[1], rgb[2]));
            for c in 0..3 {
                let got = d.pixel[c].clamp(0.0, 255.0);
                assert!((got - rgb[c] as f32).abs() <= 1.5, "{rgb:?}: {:?}", d.pixel);
            }
        }
    }

    #[test]
    fn dc_rounds_half_away_from_zero() {
        assert_eq!(dc(-128.0, 2), -512);
        assert_eq!(dc(0.0, 2), 0);
        assert_eq!(dc(127.0, 2), 508);
        assert_eq!(calc_bits(-512), (511, 10));
        assert_eq!(calc_bits(3), (3, 2));
        assert_eq!(calc_bits(-1), (0, 1));
    }

    #[test]
    fn ff_bytes_are_stuffed() {
        let mut b = Bits {
            out: Vec::new(),
            buf: 0,
            cnt: 0,
        };
        b.put((0xFF, 8));
        b.put((0x7F, 7));
        assert_eq!(b.out, vec![0xFF, 0x00]);
    }
}
