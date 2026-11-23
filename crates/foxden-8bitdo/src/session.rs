//! Talking to one pad: read and write its config block, store it, switch its light theme.

use std::time::{Duration, Instant};

use crate::config::{Config, Span};
use crate::model::{Model, Theme};
use crate::proto::{self, ACK, COMMIT_ARG, REPORT, Reply, Wire, cmd};

/// Whatever carries reports to and from the vendor collection.
pub trait Link {
    fn send(&mut self, report: &[u8]) -> bool;
    fn recv(&mut self, wait: Duration) -> Option<Vec<u8>>;
    fn flush(&mut self) {}
    fn pause(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// The vendor library tries each step this many times.
const TRIES: usize = 30;
const ANSWER: Duration = Duration::from_millis(150);
/// How long the pad may take to store its block.
const STORE: Duration = Duration::from_secs(4);

pub struct Session<L: Link> {
    pub link: L,
    pub model: &'static Model,
    /// Sends per step before giving up; fewer for best-effort calls.
    pub tries: usize,
}

impl<L: Link> Session<L> {
    pub fn new(link: L, model: &'static Model) -> Self {
        Session {
            link,
            model,
            tries: TRIES,
        }
    }

    fn wire(&self) -> Wire {
        self.model.wire
    }

    fn wait(&mut self, want: u16, for_: Duration, any_status: bool) -> Option<Reply> {
        let end = Instant::now() + for_;
        loop {
            let left = end.checked_duration_since(Instant::now())?;
            let r = self.link.recv(left.min(Duration::from_millis(50)));
            if let Some(a) = r.as_deref().and_then(proto::reply)
                && (any_status || a.status == ACK)
                && a.cmd == want
            {
                return Some(a);
            }
        }
    }

    fn exchange(&mut self, req: &[u8; REPORT], want: u16, any_status: bool) -> Option<Reply> {
        for _ in 0..self.tries.max(1) {
            self.link.flush();
            if self.link.send(req)
                && let Some(a) = self.wait(want, ANSWER, any_status)
            {
                return Some(a);
            }
        }
        None
    }

    fn ask(&mut self, req: &[u8; REPORT], want: u16) -> Option<Reply> {
        self.exchange(req, want, false)
    }

    /// `len` bytes from `at`, in a block of `total` bytes.
    pub fn read_span(&mut self, at: usize, len: usize, total: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let off = at + out.len();
            let req = proto::request(
                self.wire(),
                cmd::READ,
                0,
                None,
                total - off,
                total as u32,
                off as u32,
            );
            let a = self
                .ask(&req, cmd::READ)
                .ok_or_else(|| format!("no answer reading byte {off}"))?;
            let n = (a.count as usize).min(a.data.len()).min(len - out.len());
            if n == 0 {
                return Err(format!("empty answer at byte {off}"));
            }
            out.extend_from_slice(&a.data[..n]);
        }
        Ok(out)
    }

    pub fn read(&mut self) -> Result<Config, String> {
        let l = self.model.layout;
        Config::new(l, self.read_span(0, l.size, l.size)?)
    }

    /// The profile in use, read the way the vendor app does: the first 20 bytes alone.
    pub fn current(&mut self) -> Result<u16, String> {
        let l = self.model.layout;
        let head = self.read_span(0, l.slot + 2, l.slot + 2)?;
        Ok(u16::from_le_bytes([head[l.slot], head[l.slot + 1]]))
    }

    /// Sends `data` for `at..` of a block of `total` bytes; the pad holds it until `store`.
    pub fn write_span(&mut self, at: usize, data: &[u8], total: usize) -> Result<(), String> {
        let mut done = 0;
        while done < data.len() {
            let off = at + done;
            let rest = &data[done..];
            let req = proto::request(
                self.wire(),
                cmd::WRITE,
                0,
                Some(rest),
                rest.len(),
                total as u32,
                off as u32,
            );
            let a = self
                .ask(&req, cmd::WRITE)
                .ok_or_else(|| format!("no answer writing byte {off}"))?;
            if a.count == 0 {
                return Err(format!("byte {off} refused"));
            }
            done += a.count as usize;
            self.link.pause(10);
        }
        Ok(())
    }

    /// Tells the pad to keep what it was sent.
    pub fn store(&mut self) -> Result<(), String> {
        self.link.pause(100);
        self.link.flush();
        if !self
            .link
            .send(&proto::short(self.wire(), cmd::COMMIT, COMMIT_ARG))
        {
            return Err("could not send the store request".into());
        }
        self.wait(cmd::COMMIT, STORE, false)
            .map(|_| ())
            .ok_or_else(|| "the pad did not confirm it stored the settings".into())
    }

    /// Input reports pause while the vendor app writes, then resume.
    pub fn reports(&mut self, on: bool) -> bool {
        self.link
            .send(&proto::short(self.wire(), cmd::REPORTS, on as u16))
    }

    /// Writes the given spans of `c` (all of it when `spans` is empty) and stores them, with input
    /// reports paused as the vendor app does.
    pub fn write(&mut self, c: &Config, spans: &[Span]) -> Result<(), String> {
        let total = c.layout.size;
        self.reports(false);
        self.link.pause(30);
        let all = if spans.is_empty() {
            vec![Span { at: 0, len: total }]
        } else {
            spans.to_vec()
        };
        let result = all
            .iter()
            .try_for_each(|s| self.write_span(s.at, &c.bytes[s.at..s.at + s.len], total))
            .and_then(|_| self.store());
        self.link.pause(30);
        self.reports(true);
        result
    }

    /// Only the custom colours of profile `slot`, as `writeUltimate2single` sends them: the span
    /// alone, without the block's CRC.
    pub fn write_leds(&mut self, slot: usize, leds: &[u32]) -> Result<(), String> {
        let l = self.model.layout;
        let li = l.light.ok_or("this pad has no light ring")?;
        if slot >= l.slots || leds.len() != li.leds {
            return Err("wrong profile or LED count".into());
        }
        let mut span = Vec::with_capacity(4 + 4 * li.leds);
        span.extend(crate::config::SET.to_le_bytes());
        for c in leds {
            span.extend((c & 0xFF_FFFF).to_le_bytes());
        }
        self.reports(false);
        self.link.pause(30);
        let at = li.single + slot * span.len();
        let result = self
            .write_span(at, &span, l.size)
            .and_then(|_| self.store());
        self.link.pause(30);
        self.reports(true);
        result
    }

    pub fn set_theme(&mut self, t: Theme) -> Result<(), String> {
        let req = proto::bare(self.wire(), cmd::SET_THEME, t.code());
        self.ask(&req, cmd::SET_THEME)
            .map(|_| ())
            .ok_or_else(|| "the pad did not take the light theme".into())
    }

    pub fn theme(&mut self) -> Result<Theme, String> {
        let req = proto::bare(self.wire(), cmd::THEME, 0);
        let a = self.ask(&req, cmd::THEME).ok_or("no answer")?;
        Ok(Theme::from_code(*a.data.first().unwrap_or(&0) as u16))
    }

    /// Whether a receiver has its pad: `None` when nothing answered (a wired pad may not).
    pub fn linked(&mut self) -> Option<bool> {
        let w = Wire {
            long: true,
            crc: false,
        };
        let req = proto::request(w, cmd::LINK, 0, None, 0, 0, 0);
        let a = self.exchange(&req, cmd::LINK, true)?;
        Some(a.data.first().is_some_and(|&b| b != 0))
    }

    /// Battery percent and charging, the same query the battery module makes.
    pub fn battery(&mut self) -> Option<(u8, bool)> {
        let w = Wire {
            long: true,
            crc: false,
        };
        let req = proto::request(w, cmd::LEVEL, 0, None, 6, 0, 0);
        let a = self.exchange(&req, cmd::LEVEL, true)?;
        let b = *a.data.first()?;
        let pct = b & 0x7F;
        (1..=100)
            .contains(&pct)
            .then_some((pct, b & 0x80 != 0 && pct < 100))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::proto::{IN, REPORT};
    use std::collections::VecDeque;

    /// A pad that answers like the vendor firmware: acks, block reads and writes into memory.
    pub struct Pad {
        pub block: Vec<u8>,
        pub sent: Vec<Vec<u8>>,
        pub stored: usize,
        pub theme: u8,
        queue: VecDeque<Vec<u8>>,
        long: bool,
    }

    impl Pad {
        pub fn new(size: usize, long: bool) -> Pad {
            Pad {
                block: (0..size).map(|i| i as u8).collect(),
                sent: Vec::new(),
                stored: 0,
                theme: 5,
                queue: VecDeque::new(),
                long,
            }
        }

        fn answer(&mut self, cmd: u16, count: u16, data: &[u8]) {
            let mut r = vec![0u8; REPORT];
            r[0] = IN;
            r[1] = 0x04;
            r[2..4].copy_from_slice(&ACK.to_le_bytes());
            r[4..6].copy_from_slice(&cmd.to_le_bytes());
            r[6..8].copy_from_slice(&count.to_le_bytes());
            r[18..18 + data.len()].copy_from_slice(data);
            self.queue.push_back(r);
        }
    }

    impl Link for Pad {
        fn send(&mut self, report: &[u8]) -> bool {
            self.sent.push(report.to_vec());
            let h = if self.long {
                &report[2..]
            } else {
                &report[3..]
            };
            let u16_at = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]);
            let u32_at = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().unwrap());
            let (c, arg, len, off) = (
                u16_at(0),
                u16_at(2),
                u16_at(4) as usize,
                u32_at(12) as usize,
            );
            match c {
                cmd::READ => {
                    let n = len.min(46).min(self.block.len() - off);
                    let d = self.block[off..off + n].to_vec();
                    self.answer(c, n as u16, &d);
                }
                cmd::WRITE => {
                    let d = h[16..16 + len].to_vec();
                    self.block[off..off + len].copy_from_slice(&d);
                    self.answer(c, len as u16, &[]);
                }
                cmd::COMMIT => {
                    self.stored += 1;
                    self.answer(c, 0, &[]);
                }
                cmd::SET_THEME => {
                    self.theme = arg as u8;
                    self.answer(c, 0, &[]);
                }
                cmd::THEME => {
                    let t = self.theme;
                    self.answer(c, 1, &[t]);
                }
                _ => {}
            }
            true
        }

