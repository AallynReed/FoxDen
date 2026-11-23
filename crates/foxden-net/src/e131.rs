// SPDX-License-Identifier: GPL-2.0-or-later
//! E1.31 (sACN) output.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use foxden_core::{Color, DeviceKind};

use crate::{Driver, E131Device, Light, local};

const PORT: u16 = 5568;
const PRIORITY: u8 = 0x64;
const ACN_PID: [u8; 12] = [
    0x41, 0x53, 0x43, 0x2d, 0x45, 0x31, 0x2e, 0x31, 0x37, 0x00, 0x00, 0x00,
];
const RAW: usize = 638;
/// Root (38) + framing (77) + DMP header (10) layers before the property values.
const VALUES: usize = 125;

/// `e131_pkt_init`: a packed packet with a zero CID and source name.
fn packet(universe: u16, slots: u16) -> [u8; RAW] {
    let mut p = [0u8; RAW];
    let prop_val_cnt = slots + 1;
    let dmp_length = prop_val_cnt + 10;
    let frame_length = 77 + dmp_length;
    let root_length = 2 + 4 + 16 + frame_length;
    p[0..2].copy_from_slice(&0x0010u16.to_be_bytes());
    p[4..16].copy_from_slice(&ACN_PID);
    p[16..18].copy_from_slice(&(0x7000 | root_length).to_be_bytes());
    p[18..22].copy_from_slice(&4u32.to_be_bytes());
    p[38..40].copy_from_slice(&(0x7000 | frame_length).to_be_bytes());
    p[40..44].copy_from_slice(&2u32.to_be_bytes());
    p[108] = PRIORITY;
    p[113..115].copy_from_slice(&universe.to_be_bytes());
    p[115..117].copy_from_slice(&(0x7000 | dmp_length).to_be_bytes());
    p[117] = 0x02;
    p[118] = 0xa1;
    p[121..123].copy_from_slice(&1u16.to_be_bytes());
    p[123..125].copy_from_slice(&prop_val_cnt.to_be_bytes());
    p
}

const SEQUENCE: usize = 111;

/// `e131_send`'s length: everything up to the last property value in use.
fn send_length(p: &[u8; RAW]) -> usize {
    RAW - 513 + u16::from_be_bytes([p[123], p[124]]) as usize
}

/// `ceil(((num_leds * 3) + start_channel) / universe_size)` in float, as the controller works it out.
fn universes(d: &E131Device) -> u32 {
    (((d.num_leds * 3 + d.start_channel) as f32) / d.universe_size as f32).ceil() as u32
}

/// Devices on the same address whose universes overlap share one controller.
fn group(devices: &[E131Device]) -> Vec<Vec<E131Device>> {
    let end = |d: &E131Device| {
        d.start_universe + (d.start_channel + 3 * d.num_leds).saturating_sub(2) / 512
    };
    let mut lists: Vec<Vec<E131Device>> = Vec::new();
    for dev in devices {
        let at = lists.iter().position(|list| {
            list.iter().any(|l| {
                dev.ip == l.ip && !(end(dev) < l.start_universe || end(l) < dev.start_universe)
            })
        });
        match at {
            Some(i) => lists[i].push(dev.clone()),
            None => lists.push(vec![dev.clone()]),
        }
    }
    lists
}

struct Universe {
    number: u32,
    packet: [u8; RAW],
    dest: SocketAddr,
}

struct E131 {
    socket: UdpSocket,
    devices: Vec<E131Device>,
    universes: Vec<Universe>,
}

impl E131 {
    fn new(socket: UdpSocket, devices: Vec<E131Device>, unicast: Option<Ipv4Addr>) -> Self {
        let mut list: Vec<Universe> = Vec::new();
        for d in &devices {
            for i in 0..universes(d) {
                let number = d.start_universe + i;
                if list.iter().any(|u| u.number == number) {
                    continue;
                }
                let ip = unicast.unwrap_or(Ipv4Addr::from(0xefff_0000 | (number & 0xffff)));
                list.push(Universe {
                    number,
                    packet: packet(number as u16, d.universe_size as u16),
                    dest: SocketAddr::from((ip, PORT)),
                });
            }
        }
        E131 {
            socket,
            devices,
            universes: list,
        }
    }

    /// Fills each device's channels across its universes.
    fn fill(&mut self, colors: &[Color]) {
        let mut color_idx = 0;
        for d in &self.devices {
            let size = d.universe_size as f32;
            let mut channel = d.start_channel;
            let (mut led, mut rgb) = (0, 0);
            let mut done = false;
            for i in 0..universes(d) {
                let number = d.start_universe + i;
                for u in self.universes.iter_mut().filter(|u| u.number == number) {
                    while !done && channel as f32 <= size && (channel as usize) < 513 {
                        let c = colors.get(color_idx).copied().unwrap_or_default();
                        let value = [c.0, c.1, c.2][rgb];
                        u.packet[VALUES + channel as usize] = value;
                        rgb = (rgb + 1) % 3;
                        if rgb == 0 {
                            led += 1;
                            color_idx += 1;
                        }
                        if led >= d.num_leds {
                            done = true;
                        }
                        channel += 1;
                    }
                }
                channel = 1;
            }
        }
    }
}

