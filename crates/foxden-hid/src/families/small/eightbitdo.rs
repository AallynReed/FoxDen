// SPDX-License-Identifier: GPL-2.0-or-later
//! 8BitDo Ultimate 2 light rings. The pad keeps custom colours in its settings memory and has no
//! live mode, so only colours that hold still are written, at most every few seconds: animated
//! effects would wear the memory out and are not sent.

use std::time::{Duration, Instant};

use crate::registry::{Info, Matcher};
use foxden_8bitdo::{Link, Model, Session, Theme};
use foxden_core::{Backend, Color, Device, DeviceKind};
use hidapi::{HidApi, HidDevice};

const FAMILY: &str = "8bitdo";
/// Least time between two writes to the pad's settings memory.
const GAP: Duration = Duration::from_secs(3);
/// How long colours must stay the same before they are written.
const SETTLE: Duration = Duration::from_millis(1500);

struct Hid(HidDevice);

impl Link for Hid {
    fn send(&mut self, report: &[u8]) -> bool {
        self.0.write(report).is_ok()
    }
    fn recv(&mut self, wait: Duration) -> Option<Vec<u8>> {
        let mut buf = [0u8; 65];
        let ms = wait.as_millis().clamp(1, i32::MAX as u128) as i32;
        match self.0.read_timeout(&mut buf, ms) {
            Ok(n) if n > 0 => Some(buf[..n].to_vec()),
            _ => None,
        }
    }
}

/// Two rings of 12, clockwise from the top; the order inside a ring is a best guess.
pub fn rings(id: &str, name: &str) -> Device {
    let mut d = Device::grid(id, name, DeviceKind::Other, [0.40, 0.70, 0.20, 0.10], 24, 1);
    d.grid = [16, 8];
    d.leds = (0..24)
        .map(|i| {
            let cx = if i < 12 { 0.25 } else { 0.75 };
            let a = (i % 12) as f32 / 12.0 * std::f32::consts::TAU;
            [cx + 0.2 * a.sin(), 0.5 - 0.4 * a.cos()]
        })
        .collect();
    d
}

pub fn packed(colors: &[Color]) -> Vec<u32> {
    colors
        .iter()
        .map(|c| (c.0 as u32) << 16 | (c.1 as u32) << 8 | c.2 as u32)
        .collect()
}

/// Writing is off unless `FOXDEN_8BITDO_WRITE=1`: no pad has been driven by this code yet.
pub fn writes_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FOXDEN_8BITDO_WRITE").is_ok_and(|v| v == "1"))
}

/// What the pad was doing before the first write, read once it answers.
struct Ready {
    slot: usize,
    before: Option<Theme>,
}

pub struct Rings<L: Link + Send> {
    session: Session<L>,
    device: Device,
    writes: bool,
    ready: Option<Ready>,
    custom: bool,
    sent: Option<Vec<u32>>,
    waiting: Option<(Vec<u32>, Instant)>,
    at: Option<Instant>,
    settle: Duration,
    gap: Duration,
}

impl<L: Link + Send> Rings<L> {
    /// Sends nothing until the first write is due.
    pub fn new(mut session: Session<L>, device: Device, writes: bool) -> Self {
        session.tries = 4;
        Rings {
            session,
            device,
            writes,
            ready: None,
            custom: false,
            sent: None,
            waiting: None,
            at: None,
            settle: SETTLE,
            gap: GAP,
        }
    }

    /// The vendor app falls back to profile 0 when the pad reports none (3 or more).
    fn prepare(&mut self) -> Option<usize> {
        if let Some(r) = &self.ready {
            return Some(r.slot);
        }
        if self.session.linked() == Some(false) {
            return None;
        }
        let slot = self.session.current().ok()? as usize;
        let before = self.session.theme().ok();
        let slot = if slot < 3 { slot } else { 0 };
        self.ready = Some(Ready { slot, before });
        Some(slot)
    }

    fn send(&mut self, leds: Vec<u32>) -> Result<(), String> {
        let _held = foxden_8bitdo::lock();
        self.at = Some(Instant::now());
        let Some(slot) = self.prepare() else {
            return Ok(());
        };
        self.session.write_leds(slot, &leds)?;
        if !self.custom {
            self.session.set_theme(Theme::Custom)?;
            self.custom = true;
        }
        self.sent = Some(leds);
        Ok(())
    }
}

impl<L: Link + Send> Backend for Rings<L> {
    fn name(&self) -> &str {
        FAMILY
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        if !self.writes {
            return Ok(());
        }
        let mut leds = packed(colors);
        leds.resize(24, 0);
        if self.waiting.as_ref().map(|w| &w.0) != Some(&leds) {
            self.waiting = Some((leds, Instant::now()));
        }
        let Some((leds, since)) = &self.waiting else {
            return Ok(());
        };
        let due = self.sent.as_ref() != Some(leds)
            && since.elapsed() >= self.settle
            && self.at.is_none_or(|t| t.elapsed() >= self.gap);
        if due {
            let leds = leds.clone();
            self.send(leds)
        } else {
            Ok(())
        }
    }
}

impl<L: Link + Send> Drop for Rings<L> {
    fn drop(&mut self) {
        let before = self.ready.as_ref().and_then(|r| r.before);
        if let Some(t) = before.filter(|_| self.custom) {
            let _held = foxden_8bitdo::lock();
            self.session.tries = 2;
            let _ = self.session.set_theme(t);
        }
    }
}