        fn recv(&mut self, _wait: Duration) -> Option<Vec<u8>> {
            self.queue.pop_front()
        }

        fn pause(&mut self, _ms: u64) {}
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Pad;
    use super::*;
    use crate::config::Profile;
    use crate::model::model;

    #[test]
    fn reads_and_writes_a_whole_block() {
        let m = model(0x6012).unwrap();
        let mut s = Session::new(Pad::new(m.layout.size, true), m);
        let mut c = s.read().unwrap();
        assert_eq!(c.bytes, s.link.block);
        assert_eq!(s.current().unwrap(), u16::from_le_bytes([18, 19]));
        let mut p: Profile = c.profile(0).unwrap();
        p.name = "FoxDen".into();
        p.lights.as_mut().unwrap().leds = vec![0x00FF00; 24];
        let spans = c.set_profile(0, &p).unwrap();
        let before = s.link.sent.len();
        s.write(&c, &spans).unwrap();
        assert_eq!(s.link.block, c.bytes);
        assert_eq!(s.link.stored, 1);
        let first = &s.link.sent[before];
        assert_eq!(&first[..6], &[0x81, 0x04, 0x07, 0x00, 0x00, 0x00]);
        let last = s.link.sent.last().unwrap();
        assert_eq!(&last[..6], &[0x81, 0x04, 0x07, 0x00, 0x01, 0x00]);
        c.set_mode(crate::config::Mode::Switch);
        s.write(&c, &[]).unwrap();
        assert_eq!(s.link.block, c.bytes);
    }

    #[test]
    fn classic_pads_use_the_short_frame() {
        let m = model(0x3012).unwrap();
        let mut s = Session::new(Pad::new(m.layout.size, false), m);
        let c = s.read().unwrap();
        assert_eq!(c.bytes.len(), 0x914);
        assert_eq!(s.link.sent[0][1], 16 + 45 + 1);
    }

    #[test]
    fn theme_switches() {
        let m = model(0x600F).unwrap();
        let mut s = Session::new(Pad::new(m.layout.size, true), m);
        s.set_theme(Theme::Custom).unwrap();
        assert_eq!(s.theme().unwrap(), Theme::Custom);
    }
}
