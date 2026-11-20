//! Shared plumbing for this group's fan hubs: request/reply exchanges and short-lived caches.

use crate::hid::{Io, drain};
use foxden_hid::registry::Info;
use std::time::{Duration, Instant};

/// Drops waiting reports, sends `packet` and returns the first reply `want` accepts, else the
/// first reply that came at all (other handles' acks reach every open handle).
pub fn ask(
    io: &mut dyn Io,
    packet: &[u8],
    write_len: usize,
    read_len: usize,
    timeout_ms: u64,
    want: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    drain(io, read_len);
    io.write(packet, write_len)?;
    let end = Instant::now() + Duration::from_millis(timeout_ms);
    let mut first = None;
    while Instant::now() < end {
        let left = end.saturating_duration_since(Instant::now()).as_millis() as i32;
        let r = io.read(read_len, left.max(1))?;
        if r.is_empty() {
            continue;
        }
        if want(&r) {
            return Ok(r);
        }
        first.get_or_insert(r);
    }
    first.ok_or_else(|| "no reply".into())
}

/// Like `ask`, but only a reply `want` accepts counts.
pub fn ask_strict(
    io: &mut dyn Io,
    packet: &[u8],
    write_len: usize,
    read_len: usize,
    timeout_ms: u64,
    want: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    drain(io, read_len);
    io.write(packet, write_len)?;
    crate::hid::read_until(io, read_len, timeout_ms, want)
}

/// A value read from the device, trusted for `max_age`.
pub struct Cached<T> {
    value: Option<T>,
    at: Option<Instant>,
    max_age: Duration,
}

impl<T: Clone> Cached<T> {
    pub fn new(max_age_ms: u64) -> Self {
        Cached {
            value: None,
            at: None,
            max_age: Duration::from_millis(max_age_ms),
        }
    }

    /// The cached value, stale or not.
    pub fn peek(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// The cached value, refreshed with `read` when stale. A failed read keeps nothing.
    pub fn get(&mut self, read: impl FnOnce() -> Option<T>) -> Option<T> {
        if self.at.is_none_or(|t| t.elapsed() >= self.max_age) {
            self.value = read();
            self.at = Some(Instant::now());
        }
        self.value.clone()
    }
}

/// True when `last` is unset or older than `ms`.
pub fn due(last: Option<Instant>, ms: u64) -> bool {
    last.is_none_or(|t| t.elapsed() >= Duration::from_millis(ms))
}

/// `fan:<family>-<vidpid>-<serial or path hash>`.
pub fn fan_id(info: &Info, family: &str) -> String {
    info.device_id(family).replacen("hid:", "fan:", 1)
}

/// One physical device: the serial when it has one, else its only collection's path.
pub fn device_key(info: &Info) -> String {
    if info.serial.is_empty() {
        format!("{:04x}{:04x}{}", info.vid, info.pid, info.path)
    } else {
        info.key()
    }
}

pub fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from(*b.get(i)?) | u16::from(*b.get(i + 1)?) << 8)
}

pub fn u16be(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from(*b.get(i)?) << 8 | u16::from(*b.get(i + 1)?))
}
