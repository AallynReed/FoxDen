// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::*;
use super::dmi;
use crate::registry::{Detector, Info};
use foxden_core::{Backend, Color, DeviceKind};
use hidapi::HidApi;

const FAMILY: &str = "asus-aura-usb";
const VID: u16 = 0x0B05;
const REPLY_MS: i32 = 1000;
const LEDS_PER_PACKET: u16 = 0x14;
const MODE_DIRECT: u8 = 0xFF;
/// Addressable headers report no size; this is the count assumed.
const DEFAULT_HEADER_LEDS: usize = 60;
/// AURA_ADDRESSABLE_MAX_LEDS.
const MAX_HEADER_LEDS: usize = 120;

/// GetFirmwareVersion and GetConfigTable, which every AuraUSBController sends first.
pub fn base(io: &mut dyn Io) -> Result<([u8; 16], [u8; 60]), String> {
    let mut version = [0u8; 16];
    let mut r = [0u8; 65];
    io.write(&report(&[0xEC, 0x82]))?;
    io.read(&mut r, REPLY_MS)?;
    if r[1] == 0x02 {
        version.copy_from_slice(&r[2..18]);
    }
    let mut r = [0u8; 65];
    io.write(&report(&[0xEC, 0xB0]))?;
    io.read(&mut r, REPLY_MS)?;
    if r[1] != 0x30 {
        return Err("could not read the Aura config table".into());
    }
    let mut config = [0u8; 60];
    config.copy_from_slice(&r[4..64]);
    Ok((version, config))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub effect: u8,
    pub direct: u8,
    pub leds: usize,
    pub fixed: bool,
}

/// AuraMainboardController's device_info.
pub fn mainboard_channels(version: &[u8; 16], config: &[u8; 60]) -> Vec<Channel> {
    let total = config[0x1B];
    let headers = config[0x02];
    let mut effect = 0;
    let mut out = Vec::new();
    if total > 0 {
        let from_zero = version.starts_with(b"AULA3-AR32");
        let board = if from_zero { headers } else { effect };
        out.push(Channel {
            effect: board,
            direct: 0x04,
            leds: total as usize,
            fixed: true,
        });
        if !from_zero {
            effect += 1;
        }
    }
    for i in 0..headers {
        out.push(Channel {
            effect,
            direct: i,
            leds: DEFAULT_HEADER_LEDS,
            fixed: false,
        });
        effect += 1;
    }
    out
}

/// AuraAddressableController's device_info.
pub fn addressable_channels(config: &[u8; 60]) -> Vec<Channel> {
    (0..config[0x02])
        .map(|i| Channel {
            effect: 0x01,
            direct: i,
            leds: DEFAULT_HEADER_LEDS,
            fixed: false,
        })
        .collect()
}

/// AuraUSBController::SendDirect.
pub fn send_direct(device: u8, colors: &[Color]) -> Vec<[u8; 65]> {
    let count = colors.len() as u16;
    let (mut command, mut offset, mut in_packet, mut done) = (device, 0u8, LEDS_PER_PACKET, 0u16);
    let mut out = Vec::new();
    while done < count {
        if done + in_packet < count {
            done += in_packet;
        } else {
            command |= 0x80;
            in_packet = count - done;
            done = count;
        }
        let mut b = report(&[0xEC, 0x40, command, offset, in_packet as u8]);
        let first = (done - in_packet) as usize;
        for (i, c) in colors[first..first + in_packet as usize].iter().enumerate() {
            b[5 + i * 3..8 + i * 3].copy_from_slice(&rgb(*c));
        }
        out.push(b);
        offset = offset.wrapping_add(in_packet as u8);
        if done > 255 {
            command |= 0x10;
            offset = (done - 256) as u8;
        }
    }
    out
}

pub struct Aura {
    pub channels: Vec<Channel>,
    pub mainboard: bool,
}

impl Proto for Aura {
    /// DeviceUpdateMode in Direct, which DeviceUpdateLEDs runs once first.
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        for (i, ch) in self.channels.iter().enumerate() {
            if ch.leds == 0 {
                continue;
            }
            if self.mainboard {
                io.write(&report(&[0xEC, 0x35, ch.effect, 0x00, 0x00, MODE_DIRECT]))?;
            } else {
                io.write(&report(&[0xEC, 0x3B, i as u8, 0x00, MODE_DIRECT]))?;
            }
        }
        Ok(())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        let mut at = 0;
        for ch in &self.channels {
            for p in send_direct(ch.direct, &colors[at..at + ch.leds]) {
                io.write(&p)?;
            }
            at += ch.leds;
        }
        Ok(())
    }
}