fn model_of(hid: &mut Hid, info: &Info) -> Option<&'static Model> {
    let pid = foxden_8bitdo::identify(hid).unwrap_or(info.pid);
    foxden_8bitdo::model(pid).filter(|m| m.layout.light.is_some())
}

fn open(api: &HidApi, info: &Info, taken: &mut Vec<String>) -> Vec<Box<dyn Backend>> {
    if !foxden_8bitdo::is_channel(info.usage_page, info.usage, 64, 64) {
        return Vec::new();
    }
    let Ok(dev) = info.open(api) else {
        return Vec::new();
    };
    let mut hid = Hid(dev);
    let model = {
        let _held = foxden_8bitdo::lock();
        model_of(&mut hid, info)
    };
    let Some(model) = model else {
        return Vec::new();
    };
    let id = info.device_id(FAMILY);
    if taken.contains(&id) {
        return Vec::new();
    }
    taken.push(id.clone());
    let device = rings(&id, model.name);
    vec![Box::new(Rings::new(
        Session::new(hid, model),
        device,
        writes_enabled(),
    ))]
}

pub static MATCHERS: [Matcher; 2] = [
    Matcher {
        name: "8BitDo Ultimate 2",
        vid: Some(foxden_8bitdo::VID),
        interface: None,
        usage_page: Some(0xFFA0),
        usage: Some(0x0001),
        bus: None,
        open,
    },
    Matcher {
        name: "8BitDo Ultimate 2",
        vid: Some(foxden_8bitdo::VID),
        interface: None,
        usage_page: Some(0xFF7A),
        usage: Some(0x0001),
        bus: None,
        open,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Echo {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
        unlinked: bool,
    }

    impl Link for Echo {
        fn send(&mut self, report: &[u8]) -> bool {
            let cmd = u16::from_le_bytes([report[2], report[3]]);
            let len = u16::from_le_bytes([report[6], report[7]]);
            let mut r = vec![0u8; 64];
            r[..2].copy_from_slice(&[0x02, 0x04]);
            r[2] = 4;
            r[4..6].copy_from_slice(&cmd.to_le_bytes());
            r[6..8].copy_from_slice(&len.max(2).to_le_bytes());
            r[18] = if self.unlinked { 0 } else { 1 };
            self.queue.lock().unwrap().push_back(r);
            self.sent.lock().unwrap().push(report.to_vec());
            true
        }
        fn recv(&mut self, _wait: Duration) -> Option<Vec<u8>> {
            self.queue.lock().unwrap().pop_front()
        }
        fn pause(&mut self, _ms: u64) {}
    }

    #[test]
    fn nothing_is_sent_while_writing_is_off() {
        let link = Echo::default();
        let model = foxden_8bitdo::model(0x6012).unwrap();
        let mut b = Rings::new(Session::new(link.clone(), model), rings("x", "pad"), false);
        b.settle = Duration::ZERO;
        b.push("x", &[Color(255, 0, 0); 24]).unwrap();
        drop(b);
        assert!(link.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn a_receiver_without_its_pad_gets_nothing() {
        let link = Echo {
            unlinked: true,
            ..Echo::default()
        };
        let model = foxden_8bitdo::model(0x6012).unwrap();
        let mut b = Rings::new(Session::new(link.clone(), model), rings("x", "pad"), true);
        b.settle = Duration::ZERO;
        b.push("x", &[Color(255, 0, 0); 24]).unwrap();
        let cmds: Vec<u8> = link.sent.lock().unwrap().iter().map(|r| r[2]).collect();
        assert_eq!(cmds, [0x20]);
    }

    #[test]
    fn animated_colours_are_never_written() {
        let link = Echo::default();
        let model = foxden_8bitdo::model(0x600F).unwrap();
        let mut b = Rings::new(Session::new(link.clone(), model), rings("x", "pad"), true);
        for i in 0..50u8 {
            b.push("x", &[Color(i, 0, 0); 24]).unwrap();
        }
        assert!(link.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn writes_on_change_then_waits() {
        let link = Echo::default();
        let model = foxden_8bitdo::model(0x6012).unwrap();
        let mut b = Rings::new(Session::new(link.clone(), model), rings("x", "pad"), true);
        assert!(link.sent.lock().unwrap().is_empty());
        b.settle = Duration::ZERO;
        let red = vec![Color(255, 0, 0); 24];
        b.push("x", &red).unwrap();
        let sent = link.sent.lock().unwrap().clone();
        let cmds: Vec<u8> = sent.iter().map(|r| r[2]).collect();
        // Link, profile, theme, then the colours, the store and the custom theme.
        assert_eq!(
            cmds,
            [0x20, 0x02, 0x41, 0x07, 0x01, 0x01, 0x01, 0x06, 0x07, 0x40]
        );
        // single[0] of profile 0, where writeUltimate2single puts it.
        let first = &sent[4];
        assert_eq!(u32::from_le_bytes(first[14..18].try_into().unwrap()), 0x50C);
        assert_eq!(&first[18..22], &0x2020_0911u32.to_le_bytes());
        assert_eq!(&first[22..25], &[0x00, 0x00, 0xFF]);
        link.sent.lock().unwrap().clear();
        b.push("x", &red).unwrap();
        b.push("x", &[Color(0, 0, 255); 24]).unwrap();
        assert!(link.sent.lock().unwrap().is_empty());
        assert_eq!(packed(&[Color(1, 2, 3)]), [0x010203]);
    }
}
