// SPDX-License-Identifier: GPL-2.0-or-later

use std::net::UdpSocket;
use std::time::Duration;

use foxden_core::{Color, DeviceKind};

use crate::net::udp;
use crate::{DdpDevice, Driver, Light};

pub const PORT: u16 = 4048;
const MAX_DATA: usize = 1440;
const FLAG_VER_1: u8 = 0x40;
const FLAG_PUSH: u8 = 0x01;

/// `SendDDPPacket`: 10-byte header, offset and length big-endian.
fn packet(sequence: u8, offset: u32, data: &[u8]) -> Vec<u8> {
    let mut p = vec![FLAG_VER_1 | FLAG_PUSH, sequence & 0x0F, 1, 1];
    p.extend_from_slice(&offset.to_be_bytes());
    p.extend_from_slice(&(data.len() as u16).to_be_bytes());
    p.extend_from_slice(data);
    p
}

/// `UpdateLEDs`: each device's pixels in chunks of 1440 bytes, offsets counted per device.
fn packets(devices: &[DdpDevice], colors: &[Color], sequence: u8) -> Vec<(usize, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (i, d) in devices.iter().enumerate() {
        if at >= colors.len() {
            break;
        }
        let n = d.num_leds as usize;
        let mut data = vec![0u8; n * 3];
        for (led, c) in colors[at..].iter().take(n).enumerate() {
            data[led * 3..led * 3 + 3].copy_from_slice(&[c.0, c.1, c.2]);
        }
        for (k, chunk) in data.chunks(MAX_DATA).enumerate() {
            out.push((i, packet(sequence, (k * MAX_DATA) as u32, chunk)));
        }
        at += n;
    }
    out
}

/// Devices sharing an address and port make one controller.
fn group(devices: &[DdpDevice]) -> Vec<Vec<DdpDevice>> {
    let mut lists: Vec<Vec<DdpDevice>> = Vec::new();
    for (n, dev) in devices.iter().enumerate() {
        if dev.ip.is_empty() || dev.num_leds == 0 {
            continue;
        }
        let mut dev = dev.clone();
        if dev.name.is_empty() {
            dev.name = format!("DDP Device {}", n + 1);
        }
        match lists
            .iter_mut()
            .find(|l| l.iter().any(|o| o.ip == dev.ip && o.port == dev.port))
        {
            Some(list) => list.push(dev),
            None => lists.push(vec![dev]),
        }
    }
    lists
}

struct Ddp {
    socket: UdpSocket,
    devices: Vec<DdpDevice>,
    sequence: u8,
}

impl Driver for Ddp {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        for (_, p) in packets(&self.devices, colors, self.sequence) {
            self.socket.send(&p).map_err(|e| e.to_string())?;
        }
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }
}

fn controller(devices: Vec<DdpDevice>) -> Option<Light> {
    let first = devices.first()?;
    let socket = udp(&first.ip, first.port).ok()?;
    let name = if devices.len() == 1 {
        first.name.clone()
    } else {
        "DDP Device Group".into()
    };
    let keepalive =
        (first.keepalive_time > 0).then(|| Duration::from_millis(first.keepalive_time as u64));
    let leds: u32 = devices.iter().map(|d| d.num_leds).sum();
    Some(
        Light::new(
            format!("net:ddp:{}:{}", first.ip, first.port),
            name,
            DeviceKind::Strip,
            leds as usize,
            Box::new(Ddp {
                socket,
                devices,
                sequence: 0,
            }),
        )
        .keep(keepalive),
    )
}

pub fn open(devices: &[DdpDevice]) -> Vec<Light> {
    group(devices).into_iter().filter_map(controller).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(ip: &str, port: u16, leds: u32) -> DdpDevice {
        DdpDevice {
            ip: ip.into(),
            port,
            num_leds: leds,
            ..Default::default()
        }
    }

    #[test]
    fn header_bytes() {
        assert_eq!(
            packet(0x13, 0x0102_0304, &[9, 8, 7]),
            vec![0x41, 0x03, 0x01, 0x01, 1, 2, 3, 4, 0, 3, 9, 8, 7]
        );
    }

    #[test]
    fn large_strips_are_chunked_at_1440_bytes() {
        let colors = vec![Color(1, 2, 3); 500];
        let p = packets(&[dev("10.0.0.5", PORT, 500)], &colors, 2);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].1.len(), 10 + 1440);
        assert_eq!(&p[1].1[..10], &[0x41, 2, 1, 1, 0, 0, 0x05, 0xA0, 0, 60]);
        assert_eq!(&p[1].1[10..13], &[1, 2, 3]);
    }

    #[test]
    fn every_device_in_a_group_starts_at_offset_zero() {
        let colors = [Color(1, 1, 1), Color(2, 2, 2), Color(3, 3, 3)];
        let p = packets(
            &[dev("10.0.0.5", PORT, 2), dev("10.0.0.5", PORT, 1)],
            &colors,
            0,
        );
        assert_eq!(p[0].1[4..8], [0, 0, 0, 0]);
        assert_eq!(&p[0].1[10..], &[1, 1, 1, 2, 2, 2]);
        assert_eq!(p[1].1[4..8], [0, 0, 0, 0]);
        assert_eq!(&p[1].1[10..], &[3, 3, 3]);
    }

    #[test]
    fn grouping_skips_incomplete_devices() {
        let lists = group(&[
            dev("10.0.0.5", PORT, 10),
            dev("10.0.0.5", PORT, 0),
            dev("", PORT, 10),
            dev("10.0.0.5", 4049, 10),
            dev("10.0.0.5", PORT, 5),
        ]);
        assert_eq!(lists.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(lists[0][1].name, "DDP Device 5");
    }
}
