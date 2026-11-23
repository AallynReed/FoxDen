//! 8BitDo controllers over their vendor channel: every setting of their own app (profiles,
//! button maps, sticks, triggers, vibration, motion, light rings), from a reading of
//! 8BitDo Ultimate Software V2. Untested on hardware.

pub mod config;
pub mod model;
pub mod proto;
pub mod session;

use std::time::Duration;

use foxden_battery::hid::{Coll, Dev, enumerate};

pub use config::{Config, Lights, Mode, Motion, Pair, Profile, Range, Raw, Rumble, Span};
pub use foxden_battery::eightbitdo_lock as lock;
pub use model::{Model, Theme, VID, model};
pub use session::{Link, Session};

impl Link for Dev {
    fn send(&mut self, report: &[u8]) -> bool {
        self.write(report)
    }
    fn recv(&mut self, wait: Duration) -> Option<Vec<u8>> {
        self.read(wait)
    }
    fn flush(&mut self) {
        Dev::flush(self)
    }
}

/// The vendor collection the config protocol uses.
pub fn is_channel(page: u16, usage: u16, in_len: u16, out_len: u16) -> bool {
    matches!(page, 0xFF7A | 0xFFA0) && usage == 0x0001 && in_len >= 64 && out_len >= 64
}

/// One pad, found on its vendor collection.
#[derive(Clone, Debug)]
pub struct Found {
    /// "2DC8:6013", the USB id of the pad or its receiver.
    pub key: String,
    pub model: &'static Model,
    coll: Coll,
}

/// The product id the pad reports behind a receiver or in another mode.
pub fn identify(link: &mut impl Link) -> Option<u16> {
    let w = proto::Wire {
        long: true,
        crc: false,
    };
    let req = proto::request(w, proto::cmd::INFO, 0, None, 0, 0, 0);
    for _ in 0..3 {
        link.flush();
        if !link.send(&req) {
            continue;
        }
        let end = std::time::Instant::now() + Duration::from_millis(300);
        while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
            if let Some(a) = link.recv(left).as_deref().and_then(proto::reply)
                && a.cmd == proto::cmd::INFO
                && a.data.len() >= 6
            {
                return Some(u16::from_le_bytes([a.data[4], a.data[5]]));
            }
        }
    }
    None
}

/// Pads on a vendor collection, asked who they are. Holds the vendor lock while it asks.
pub fn find() -> Vec<Found> {
    let _held = foxden_battery::eightbitdo_lock();
    let mut out: Vec<Found> = Vec::new();
    for c in enumerate() {
        if c.vid != VID || !is_channel(c.page, c.usage, c.in_len, c.out_len) {
            continue;
        }
        let key = format!("{:04X}:{:04X}", c.vid, c.pid);
        if out.iter().any(|f| f.key == key) {
            continue;
        }
        let Some(mut dev) = Dev::open(&c) else {
            continue;
        };
        let pid = identify(&mut dev).unwrap_or(c.pid);
        if let Some(model) = model(pid) {
            out.push(Found {
                key,
                model,
                coll: c,
            });
        }
    }
    out
}

impl Found {
    /// Opens the pad; the vendor lock is held while `f` runs, so battery polls wait.
    pub fn with<T>(
        &self,
        f: impl FnOnce(&mut Session<Dev>) -> Result<T, String>,
    ) -> Result<T, String> {
        let _held = foxden_battery::eightbitdo_lock();
        let dev = Dev::open(&self.coll).ok_or("the pad could not be opened")?;
        f(&mut Session::new(dev, self.model))
    }
}
