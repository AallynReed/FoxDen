// SPDX-License-Identifier: GPL-2.0-or-later

use std::collections::BTreeMap;

use super::Link;

const RECEIVER_INDEX: u8 = 0xFF;
const SET_REGISTER: u8 = 0x80;
const GET_REGISTER: u8 = 0x81;
const GET_LONG_REGISTER: u8 = 0x83;
/// `LOGITECH_PROTOCOL_TIMEOUT`.
const TIMEOUT: i32 = 300;

fn short(sub_id: u8, address: u8, data: [u8; 3]) -> [u8; 7] {
    [
        0x10,
        RECEIVER_INDEX,
        sub_id,
        address,
        data[0],
        data[1],
        data[2],
    ]
}

fn read(link: &mut impl Link) -> [u8; 64] {
    let mut buf = [0u8; 64];
    let _ = link.read(&mut buf, TIMEOUT);
    buf
}

/// `getWirelessDevice`: paired slots by wireless PID. The request's register address is left
/// uninitialized in the C++ (`shortFAPrequest::init` assigns it to itself); 0x00, the
/// notification-flags register the code reads bit 0 of, is what it is meant to be.
pub fn paired(link: &mut impl Link) -> BTreeMap<u16, u8> {
    let mut slots = BTreeMap::new();
    let _ = link.write(&short(GET_REGISTER, 0x00, [0; 3]));
    let response = read(link);
    if response[5] & 1 == 0 {
        let _ = link.write(&short(SET_REGISTER, 0x00, [0, 1, 0]));
        read(link);
    }
    let _ = link.write(&short(GET_REGISTER, 0x02, [0; 3]));
    let count = (read(link)[5] as usize).min(6);
    if count > 0 {
        let _ = link.write(&short(SET_REGISTER, 0x02, [0x02, 0, 0]));
        for _ in 0..count + 1 {
            let devices = read(link);
            let pid = u16::from_le_bytes([devices[5], devices[6]]);
            if devices[1] != RECEIVER_INDEX {
                slots.entry(pid).or_insert(devices[1]);
            }
        }
    }
    slots
}

/// Reads long register 0xB5 sub `sub`, skipping unrelated frames, as the name and serial reads do.
fn pairing_register(link: &mut impl Link, sub: u8) -> Option<[u8; 64]> {
    let _ = link.write(&short(GET_LONG_REGISTER, 0xB5, [sub, 0, 0]));
    for _ in 0..8 {
        let mut buf = [0u8; 64];
        if link.read(&mut buf, TIMEOUT).unwrap_or(0) == 0 {
            return None;
        }
        if buf[2] == GET_LONG_REGISTER && buf[3] == 0xB5 && buf[4] == sub {
            return Some(buf);
        }
    }
    None
}

/// Pairing info reply: wireless PID big-endian at data bytes 3-4, device type at data byte 7.
pub fn parse_kind(r: &[u8]) -> (u16, u8) {
    (u16::from_be_bytes([r[7], r[8]]), r[11] & 0x0F)
}

/// `getWirelessDeviceSerial`: the slot's unit id as the receiver stores it.
pub fn serial(link: &mut impl Link, index: u8) -> String {
    if index < 1 {
        return String::new();
    }
    let Some(r) = pairing_register(link, 0x30 + index - 1) else {
        return String::new();
    };
    let serial = format!("{:02X}{:02X}{:02X}{:02X}", r[5], r[6], r[7], r[8]);
    if serial == "00000000" {
        String::new()
    } else {
        serial
    }
}

/// `getWirelessDeviceName`: the codename the receiver stores for the slot.
pub fn name(link: &mut impl Link, index: u8) -> String {
    if index < 1 {
        return String::new();
    }
    let Some(r) = pairing_register(link, 0x40 + index - 1) else {
        return String::new();
    };
    let len = r[5] as usize;
    if len == 0 || len > 14 {
        return String::new();
    }
    let mut name = r[6..6 + len].to_vec();
    while matches!(name.last(), Some(0 | b' ')) {
        name.pop();
    }
    if name.iter().any(|&c| !(0x20..=0x7E).contains(&c)) {
        return String::new();
    }
    String::from_utf8(name).unwrap_or_default()
}

/// `LogitechDevicePathKey`: a Windows collection path with its `&colNN` part and the
/// instance suffix it shifts removed, so sibling collections of one interface compare equal.
pub fn path_key(path: &str) -> String {
    let mut key = path.to_string();
    let lower = key.to_ascii_lowercase();
    let Some(pos) = lower.find("&col") else {
        return key;
    };
    let end = key[pos + 4..]
        .find(|c: char| !c.is_ascii_hexdigit())
        .map_or(key.len(), |e| pos + 4 + e);
    key.replace_range(pos..end, "");
    if let Some(guid) = key.rfind('#')
        && let Some(last) = key[..guid].rfind('&')
    {
        key.replace_range(last..guid, "");
    }
    key
}

