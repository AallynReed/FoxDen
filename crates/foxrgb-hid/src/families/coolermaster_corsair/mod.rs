//! Ported from OpenRGB Controllers/CoolerMasterController, CorsairPeripheralController, CorsairPeripheralV2Controller and CorsairVoidController (GPL-2.0-or-later).

mod cm_argb;
mod cm_gd160;
mod cm_gen2;
mod cm_keyboard;
mod cm_mm;
mod cm_mm711;
mod cm_mm712;
mod cm_monitor;
mod cm_mp750;
mod cm_r6000;
mod cm_rgb;
mod cm_small_argb;
mod corsair_k55_xt;
mod corsair_peripheral;
mod corsair_v2;
mod corsair_void;
mod klm;

use std::time::{Duration, Instant};

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;

use crate::registry::Detector;

pub fn detectors() -> Vec<&'static Detector> {
    [
        cm_keyboard::DETECTORS,
        cm_argb::DETECTORS,
        cm_gen2::DETECTORS,
        cm_rgb::DETECTORS,
        cm_small_argb::DETECTORS,
        cm_mm::DETECTORS,
        cm_mm711::DETECTORS,
        cm_mm712::DETECTORS,
        cm_mp750::DETECTORS,
        cm_r6000::DETECTORS,
        cm_monitor::DETECTORS,
        cm_gd160::DETECTORS,
        corsair_peripheral::DETECTORS,
        corsair_k55_xt::DETECTORS,
        corsair_v2::DETECTORS,
        corsair_void::DETECTORS,
    ]
    .iter()
    .flat_map(|list| list.iter())
    .collect()
}

/// One `REGISTER_HID_DETECTOR*` line.
macro_rules! det {
    ($name:expr, $vid:expr, $pid:expr, $i:expr, $p:expr, $u:expr, $open:expr) => {
        crate::registry::Detector {
            name: $name,
            vid: $vid,
            pid: $pid,
            interface: $i,
            usage_page: $p,
            usage: $u,
            open: $open,
        }
    };
}
pub(crate) use det;

/// What a device does with one frame of colours, one per LED.
pub(super) trait Driver: Send {
    /// `first` is true until a frame has gone out, so the driver can take software control.
    fn frame(&mut self, colors: &[Color], first: bool) -> Result<(), String>;
}

/// A single-device backend that skips unchanged frames and resends after `refresh`.
pub(super) struct Lights<D> {
    name: &'static str,
    device: Device,
    refresh: Option<Duration>,
    driver: D,
    sent: Option<Vec<Color>>,
    ready: bool,
    at: Instant,
}

impl<D: Driver + 'static> Lights<D> {
    pub fn boxed(
        name: &'static str,
        device: Device,
        refresh: Option<Duration>,
        driver: D,
    ) -> Box<dyn Backend> {
        Box::new(Lights {
            name,
            device,
            refresh,
            driver,
            sent: None,
            ready: false,
            at: Instant::now(),
        })
    }
}

impl<D: Driver> Backend for Lights<D> {
    fn name(&self) -> &str {
        self.name
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let mut frame = colors.to_vec();
        frame.resize(self.device.leds.len(), Color::BLACK);
        let due = self.refresh.is_some_and(|r| self.at.elapsed() >= r);
        if self.sent.as_ref() == Some(&frame) && !due {
            return Ok(());
        }
        match self.driver.frame(&frame, !self.ready) {
            Ok(()) => {
                self.ready = true;
                self.sent = Some(frame);
                self.at = Instant::now();
                Ok(())
            }
            Err(e) => {
                self.ready = false;
                self.sent = None;
                Err(e)
            }
        }
    }
}

pub(super) fn write(hid: &HidDevice, report: &[u8]) -> Result<(), String> {
    hid.write(report).map(|_| ()).map_err(|e| e.to_string())
}

/// One input report, empty on timeout.
pub(super) fn read(hid: &HidDevice, ms: i32) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; 1025];
    let n = hid.read_timeout(&mut buf, ms).map_err(|e| e.to_string())?;
    Ok(buf[..n].to_vec())
}

/// Writes a report and waits for the answer, as OpenRGB's write-then-read pairs do.
pub(super) fn exchange(hid: &HidDevice, report: &[u8], ms: i32) -> Result<Vec<u8>, String> {
    write(hid, report)?;
    read(hid, ms)
}