/// Gives each addressable header the user's LED count.
pub fn sized(id: &str, channels: &mut [Channel]) {
    for (n, (i, ch)) in channels
        .iter_mut()
        .enumerate()
        .filter(|(_, c)| !c.fixed)
        .enumerate()
    {
        let name = format!("Header {}", n + 1);
        ch.leds = crate::lengths::length(id, i, &name, ch.leds, 0, MAX_HEADER_LEDS);
    }
}

/// The controller constructors: base queries, then SetGen1 on mainboards.
pub fn setup(io: &mut dyn Io, mainboard: bool) -> Result<Vec<Channel>, String> {
    let (version, config) = base(io)?;
    if !mainboard {
        return Ok(addressable_channels(&config));
    }
    io.write(&report(&[0xEC, 0x52, 0x53, 0x00, 0x01]))?;
    Ok(mainboard_channels(&version, &config))
}

fn build(
    api: &HidApi,
    info: &Info,
    d: &Detector,
    name: Option<String>,
    mainboard: bool,
) -> Result<Box<dyn Backend>, String> {
    let mut io = opened(api, info)?;
    let mut channels = setup(io.as_mut(), mainboard)?;
    if channels.is_empty() {
        return Err("no Aura channels".into());
    }
    sized(&info.device_id(FAMILY), &mut channels);
    let zones: Vec<usize> = channels.iter().map(|c| c.leds).collect();
    let mut dev = stacked(info, FAMILY, d, DeviceKind::Motherboard, &zones);
    if let Some(name) = name {
        dev.name = name;
    }
    let proto = Aura {
        channels,
        mainboard,
    };
    Ok(Box::new(Driver::new(io, dev, FAMILY, proto)))
}

fn board(suffix: &str) -> Option<String> {
    dmi::board().map(|b| format!("ASUS {b}{suffix}"))
}

fn addressable(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    build(api, info, d, board(" Addressable"), false)
}

fn terminal(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    build(api, info, d, None, false)
}

fn motherboard(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    build(api, info, d, board(""), true)
}

fn cooler(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    build(api, info, d, None, true)
}

