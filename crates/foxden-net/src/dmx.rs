// SPDX-License-Identifier: GPL-2.0-or-later

use std::io::Write;
use std::thread;
use std::time::Duration;

use foxden_core::{Color, DeviceKind};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};

use crate::{DmxDevice, Driver, Light};

const BAUD: u32 = 250_000;
const BRIGHTNESS: u8 = 255;
const BREAK: Duration = Duration::from_millis(1);

/// `DeviceUpdateLEDs`: start code 0 and 512 channels, one colour per device.
fn frame(devices: &[DmxDevice], colors: &[Color]) -> [u8; 513] {
    let mut d = [0u8; 513];
    let mut put = |ch: u32, v: u8| {
        if ch > 0 && (ch as usize) < d.len() {
            d[ch as usize] = v;
        }
    };
    for (i, dev) in devices.iter().enumerate() {
        let c = colors.get(i).copied().unwrap_or_default();
        put(dev.brightness_channel, BRIGHTNESS);
        put(dev.red_channel, c.0);
        put(dev.green_channel, c.1);
        put(dev.blue_channel, c.2);
    }
    d
}

struct Dmx {
    port: Box<dyn SerialPort>,
    devices: Vec<DmxDevice>,
}

impl Driver for Dmx {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        let d = frame(&self.devices, colors);
        self.port.set_break().map_err(|e| e.to_string())?;
        thread::sleep(BREAK);
        self.port.clear_break().map_err(|e| e.to_string())?;
        self.port.write_all(&d).map_err(|e| e.to_string())
    }
}

/// `DetectDMXControllers` puts every device in one group, on the first device's port.
pub fn open(devices: &[DmxDevice]) -> Vec<Light> {
    let Some(first) = devices.first() else {
        return Vec::new();
    };
    let Ok(mut port) = serialport::new(&first.port, BAUD)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::Two)
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(500))
        .open()
    else {
        return Vec::new();
    };
    let _ = port.write_request_to_send(false);
    let keepalive = devices
        .iter()
        .map(|d| d.keepalive_time)
        .filter(|t| *t > 0)
        .min()
        .map(|t| Duration::from_millis(t as u64));
    let name = if devices.len() == 1 {
        first.name.clone()
    } else {
        "DMX Device Group".into()
    };
    let kind = if devices.len() == 1 {
        DeviceKind::Other
    } else {
        DeviceKind::Strip
    };
    let light = Light::new(
        format!("net:dmx:{}", first.port),
        if name.is_empty() {
            "DMX device".into()
        } else {
            name
        },
        kind,
        devices.len(),
        Box::new(Dmx {
            port,
            devices: devices.to_vec(),
        }),
    )
    .keep(keepalive);
    vec![light]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_puts_each_device_on_its_channels() {
        let devices = vec![
            DmxDevice {
                red_channel: 1,
                green_channel: 2,
                blue_channel: 3,
                brightness_channel: 4,
                ..Default::default()
            },
            DmxDevice {
                red_channel: 10,
                green_channel: 11,
                blue_channel: 512,
                ..Default::default()
            },
        ];
        let d = frame(&devices, &[Color(1, 2, 3), Color(4, 5, 6)]);
        assert_eq!(d.len(), 513);
        assert_eq!(&d[..5], &[0, 1, 2, 3, 255]);
        assert_eq!(&d[10..12], &[4, 5]);
        assert_eq!(d[512], 6);
        assert_eq!(d.iter().filter(|b| **b != 0).count(), 7);
    }
}