/// A 65-byte report: report id 0, then `bytes` from index 1.
pub(super) fn report(bytes: &[u8]) -> [u8; 65] {
    let mut r = [0u8; 65];
    r[1..1 + bytes.len()].copy_from_slice(bytes);
    r
}

pub(super) fn rgb(c: Color) -> [u8; 3] {
    [c.0, c.1, c.2]
}

/// `n` LEDs evenly along a horizontal line.
pub(super) fn line(n: usize) -> Vec<[f32; 2]> {
    (0..n).map(|i| [(i as f32 + 0.5) / n as f32, 0.5]).collect()
}

/// Rows of LEDs, one row per entry of `counts`.
pub(super) fn rows(counts: &[usize]) -> Vec<[f32; 2]> {
    let n = counts.len().max(1) as f32;
    counts
        .iter()
        .enumerate()
        .flat_map(|(r, &count)| {
            (0..count).map(move |i| [(i as f32 + 0.5) / count as f32, (r as f32 + 0.5) / n])
        })
        .collect()
}

/// Positions of LEDs `0..count` from an OpenRGB matrix map; LEDs not in it go in a row below.
pub(super) fn matrix(map: &[&[u32]], count: usize) -> (Vec<[f32; 2]>, [u32; 2]) {
    let cols = map.iter().map(|r| r.len()).max().unwrap_or(1);
    let mut at: Vec<Option<(usize, usize)>> = vec![None; count];
    for (r, row) in map.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            if let Some(slot) = at.get_mut(v as usize) {
                *slot = Some((r, c));
            }
        }
    }
    let spare: Vec<usize> = (0..count).filter(|&i| at[i].is_none()).collect();
    let rows = map.len() + usize::from(!spare.is_empty());
    let leds = (0..count)
        .map(|i| match at[i] {
            Some((r, c)) => [
                (c as f32 + 0.5) / cols as f32,
                (r as f32 + 0.5) / rows as f32,
            ],
            None => {
                let k = spare.iter().position(|&s| s == i).unwrap_or(0);
                [
                    (k as f32 + 0.5) / spare.len() as f32,
                    (rows as f32 - 0.5) / rows as f32,
                ]
            }
        })
        .collect();
    (leds, [cols as u32, rows as u32])
}

/// A default spot on the canvas for each kind of device.
pub(super) fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        DeviceKind::Strip => [0.06, 0.12, 0.52, 0.05],
        DeviceKind::Gpu => [0.06, 0.30, 0.40, 0.05],
        _ => [0.62, 0.40, 0.30, 0.10],
    }
}

pub(super) fn device(
    id: String,
    name: &str,
    kind: DeviceKind,
    leds: Vec<[f32; 2]>,
    grid: [u32; 2],
) -> Device {
    Device {
        id,
        name: name.into(),
        kind,
        rect: rect(kind),
        grid,
        leds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_detector_is_distinct() {
        let all = detectors();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(
                    !(a.vid == b.vid
                        && a.pid == b.pid
                        && a.interface == b.interface
                        && a.usage_page == b.usage_page
                        && a.usage == b.usage),
                    "{} and {} overlap",
                    a.name,
                    b.name
                );
            }
        }
    }

    #[test]
    fn corsair_ids_have_one_driver() {
        let lists = [
            corsair_peripheral::DETECTORS,
            corsair_k55_xt::DETECTORS,
            corsair_v2::DETECTORS,
            corsair_void::DETECTORS,
        ];
        for (i, a) in lists.iter().enumerate() {
            for b in &lists[i + 1..] {
                for d in a.iter() {
                    assert!(!b.iter().any(|e| e.pid == d.pid), "{}", d.name);
                }
            }
        }
        assert_eq!(corsair_peripheral::DETECTORS.len(), 44);
        assert_eq!(corsair_v2::DETECTORS.len(), 40);
    }

    #[test]
    fn matrix_places_spare_leds_below() {
        let (leds, grid) = matrix(&[&[0, u32::MAX], &[u32::MAX, 1]], 3);
        assert_eq!(grid, [2, 3]);
        assert_eq!(leds[0], [0.25, 0.5 / 3.0]);
        assert_eq!(leds[1], [0.75, 1.5 / 3.0]);
        assert_eq!(leds[2], [0.5, 2.5 / 3.0]);
    }

    #[test]
    fn report_framing() {
        let r = report(&[0x41, 0x80]);
        assert_eq!(&r[..3], &[0x00, 0x41, 0x80]);
        assert!(r[3..].iter().all(|&b| b == 0));
    }
}
