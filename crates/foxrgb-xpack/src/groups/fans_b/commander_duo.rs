//! Corsair iCUE Commander DUO ARGB lighting.

use super::util::{ask, u16le};
use crate::hid::{Driver, Io, Proto, open_io};
use foxrgb_core::{Backend, Color, Device, DeviceKind};
use foxrgb_hid::port::LIGHTS;
use foxrgb_hid::registry::{Detector, Info};
use hidapi::HidApi;

pub const PID: u16 = 0x0C56;
const VID: u16 = 0x1B1C;
const LEN: usize = 65;
const READ_LEN: usize = 64;
const REPLY_MS: u64 = 500;
const CHANNELS: usize = 2;
const MAX_LEDS: usize = 50;
/// Data bytes after the two command bytes of a color packet.
const CHUNK: usize = 61;
const SOFTWARE: [u8; 4] = [0x01, 0x03, 0x00, 0x02];
const HARDWARE: [u8; 4] = [0x01, 0x03, 0x00, 0x01];
const LEDS_EP: u8 = 0x20;
const COLOR_EP: u8 = 0x22;
const CONNECTED: u16 = 2;

/// `00 08`, then the command.
pub fn cmd(c: &[u8]) -> Vec<u8> {
    let mut p = vec![0x00, 0x08];
    p.extend_from_slice(c);
    p
}

/// LEDs on each ARGB port from the 0x20 endpoint: status then count, per port.
pub fn led_counts(r: &[u8]) -> Vec<usize> {
    (0..CHANNELS)
        .map(|i| match u16le(r, 6 + 4 * i) {
            Some(CONNECTED) => u16le(r, 8 + 4 * i).map_or(0, usize::from).min(MAX_LEDS),
            _ => 0,
        })
        .collect()
}

/// Length, `00 00 12 00`, then RGB, split into a `06 00` packet and `07 00` packets.
pub fn color_packets(colors: &[Color]) -> Vec<Vec<u8>> {
    let rgb: Vec<u8> = colors.iter().flat_map(|c| [c.0, c.1, c.2]).collect();
    let mut data = ((rgb.len() + 2) as u16).to_le_bytes().to_vec();
    data.extend([0x00, 0x00, 0x12, 0x00]);
    data.extend(rgb);
    data.chunks(CHUNK)
        .enumerate()
        .map(|(i, chunk)| {
            let mut c = vec![if i == 0 { 0x06 } else { 0x07 }, 0x00];
            c.extend_from_slice(chunk);
            cmd(&c)
        })
        .collect()
}

fn exchange(io: &mut dyn Io, packet: &[u8]) -> Result<Vec<u8>, String> {
    let first = packet[2];
    ask(io, packet, LEN, READ_LEN, REPLY_MS, |r| {
        r.get(1) == Some(&first)
    })
}

fn read_leds(io: &mut dyn Io) -> Option<Vec<usize>> {
    exchange(io, &cmd(&[0x0D, 0x01, LEDS_EP])).ok()?;
    let r = exchange(io, &cmd(&[0x08, 0x01]));
    let _ = exchange(io, &cmd(&[0x05, 0x01, 0x01]));
    Some(led_counts(&r.ok()?)).filter(|c| c.iter().sum::<usize>() > 0)
}

fn fans_active(io: &dyn Io) -> bool {
    io.port().is_some_and(|p| p.device().others_active(LIGHTS))
}

/// Hardware mode hands the fans back too, so while they are driven it is left to them.
fn hand_back(io: &mut dyn Io) {
    if fans_active(io) {
        if let Some(p) = io.port() {
            p.device().owe(true);
        }
        return;
    }
    let _ = exchange(io, &cmd(&HARDWARE));
}

struct Lights;

impl Proto for Lights {
    fn start(&mut self, io: &mut dyn Io) -> Result<(), String> {
        exchange(io, &cmd(&SOFTWARE))?;
        let _ = exchange(io, &cmd(&[0x05, 0x01, 0x01, COLOR_EP]));
        exchange(io, &cmd(&[0x0D, 0x00, COLOR_EP])).map(|_| ())
    }
    fn frame(&mut self, io: &mut dyn Io, colors: &[Color]) -> Result<(), String> {
        for p in color_packets(colors) {
            exchange(io, &p)?;
        }
        Ok(())
    }
    fn keepalive_ms(&self) -> Option<u64> {
        Some(10_000)
    }
    fn stop(&mut self, io: &mut dyn Io) {
        hand_back(io);
    }
}

/// The ports' LEDs, read in software mode when hardware mode refuses.
fn counts(io: &mut dyn Io) -> Option<Vec<usize>> {
    if let Some(c) = read_leds(io) {
        return Some(c);
    }
    exchange(io, &cmd(&SOFTWARE)).ok()?;
    let c = read_leds(io);
    hand_back(io);
    c
}