/// Wireless PIDs whose slots stay with a legacy detector, enabled by default.
pub fn legacy(pid: u16) -> bool {
    matches!(
        pid,
        0x4053 | 0x405D | 0x405F | 0x4067 | 0x4070 | 0x4079 | 0x407F | 0x4086 | 0x4087
    )
}

#[cfg(test)]
mod tests {
    use super::super::fake::Fake;
    use super::*;

    fn r(b: &[u8]) -> Vec<u8> {
        b.to_vec()
    }

    // getWirelessDevice: flags already on, two paired devices, fake reconnect lists them.
    #[test]
    fn pairing_table_read() {
        let mut f = Fake::new(|w| match (w[2], w[3]) {
            (0x81, 0x00) => vec![r(&[0x10, 0xFF, 0x81, 0x00, 0x00, 0x01, 0x00])],
            (0x81, 0x02) => vec![r(&[0x10, 0xFF, 0x81, 0x02, 0x00, 0x02, 0x00])],
            (0x80, 0x02) => vec![
                r(&[0x10, 0xFF, 0x80, 0x02, 0x00, 0x00, 0x00]),
                r(&[0x10, 0x02, 0x41, 0x04, 0x40, 0x99, 0x40]),
                r(&[0x10, 0x01, 0x41, 0x04, 0x00, 0x8D, 0x40]),
            ],
            _ => Vec::new(),
        });
        let slots = paired(&mut f);
        assert_eq!(
            slots.into_iter().collect::<Vec<_>>(),
            [(0x408D, 1), (0x4099, 2)]
        );
        assert_eq!(
            f.writes(),
            [
                r(&[0x10, 0xFF, 0x81, 0x00, 0, 0, 0]),
                r(&[0x10, 0xFF, 0x81, 0x02, 0, 0, 0]),
                r(&[0x10, 0xFF, 0x80, 0x02, 0x02, 0, 0]),
            ]
        );
    }

    // getWirelessDevice: notifications off are switched on first; no devices means no reconnect.
    #[test]
    fn pairing_enables_notifications() {
        let mut f = Fake::new(|_| vec![r(&[0x10, 0xFF, 0x81, 0x00, 0x00, 0x00, 0x00])]);
        assert!(paired(&mut f).is_empty());
        assert_eq!(
            f.writes(),
            [
                r(&[0x10, 0xFF, 0x81, 0x00, 0, 0, 0]),
                r(&[0x10, 0xFF, 0x80, 0x00, 0, 1, 0]),
                r(&[0x10, 0xFF, 0x81, 0x02, 0, 0, 0]),
            ]
        );
    }

    // getWirelessDeviceSerial / getWirelessDeviceName for slot 2.
    #[test]
    fn serial_and_name_registers() {
        let mut f = Fake::new(|w| {
            let mut reply = vec![0x11, 0xFF, 0x83, 0xB5, w[4]];
            if w[4] == 0x31 {
                reply.extend([0x12, 0xAB, 0x00, 0x7F]);
            } else {
                reply.extend([5]);
                reply.extend(b"G915\0");
            }
            reply.resize(20, 0);
            vec![r(&[0x10, 0x02, 0x41, 0, 0, 0, 0]), reply]
        });
        assert_eq!(serial(&mut f, 2), "12AB007F");
        assert_eq!(name(&mut f, 2), "G915");
        assert_eq!(
            f.writes(),
            [
                r(&[0x10, 0xFF, 0x83, 0xB5, 0x31, 0, 0]),
                r(&[0x10, 0xFF, 0x83, 0xB5, 0x41, 0, 0]),
            ]
        );
        let mut silent = Fake::new(|_| Vec::new());
        assert_eq!(serial(&mut silent, 1), "");
    }

    // Pairing info of a G502 X LIGHTSPEED behind a C547 receiver, as a user's receiver answered.
    #[test]
    fn pairing_info_gives_pid_and_type() {
        let mut reply = vec![0x11, 0xFF, 0x83, 0xB5];
        reply.extend([0x20, 0x07, 0x08, 0x40, 0x9F, 0x11, 0x00, 0x02, 0xB1]);
        reply.resize(20, 0);
        assert_eq!(parse_kind(&reply), (0x409F, 2));
    }

    // LogitechDevicePathKey on sibling Windows collections.
    #[test]
    fn path_keys_match_siblings() {
        let a = r"\\?\HID#VID_046D&PID_C547&MI_02&Col01#8&2a0b3e5&0&0000#{4d1e55b2}";
        let b = r"\\?\HID#VID_046D&PID_C547&MI_02&Col02#8&2a0b3e5&0&0001#{4d1e55b2}";
        assert_eq!(path_key(a), path_key(b));
        assert_eq!(
            path_key(a),
            r"\\?\HID#VID_046D&PID_C547&MI_02#8&2a0b3e5&0#{4d1e55b2}"
        );
        assert_eq!(path_key("/dev/hidraw3"), "/dev/hidraw3");
    }
}