impl Driver for E131 {
    fn send(&mut self, colors: &[Color]) -> Result<(), String> {
        self.fill(colors);
        let mut result = Ok(());
        for u in &mut self.universes {
            let n = send_length(&u.packet);
            if let Err(e) = self.socket.send_to(&u.packet[..n], u.dest) {
                result = Err(e.to_string());
            }
            u.packet[SEQUENCE] = u.packet[SEQUENCE].wrapping_add(1);
        }
        result
    }
}

fn controller(devices: Vec<E131Device>) -> Option<Light> {
    let first = devices.first()?;
    let unicast = if first.ip.is_empty() {
        None
    } else {
        match local(&first.ip).ok()? {
            std::net::IpAddr::V4(v4) => Some(v4),
            _ => return None,
        }
    };
    if devices
        .iter()
        .any(|d| !(1..=512).contains(&d.universe_size) || d.start_universe == 0)
    {
        return None;
    }
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    let _ = socket.set_multicast_ttl_v4(1);
    // The address stays out of group names so error reports never carry it.
    let name = if devices.len() == 1 {
        first.name.clone()
    } else {
        "E1.31 Device Group".into()
    };
    let key = format!("{}-{}", first.ip, first.start_universe);
    let leds: u32 = devices.iter().map(|d| d.num_leds).sum();
    let keepalive = devices
        .iter()
        .map(|d| d.keepalive_time)
        .filter(|t| *t > 0)
        .min()
        .map(|t| Duration::from_millis(t as u64));
    Some(
        Light::new(
            format!("net:e131:{key}"),
            if name.is_empty() {
                "E1.31 device".into()
            } else {
                name
            },
            DeviceKind::Strip,
            leds as usize,
            Box::new(E131::new(socket, devices, unicast)),
        )
        .keep(keepalive),
    )
}

pub fn open(devices: &[E131Device]) -> Vec<Light> {
    group(devices).into_iter().filter_map(controller).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(ip: &str, leds: u32, universe: u32, channel: u32) -> E131Device {
        E131Device {
            name: "d".into(),
            ip: ip.into(),
            num_leds: leds,
            start_universe: universe,
            start_channel: channel,
            keepalive_time: 0,
            universe_size: 512,
        }
    }

    #[test]
    fn full_universe_packet_layout() {
        let p = packet(1, 512);
        assert_eq!(&p[0..4], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0");
        // root 0x7000 | 622, vector 4
        assert_eq!(&p[16..22], &[0x72, 0x6e, 0, 0, 0, 4]);
        assert!(p[22..38].iter().all(|b| *b == 0));
        // frame 0x7000 | 600, vector 2, empty source name
        assert_eq!(&p[38..44], &[0x72, 0x58, 0, 0, 0, 2]);
        assert!(p[44..108].iter().all(|b| *b == 0));
        // priority 100, reserved, sequence 0, options 0, universe 1
        assert_eq!(&p[108..115], &[0x64, 0, 0, 0, 0, 0, 1]);
        // dmp 0x7000 | 523, vector 2, type 0xa1, first 0, increment 1, count 513
        assert_eq!(
            &p[115..125],
            &[0x72, 0x0b, 0x02, 0xa1, 0, 0, 0, 1, 0x02, 0x01]
        );
        assert_eq!(send_length(&p), 638);
        assert_eq!(send_length(&packet(2, 170)), 125 + 171);
    }

    #[test]
    fn colours_fill_channels_from_the_start_channel_across_universes() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        // 171 LEDs from channel 1: 512 channels hold 170 LEDs and 2/3 of the next
        let d = dev("", 171, 1, 1);
        assert_eq!(universes(&d), 2);
        let mut e = E131::new(socket, vec![d], None);
        let colors: Vec<Color> = (0..171).map(|i| Color(i as u8, 1, 2)).collect();
        e.fill(&colors);
        let u1 = &e.universes[0].packet;
        assert_eq!(u1[VALUES], 0, "start code");
        assert_eq!(&u1[VALUES + 1..VALUES + 4], &[0, 1, 2]);
        assert_eq!(&u1[VALUES + 508..VALUES + 513], &[169, 1, 2, 170, 1]);
        let u2 = &e.universes[1].packet;
        assert_eq!(u2[VALUES + 1], 2);
        assert_eq!(u2[VALUES + 2], 0);
        assert_eq!(e.universes[1].dest, "239.255.0.2:5568".parse().unwrap());
    }

    #[test]
    fn sequence_counts_up_after_each_send() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut e = E131::new(
            socket,
            vec![dev("127.0.0.1", 1, 7, 1)],
            Some(Ipv4Addr::LOCALHOST),
        );
        e.universes[0].dest = sink.local_addr().unwrap();
        e.send(&[Color(9, 8, 7)]).unwrap();
        e.send(&[Color(9, 8, 7)]).unwrap();
        let mut buf = [0u8; 700];
        let n = sink.recv(&mut buf).unwrap();
        assert_eq!(n, 638);
        assert_eq!(buf[SEQUENCE], 0);
        assert_eq!(&buf[VALUES + 1..VALUES + 4], &[9, 8, 7]);
        sink.recv(&mut buf).unwrap();
        assert_eq!(buf[SEQUENCE], 1);
    }

    #[test]
    fn overlapping_universes_on_one_address_share_a_controller() {
        let lists = group(&[
            dev("10.0.0.2", 200, 1, 1),
            dev("10.0.0.2", 10, 2, 1),
            dev("10.0.0.2", 10, 5, 1),
            dev("10.0.0.3", 10, 1, 1),
        ]);
        assert_eq!(
            lists.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![2, 1, 1]
        );
    }
}
