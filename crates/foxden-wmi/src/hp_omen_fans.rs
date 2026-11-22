// SPDX-License-Identifier: GPL-2.0-or-later
//! Omen and Victus fan speeds through the BIOS WMI interface. Read only.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use foxden_fan::{Fan, FanBackend, Sensor};

use crate::hp_omen::{Bios, COMMAND_INFO, Wmi};

const FAN_SPEED: u32 = 0x11;
const FAN_LEVELS: u32 = 0x2D;
const NAMES: [&str; 2] = ["CPU fan", "GPU fan"];
const EVERY: Duration = Duration::from_secs(2);
const KEEP: Duration = Duration::from_secs(8);

/// Boards that report every fan in one call, in hundreds of rpm.
const LEVEL_BOARDS: &[&str] = &[
    "8902", "8A3D", "8A44", "8A4D", "8BAA", "8BA9", "8BAB", "8B2F", "8BB3", "8BBE", "8BC2", "8BCA",
    "8BCD", "8BD4", "8BD5", "8C76", "8C77", "8C78", "8C99", "8C9C", "8D26", "8D3F", "8D40", "8D41",
    "8D42", "8D87", "8D88", "8DD0", "8DD6", "8DE1", "8E35", "8E41", "8F1C",
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Method {
    Speed,
    Levels,
}

fn method(board: Option<&str>) -> Method {
    match board {
        Some(b) if LEVEL_BOARDS.iter().any(|l| l.eq_ignore_ascii_case(b)) => Method::Levels,
        _ => Method::Speed,
    }
}

/// Rpm of `fan`, or `None` when the BIOS refuses.
fn read(bios: &dyn Bios, how: Method, fan: usize) -> Option<u32> {
    match how {
        Method::Speed => {
            let d = bios.query(COMMAND_INFO, FAN_SPEED, &[fan as u8]).ok()?;
            Some(u32::from(*d.get(2)?) << 8 | u32::from(*d.get(3)?))
        }
        Method::Levels => {
            let d = bios.query(COMMAND_INFO, FAN_LEVELS, &[0]).ok()?;
            Some(u32::from(*d.get(fan)?) * 100)
        }
    }
}

type Latest = Mutex<Vec<Option<(Instant, u32)>>>;

#[derive(Clone)]
struct OmenFan {
    id: String,
    name: &'static str,
    slot: usize,
    latest: Arc<Latest>,
    _alive: Sender<()>,
}

impl Fan for OmenFan {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn rpm(&mut self) -> Option<u32> {
        let (at, rpm) = self.latest.lock().unwrap_or_else(|e| e.into_inner())[self.slot]?;
        (at.elapsed() < KEEP).then_some(rpm)
    }
    fn set_duty(&mut self, _duty: f32) -> Result<(), String> {
        Err("Omen fans are read only".into())
    }
    fn release(&mut self) -> Result<(), String> {
        Ok(())
    }
}

struct Omen {
    fans: Vec<OmenFan>,
}

impl FanBackend for Omen {
    fn name(&self) -> &str {
        "hp omen"
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        Vec::new()
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        self.fans
            .iter()
            .cloned()
            .map(|f| Box::new(f) as Box<dyn Fan>)
            .collect()
    }
}

pub fn detect() -> Vec<Box<dyn FanBackend>> {
    detect_with(Box::new(Wmi)).into_iter().collect()
}

/// The fans that answer, read on a thread of their own so a stalled call holds no fan tick.
fn detect_with(bios: Box<dyn Bios>) -> Option<Box<dyn FanBackend>> {
    let how = method(bios.board().as_deref());
    let found: Vec<(usize, u32)> = (0..NAMES.len())
        .filter_map(|fan| Some((fan, read(bios.as_ref(), how, fan)?)))
        .collect();
    if found.is_empty() {
        return None;
    }
    let now = Instant::now();
    let latest: Arc<Latest> = Arc::new(Mutex::new(
        found.iter().map(|&(_, rpm)| Some((now, rpm))).collect(),
    ));
    let (alive, gone) = mpsc::channel();
    let fans = found
        .iter()
        .enumerate()
        .map(|(slot, &(fan, _))| OmenFan {
            id: format!("wmi:hp-omen/fan{fan}"),
            name: NAMES[fan],
            slot,
            latest: latest.clone(),
            _alive: alive.clone(),
        })
        .collect();
    let numbers: Vec<usize> = found.iter().map(|&(fan, _)| fan).collect();
    std::thread::spawn(move || {
        while let Err(RecvTimeoutError::Timeout) = gone.recv_timeout(EVERY) {
            for (slot, &fan) in numbers.iter().enumerate() {
                if let Some(rpm) = read(bios.as_ref(), how, fan) {
                    latest.lock().unwrap_or_else(|e| e.into_inner())[slot] =
                        Some((Instant::now(), rpm));
                }
            }
        }
    });
    Some(Box::new(Omen { fans }))
}

#[cfg(test)]
mod tests {
    use super::*;

    type Log = Arc<Mutex<Vec<(u32, u32, Vec<u8>)>>>;

    struct Fake {
        log: Log,
        board: Option<&'static str>,
        fans: usize,
    }

    impl Bios for Fake {
        fn execute(&self, _: u32, _: u32, _: &[u8], _: bool) -> Result<Vec<u8>, String> {
            Err("not expected".into())
        }
        fn query(&self, command: u32, kind: u32, input: &[u8]) -> Result<Vec<u8>, String> {
            self.log
                .lock()
                .unwrap()
                .push((command, kind, input.to_vec()));
            let mut d = vec![0u8; 128];
            match kind {
                FAN_SPEED if (input[0] as usize) < self.fans => {
                    d[2..4].copy_from_slice(&(2400 + u16::from(input[0])).to_be_bytes());
                }
                FAN_LEVELS => d[..2].copy_from_slice(&[31, 45]),
                _ => return Err("BIOS returned UI4(4)".into()),
            }
            Ok(d)
        }
        fn board(&self) -> Option<String> {
            self.board.map(String::from)
        }
    }

    fn run(board: Option<&'static str>, fans: usize) -> (Log, Option<Box<dyn FanBackend>>) {
        let log = Log::default();
        let fake = Fake {
            log: log.clone(),
            board,
            fans,
        };
        (log.clone(), detect_with(Box::new(fake)))
    }

    fn rpms(backend: &mut Box<dyn FanBackend>) -> Vec<(String, Option<u32>)> {
        backend
            .fans()
            .iter_mut()
            .map(|f| (f.id().to_string(), f.rpm()))
            .collect()
    }

    #[test]
    fn speed_query_reads_big_endian_rpm_per_fan() {
        let (log, found) = run(Some("88FA"), 1);
        let mut found = found.unwrap();
        assert_eq!(
            log.lock().unwrap()[..2],
            [(131080, 0x11, vec![0]), (131080, 0x11, vec![1])]
        );
        assert_eq!(rpms(&mut found), [("wmi:hp-omen/fan0".into(), Some(2400))]);
    }

    #[test]
    fn level_boards_read_hundreds_of_rpm() {
        let (log, found) = run(Some("8bd5"), 0);
        let mut found = found.unwrap();
        assert_eq!(log.lock().unwrap()[0], (131080, 0x2D, vec![0]));
        assert_eq!(
            rpms(&mut found),
            [
                ("wmi:hp-omen/fan0".into(), Some(3100)),
                ("wmi:hp-omen/fan1".into(), Some(4500))
            ]
        );
    }

    #[test]
    fn refused_queries_find_nothing_and_fans_never_write() {
        assert!(run(None, 0).1.is_none());
        let (log, found) = run(None, 2);
        let mut fan = found.unwrap().fans().remove(0);
        let before = log.lock().unwrap().len();
        assert!(fan.set_duty(1.0).is_err());
        assert!(fan.release().is_ok());
        assert_eq!(log.lock().unwrap().len(), before);
    }
}