fn layout(info: &Info, name: &str, counts: &[usize]) -> Device {
    let widest = counts.iter().copied().max().unwrap_or(1).max(1);
    let mut dev = Device::grid(
        &info.device_id("corsair-commander-duo"),
        name,
        DeviceKind::Other,
        [0.30, 0.40, 0.40, 0.10],
        widest as u32,
        CHANNELS as u32,
    );
    dev.leds = counts
        .iter()
        .enumerate()
        .flat_map(|(ch, &n)| {
            (0..n).map(move |i| {
                [
                    (i as f32 + 0.5) / n as f32,
                    (ch as f32 + 0.5) / CHANNELS as f32,
                ]
            })
        })
        .collect();
    dev
}

fn open(api: &HidApi, info: &Info, d: &Detector) -> Result<Box<dyn Backend>, String> {
    let mut io = open_io(api, info)?;
    let counts = counts(io.as_mut()).ok_or("no ARGB devices found")?;
    let dev = layout(info, d.name, &counts);
    Ok(Driver::new(io, dev, "corsair commander duo", Lights).boxed())
}

pub static DETECTORS: [Detector; 1] = [Detector {
    interface: Some(0),
    usage: Some(0x01),
    ..crate::hid::on_p(
        "Corsair iCUE Commander DUO",
        VID,
        PID,
        0xFF42,
        Some(0x01),
        open,
    )
}];

#[cfg(test)]
mod tests {
    use super::*;
    use foxrgb_hid::port::{FANS, HidResult, Port, Raw};
    use std::sync::{Arc, Mutex};

    /// Records writes and answers each with its command byte echoed.
    #[derive(Clone, Default)]
    struct Echo(Arc<Mutex<Seen>>);

    type Seen = (Vec<Vec<u8>>, Option<u8>);

    impl Raw for Echo {
        fn write(&self, data: &[u8]) -> HidResult<usize> {
            let mut s = self.0.lock().unwrap();
            s.0.push(data.to_vec());
            s.1 = data.get(2).copied();
            Ok(data.len())
        }
        fn read_timeout(&self, buf: &mut [u8], _: i32) -> HidResult<usize> {
            let Some(c) = self.0.lock().unwrap().1.take() else {
                return Ok(0);
            };
            buf.fill(0);
            buf[1] = c;
            Ok(buf.len())
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            Ok(())
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
        fn get_input_report(&self, _: &mut [u8]) -> HidResult<usize> {
            Ok(0)
        }
    }

    #[test]
    fn led_counts_per_port() {
        let mut r = vec![0u8; 64];
        r[6] = 2;
        r[8] = 21;
        r[10] = 1;
        r[12] = 30;
        assert_eq!(led_counts(&r), [21, 0]);
        r[10] = 2;
        r[12] = 80;
        assert_eq!(led_counts(&r), [21, 50]);
    }

    #[test]
    fn color_packets_split_at_61_bytes() {
        let colors: Vec<Color> = (0..25).map(|i| Color(i, 0x10, 0x20)).collect();
        let p = color_packets(&colors);
        assert_eq!(p.len(), 2);
        assert_eq!(
            &p[0][..10],
            &[0x00, 0x08, 0x06, 0x00, 77, 0, 0, 0, 0x12, 0x00]
        );
        assert_eq!(&p[0][10..13], &[0, 0x10, 0x20]);
        assert_eq!(p[0].len(), 2 + 2 + CHUNK);
        assert_eq!(&p[1][..4], &[0x00, 0x08, 0x07, 0x00]);
        assert_eq!(p[1].len(), 4 + (6 + 75 - CHUNK));
        assert_eq!(&p[1][p[1].len() - 3..], &[24, 0x10, 0x20]);
    }

    #[test]
    fn release_waits_for_the_fans() {
        let echo = Echo::default();
        let port = Port::alone(Box::new(echo.clone()));
        let device = port.device().clone();
        let mut io: Box<dyn Io> = Box::new(port);
        device.set_active(FANS, true);
        Lights.stop(io.as_mut());
        assert!(echo.0.lock().unwrap().0.is_empty());
        assert!(device.owed());

        device.set_active(FANS, false);
        Lights.stop(io.as_mut());
        let sent = &echo.0.lock().unwrap().0;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], crate::hid::padded(&cmd(&HARDWARE), LEN));
    }

    #[test]
    fn start_takes_software_mode_and_opens_the_color_endpoint() {
        let echo = Echo::default();
        let mut io: Box<dyn Io> = Box::new(Port::alone(Box::new(echo.clone())));
        Lights.start(io.as_mut()).unwrap();
        let sent: Vec<Vec<u8>> = echo
            .0
            .lock()
            .unwrap()
            .0
            .iter()
            .map(|p| p[..6].to_vec())
            .collect();
        assert_eq!(
            sent,
            [
                vec![0x00, 0x08, 0x01, 0x03, 0x00, 0x02],
                vec![0x00, 0x08, 0x05, 0x01, 0x01, 0x22],
                vec![0x00, 0x08, 0x0D, 0x00, 0x22, 0x00],
            ]
        );
    }
}