pub static DETECTORS: &[Detector] = &[
    det(
        "ASUS Aura Addressable",
        VID,
        0x1867,
        ANY,
        ANY,
        ANY,
        addressable,
    ),
    det(
        "ASUS Aura Addressable",
        VID,
        0x1872,
        ANY,
        ANY,
        ANY,
        addressable,
    ),
    det(
        "ASUS Aura Addressable",
        VID,
        0x18A3,
        ANY,
        ANY,
        ANY,
        addressable,
    ),
    det(
        "ASUS Aura Addressable",
        VID,
        0x18A5,
        ANY,
        ANY,
        ANY,
        addressable,
    ),
    det("ASUS ROG Strix LC III", VID, 0x1B29, ANY, ANY, ANY, cooler),
    det(
        "ASUS Aura Motherboard",
        VID,
        0x18F3,
        ANY,
        ANY,
        ANY,
        motherboard,
    ),
    det(
        "ASUS Aura Motherboard",
        VID,
        0x1939,
        ANY,
        ANY,
        ANY,
        motherboard,
    ),
    det(
        "ASUS Aura Motherboard",
        VID,
        0x19AF,
        ANY,
        0xFF72,
        0x00A1,
        motherboard,
    ),
    det(
        "ASUS Aura Motherboard",
        VID,
        0x1AA6,
        ANY,
        ANY,
        ANY,
        motherboard,
    ),
    det(
        "ASUS Aura Motherboard",
        VID,
        0x1BED,
        ANY,
        ANY,
        ANY,
        motherboard,
    ),
    det(
        "ASUS ROG AURA Terminal",
        VID,
        0x1889,
        ANY,
        ANY,
        ANY,
        terminal,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use rec::*;

    fn replies(version: &[u8], config: &[(usize, u8)]) -> Vec<Vec<u8>> {
        let mut v = vec![0xEC, 0x02];
        v.extend_from_slice(version);
        let mut c = vec![0u8; 65];
        c[..2].copy_from_slice(&[0xEC, 0x30]);
        for &(i, x) in config {
            c[4 + i] = x;
        }
        vec![pad(&v, 65), c]
    }

    // GetFirmwareVersion, GetConfigTable, then the mainboard device_info
    #[test]
    fn config_table_gives_channels() {
        let mut rec = Rec::with(replies(
            b"AULA3-6K75-0207",
            &[(0x1B, 5), (0x1D, 1), (0x02, 2)],
        ));
        let ch = setup(&mut rec, true).unwrap();
        assert_eq!(
            rec.take(),
            vec![
                w(pad(&[0xEC, 0x82], 65)),
                Op::Read,
                w(pad(&[0xEC, 0xB0], 65)),
                Op::Read,
                w(pad(&[0xEC, 0x52, 0x53, 0, 1], 65)),
            ]
        );
        let mut rec = Rec::with(replies(b"AULA3-6K75-0207", &[(0x1B, 5), (0x02, 2)]));
        assert_eq!(setup(&mut rec, false).unwrap().len(), 2);
        assert_eq!(rec.take().len(), 4);
        let (_, config) = base(&mut Rec::with(replies(b"", &[(0x1B, 5), (0x02, 2)]))).unwrap();
        assert_eq!(ch.len(), 3);
        assert_eq!((ch[0].effect, ch[0].direct, ch[0].leds), (0, 4, 5));
        assert_eq!((ch[1].effect, ch[1].direct), (1, 0));
        assert_eq!((ch[2].effect, ch[2].direct), (2, 1));
        let ar32 = mainboard_channels(
            &{
                let mut v = [0u8; 16];
                v[..10].copy_from_slice(b"AULA3-AR32");
                v
            },
            &config,
        );
        assert_eq!((ar32[0].effect, ar32[1].effect, ar32[2].effect), (2, 0, 1));
        let mut bad = Rec::with(vec![vec![0xEC, 0x02], vec![0xEC, 0x31]]);
        assert!(base(&mut bad).is_err());
    }

    #[test]
    fn a_set_length_sizes_the_header() {
        let config = {
            let mut c = [0u8; 60];
            (c[0x1B], c[0x02]) = (5, 2);
            c
        };
        let mut ch = mainboard_channels(&[0; 16], &config);
        crate::lengths::set_one("hid:aura-test", 2, 200);
        crate::lengths::set_one("hid:aura-test", 1, 12);
        sized("hid:aura-test", &mut ch);
        assert_eq!(ch.iter().map(|c| c.leds).collect::<Vec<_>>(), [5, 12, 120]);
        let zones: Vec<_> = crate::lengths::resizable()
            .into_iter()
            .filter(|z| z.device == "hid:aura-test")
            .map(|z| (z.index, z.name))
            .collect();
        assert_eq!(zones, [(1, "Header 1".into()), (2, "Header 2".into())]);
    }

    // SendDirect: 20 LEDs per packet, apply flag on the last
    #[test]
    fn direct_packets() {
        let colors: Vec<Color> = (0..25u8).map(|i| Color(i, 1, 2)).collect();
        let p = send_direct(0x04, &colors);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0][..8], [0xEC, 0x40, 0x04, 0x00, 0x14, 0, 1, 2]);
        assert_eq!(p[0][62..65], [19, 1, 2]);
        assert_eq!(p[1][..8], [0xEC, 0x40, 0x84, 0x14, 0x05, 20, 1, 2]);
        assert_eq!(p[1][20..23], [0, 0, 0]);
        let long = send_direct(0x00, &vec![Color::BLACK; 270]);
        assert_eq!(long[12][2..5], [0x00, 0xF0, 0x14]);
        assert_eq!(long[13][2..5], [0x90, 0x04, 0x0A]);
    }

    // DeviceUpdateMode(Direct) once, then SetChannelLEDs per zone
    #[test]
    fn mainboard_start_and_frame() {
        let channels = vec![
            Channel {
                effect: 0,
                direct: 4,
                leds: 2,
                fixed: true,
            },
            Channel {
                effect: 1,
                direct: 0,
                leds: 1,
                fixed: false,
            },
        ];
        let a = [Color(1, 2, 3), Color(4, 5, 6), Color(7, 8, 9)];
        let [first, _, _] = pushes(
            Aura {
                channels: channels.clone(),
                mainboard: true,
            },
            3,
            vec![],
            &a,
            &[],
        );
        assert_eq!(
            first,
            vec![
                w(pad(&[0xEC, 0x35, 0, 0, 0, 0xFF], 65)),
                w(pad(&[0xEC, 0x35, 1, 0, 0, 0xFF], 65)),
                w(pad(&[0xEC, 0x40, 0x84, 0, 2, 1, 2, 3, 4, 5, 6], 65)),
                w(pad(&[0xEC, 0x40, 0x80, 0, 1, 7, 8, 9], 65)),
            ]
        );
        let [first, _, _] = pushes(
            Aura {
                channels,
                mainboard: false,
            },
            3,
            vec![],
            &a,
            &[],
        );
        assert_eq!(first[0], w(pad(&[0xEC, 0x3B, 0, 0, 0xFF], 65)));
        assert_eq!(first[1], w(pad(&[0xEC, 0x3B, 1, 0, 0xFF], 65)));
    }
}
