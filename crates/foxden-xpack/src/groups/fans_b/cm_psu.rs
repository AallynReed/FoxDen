//! Cooler Master XG Plus power supplies: fan speed, temperature and power. Read only.

use super::util::{Cached, ask_strict, device_key, fan_id, u16le};
use crate::fan::{Channel, FanHub, backend_on};
use crate::hid::{Io, open_io};
use foxden_fan::{FanBackend, Kind};
use foxden_hid::registry::Info;
use hidapi::HidApi;

pub const VID: u16 = 0x2516;
pub const IDS: &[(u16, &str)] = &[(0x017B, "Cooler Master XG750 Plus")];
const INTERFACE: i32 = 0;
const PAGE: u16 = 0xFF00;
const WRITE_LEN: usize = 65;
const READ_LEN: usize = 65;
const REPLY_MS: u64 = 500;
const POLL_MS: u64 = 1000;
const TELEMETRY: [u8; 4] = [0x00, 0x80, 0x06, 0x01];
const REPLY: [u8; 3] = [0x80, 0x06, 0x03];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Telemetry {
    pub rpm: u16,
    pub watts: u16,
    pub celsius: u16,
}

/// Where the reply header starts: first byte, or after a leading report id 0.
fn header(r: &[u8]) -> Option<usize> {
    [0, 1]
        .into_iter()
        .find(|&at| r.get(at..at + 3) == Some(&REPLY[..]))
}

pub fn parse(r: &[u8]) -> Option<Telemetry> {
    let at = header(r)? + 3;
    Some(Telemetry {
        rpm: u16le(r, at)?,
        watts: u16le(r, at + 2)?,
        celsius: u16le(r, at + 4)?,
    })
}

struct Psu {
    io: Box<dyn Io>,
    id: String,
    name: &'static str,
    last: Cached<Telemetry>,
}

impl Psu {
    fn read(&mut self) -> Option<Telemetry> {
        let io = &mut self.io;
        self.last.get(|| {
            let r = ask_strict(
                io.as_mut(),
                &TELEMETRY,
                WRITE_LEN,
                READ_LEN,
                REPLY_MS,
                |r| header(r).is_some(),
            );
            parse(&r.ok()?)
        })
    }
}

impl FanHub for Psu {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn name(&self) -> String {
        self.name.into()
    }
    fn channels(&self) -> Vec<Channel> {
        vec![Channel::fan("Fan")]
    }
    fn sensors(&self) -> Vec<String> {
        vec!["Temperature".into(), "Power".into()]
    }
    fn sensor_kind(&self, sensor: usize) -> Kind {
        if sensor == 1 {
            Kind::Power
        } else {
            Kind::Temperature
        }
    }
    fn rpm(&mut self, _channel: usize) -> Option<u32> {
        self.read().map(|t| u32::from(t.rpm))
    }
    fn set_duty(&mut self, _channel: usize, _duty: f32) -> Result<(), String> {
        Err("the power supply fan is read only".into())
    }
    fn release(&mut self, _channel: usize) -> Result<(), String> {
        Ok(())
    }
    fn temperature(&mut self, sensor: usize) -> Option<f32> {
        let t = self.read()?;
        Some(f32::from(if sensor == 1 { t.watts } else { t.celsius }))
    }
}

pub fn wants(info: &Info) -> bool {
    info.vid == VID
        && IDS.iter().any(|i| i.0 == info.pid)
        && info.interface == INTERFACE
        && info.usage_page == PAGE
}

pub fn hub(api: &HidApi, info: &Info) -> Option<Box<dyn FanBackend>> {
    let name = IDS.iter().find(|i| i.0 == info.pid)?.1;
    Some(backend_on(
        info,
        Psu {
            io: open_io(api, info).ok()?,
            id: fan_id(info, "coolermaster-psu"),
            name,
            last: Cached::new(POLL_MS),
        },
    ))
}

pub fn key(info: &Info) -> String {
    device_key(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Rec {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        reply: Vec<u8>,
        owed: Arc<Mutex<bool>>,
    }

    impl Io for Rec {
        fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push(crate::hid::padded(data, len));
            *self.owed.lock().unwrap() = true;
            Ok(())
        }
        fn feature(&mut self, _: &[u8], _: usize) -> Result<(), String> {
            Ok(())
        }
        fn get_feature(&mut self, _: u8, _: usize) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
        fn read(&mut self, _: usize, _: i32) -> Result<Vec<u8>, String> {
            let owed = std::mem::take(&mut *self.owed.lock().unwrap());
            Ok(if owed { self.reply.clone() } else { Vec::new() })
        }
    }

    fn psu(reply: &[u8]) -> (Rec, Psu) {
        let rec = Rec {
            reply: reply.to_vec(),
            ..Rec::default()
        };
        let psu = Psu {
            io: Box::new(rec.clone()),
            id: String::new(),
            name: "XG",
            last: Cached::new(60_000),
        };
        (rec, psu)
    }

    #[test]
    fn telemetry_reply_decodes() {
        let r = [0x80, 0x06, 0x03, 0x93, 0x02, 0xBA, 0x00, 0x12, 0x00];
        let want = Telemetry {
            rpm: 659,
            watts: 186,
            celsius: 18,
        };
        assert_eq!(parse(&r), Some(want));
        let mut with_id = vec![0];
        with_id.extend(r);
        assert_eq!(parse(&with_id), Some(want));
        assert_eq!(parse(&[0x80, 0x01, 0x03, 0x42]), None);
    }

    #[test]
    fn only_the_telemetry_request_is_sent() {
        let mut reply = vec![0x80, 0x06, 0x03, 0x93, 0x02, 0xBA, 0x00, 0x12, 0x00];
        reply.resize(64, 0);
        let (rec, mut psu) = psu(&reply);
        assert_eq!(psu.rpm(0), Some(659));
        assert_eq!(psu.temperature(0), Some(18.0));
        assert_eq!(psu.temperature(1), Some(186.0));
        assert_eq!(psu.sensor_kind(1), Kind::Power);
        assert!(psu.set_duty(0, 0.2).is_err());
        assert!(psu.release(0).is_ok());
        let sent = rec.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let mut want = vec![0x00, 0x80, 0x06, 0x01];
        want.resize(WRITE_LEN, 0);
        assert_eq!(sent[0], want);
    }
}
