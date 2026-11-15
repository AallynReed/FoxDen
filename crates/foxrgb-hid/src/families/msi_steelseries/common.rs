//! Ported from OpenRGB Controllers/MSIMotherboardController and Controllers/SteelSeriesController (GPL-2.0-or-later).

use foxrgb_core::{Backend, Color, Device, DeviceKind};
use hidapi::HidDevice;
use std::time::Duration;

/// One transfer, in the order OpenRGB makes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Feature(Vec<u8>),
    Write(Vec<u8>),
    /// Reads and drops one input report, waiting up to this many milliseconds.
    Read(usize, i32),
    Sleep(u64),
}

/// What a family sends for one frame; the first call also takes software control.
pub trait Proto: Send {
    fn frame(&mut self, colors: &[Color]) -> Vec<Op>;
    /// Called after a failed transfer so the next frame starts over.
    fn reset(&mut self) {}
}

pub struct Driver<P: Proto> {
    hid: HidDevice,
    name: &'static str,
    device: Device,
    sent: Option<Vec<Color>>,
    proto: P,
}

impl<P: Proto> Driver<P> {
    pub fn new(hid: HidDevice, name: &'static str, device: Device, proto: P) -> Self {
        Driver {
            hid,
            name,
            device,
            sent: None,
            proto,
        }
    }

    pub fn boxed(hid: HidDevice, name: &'static str, device: Device, proto: P) -> Box<dyn Backend>
    where
        P: 'static,
    {
        Box::new(Self::new(hid, name, device, proto))
    }
}

pub fn run(hid: &HidDevice, ops: &[Op]) -> Result<(), String> {
    for op in ops {
        match op {
            Op::Feature(b) => hid.send_feature_report(b).map_err(|e| e.to_string())?,
            Op::Write(b) => {
                hid.write(b).map_err(|e| e.to_string())?;
            }
            Op::Read(len, ms) => {
                let mut buf = vec![0u8; *len];
                let _ = hid.read_timeout(&mut buf, *ms);
            }
            Op::Sleep(ms) => std::thread::sleep(Duration::from_millis(*ms)),
        }
    }
    Ok(())
}

impl<P: Proto> Backend for Driver<P> {
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
        if self.sent.as_deref() == Some(&frame[..]) {
            return Ok(());
        }
        let ops = self.proto.frame(&frame);
        match run(&self.hid, &ops) {
            Ok(()) => {
                self.sent = Some(frame);
                Ok(())
            }
            Err(e) => {
                self.sent = None;
                self.proto.reset();
                Err(e)
            }
        }
    }
}

/// Feature report read with the report id in byte 0; returns the length hidapi gives.
pub fn get_feature(hid: &HidDevice, id: u8, len: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; len];
    buf[0] = id;
    let n = hid
        .get_feature_report(&mut buf)
        .map_err(|e| e.to_string())?;
    buf.truncate(n);
    Ok(buf)
}

pub fn rect(kind: DeviceKind) -> [f32; 4] {
    match kind {
        DeviceKind::Motherboard => [0.06, 0.10, 0.52, 0.15],
        DeviceKind::Keyboard => [0.06, 0.58, 0.54, 0.32],
        DeviceKind::Mouse => [0.70, 0.62, 0.06, 0.22],
        DeviceKind::Strip => [0.06, 0.45, 0.52, 0.05],
        _ => [0.62, 0.30, 0.30, 0.20],
    }
}

/// Each zone on its own row, LEDs spread across it.
pub fn rows(zones: &[usize]) -> (Vec<[f32; 2]>, [u32; 2]) {
    let n = zones.len().max(1);
    let mut leds = Vec::new();
    for (r, &count) in zones.iter().enumerate() {
        let y = (r as f32 + 0.5) / n as f32;
        leds.extend((0..count).map(|i| [(i as f32 + 0.5) / count as f32, y]));
    }
    let cols = zones.iter().copied().max().unwrap_or(1).max(1);
    (leds, [cols as u32, n as u32])
}

/// LEDs placed by an OpenRGB matrix map (row-major, `None` for gaps); LEDs missing from it go on an extra row.
pub fn matrix(
    count: usize,
    height: usize,
    width: usize,
    map: &[Option<usize>],
) -> (Vec<[f32; 2]>, [u32; 2]) {
    let mut cells: Vec<Option<(usize, usize)>> = vec![None; count];
    for (at, led) in map.iter().enumerate() {
        if let Some(l) = led
            && *l < count
            && cells[*l].is_none()
        {
            cells[*l] = Some((at / width, at % width));
        }
    }
    let missing: Vec<usize> = (0..count).filter(|&i| cells[i].is_none()).collect();
    let h = height + usize::from(!missing.is_empty());
    for (k, &i) in missing.iter().enumerate() {
        cells[i] = Some((height, k * width / missing.len().max(1)));
    }
    let leds = cells
        .iter()
        .map(|c| {
            let (r, col) = c.unwrap_or((0, 0));
            [
                (col as f32 + 0.5) / width as f32,
                (r as f32 + 0.5) / h as f32,
            ]
        })
        .collect();
    (leds, [width as u32, h as u32])
}

pub fn device(
    id: String,
    name: &str,
    kind: DeviceKind,
    layout: (Vec<[f32; 2]>, [u32; 2]),
) -> Device {
    Device {
        id,
        name: name.into(),
        kind,
        rect: rect(kind),
        grid: layout.1,
        leds: layout.0,
    }
}

pub fn line(count: usize) -> (Vec<[f32; 2]>, [u32; 2]) {
    rows(&[count])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_places_every_led() {
        let (leds, grid) = matrix(3, 1, 2, &[Some(0), Some(1)]);
        assert_eq!(grid, [2, 2]);
        assert_eq!(leds[0], [0.25, 0.25]);
        assert_eq!(leds[2][1], 0.75);
    }

    #[test]
    fn rows_one_per_zone() {
        let (leds, grid) = rows(&[2, 1]);
        assert_eq!(grid, [2, 2]);
        assert_eq!(leds, vec![[0.25, 0.25], [0.75, 0.25], [0.5, 0.75]]);
    }
}
