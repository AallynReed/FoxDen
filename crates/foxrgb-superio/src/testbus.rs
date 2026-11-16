//! A pretend LPC bus for tests: configuration registers, chip register files and a log.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::lights::Pawn;
use crate::port::Lpc;

/// Something answering on I/O ports.
pub trait Device: Send {
    fn inb(&mut self, port: u16) -> u8;
    fn outb(&mut self, port: u16, value: u8);
}

#[derive(Default)]
pub struct State {
    pub log: Vec<String>,
    /// Configuration registers by (logical device, register); below 0x30 the device is ignored.
    pub config: HashMap<(u8, u8), u8>,
    ldn: u8,
    devices: Vec<(u16, u16, Box<dyn Device>)>,
}

#[derive(Clone, Default)]
pub struct Bus(pub Arc<Mutex<State>>);

impl Bus {
    pub fn config(&self, ldn: u8, reg: u8, value: u8) -> &Self {
        let key = if reg < 0x30 { (0, reg) } else { (ldn, reg) };
        self.0.lock().unwrap().config.insert(key, value);
        self
    }

    pub fn word(&self, ldn: u8, reg: u8, value: u16) -> &Self {
        self.config(ldn, reg, (value >> 8) as u8)
            .config(ldn, reg + 1, value as u8)
    }

    pub fn device(&self, first: u16, last: u16, device: impl Device + 'static) -> &Self {
        self.0
            .lock()
            .unwrap()
            .devices
            .push((first, last, Box::new(device)));
        self
    }

    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap().log)
    }

    pub fn lpc(&self, port: u16) -> Lpc {
        let lpc = Lpc::new(Box::new(self.clone()), port).without_sleep();
        self.take();
        lpc
    }
}

impl State {
    fn cfg(&self, reg: u8) -> u8 {
        let key = if reg < 0x30 {
            (0, reg)
        } else {
            (self.ldn, reg)
        };
        self.config.get(&key).copied().unwrap_or(0xFF)
    }

    fn device(&mut self, port: u16) -> Option<&mut Box<dyn Device>> {
        self.devices
            .iter_mut()
            .find(|(a, b, _)| (*a..=*b).contains(&port))
            .map(|(.., d)| d)
    }
}

impl Pawn for Bus {
    fn exec(&self, name: &str, input: &[u64], _outputs: usize) -> Result<Vec<u64>, String> {
        let name = name.trim_start_matches("ioctl_");
        let mut s = self.0.lock().unwrap();
        let args: Vec<String> = input.iter().map(|v| format!("{v:02x}")).collect();
        s.log
            .push([name.to_string(), args.join(" ")].join(" ").trim().into());
        let a = |i: usize| input.get(i).copied().unwrap_or(0);
        let out = match name {
            "superio_inb" => s.cfg(a(0) as u8) as u64,
            "superio_inw" => (s.cfg(a(0) as u8) as u64) << 8 | s.cfg(a(0) as u8 + 1) as u64,
            "superio_outb" => {
                if a(0) == 0x07 {
                    s.ldn = a(1) as u8;
                }
                let key = if a(0) < 0x30 {
                    (0, a(0) as u8)
                } else {
                    (s.ldn, a(0) as u8)
                };
                s.config.insert(key, a(1) as u8);
                return Ok(Vec::new());
            }
            "pio_inb" => s.device(a(0) as u16).map_or(0xFF, |d| d.inb(a(0) as u16)) as u64,
            "pio_outb" => {
                if let Some(d) = s.device(a(0) as u16) {
                    d.outb(a(0) as u16, a(1) as u8);
                }
                return Ok(Vec::new());
            }
            _ => return Ok(Vec::new()),
        };
        Ok(vec![out])
    }
}

/// A shared register file, so a test can look inside after handing a device to the bus.
pub type Regs = Arc<Mutex<HashMap<u16, u8>>>;

pub fn regs(values: &[(u16, u8)]) -> Regs {
    Arc::new(Mutex::new(values.iter().copied().collect()))
}

fn get(regs: &Regs, at: u16) -> u8 {
    regs.lock().unwrap().get(&at).copied().unwrap_or(0)
}

/// Index at base+5, data at base+6; register 0x4E selects the bank.
pub struct Banked {
    pub base: u16,
    pub regs: Regs,
    index: u8,
    bank: u8,
}

impl Banked {
    pub fn new(base: u16, regs: &Regs) -> Banked {
        Banked {
            base,
            regs: regs.clone(),
            index: 0,
            bank: 0,
        }
    }
}

impl Device for Banked {
    fn inb(&mut self, port: u16) -> u8 {
        match port - self.base {
            5 => self.index,
            6 => get(&self.regs, (self.bank as u16) << 8 | self.index as u16),
            _ => 0xFF,
        }
    }
    fn outb(&mut self, port: u16, value: u8) {
        match port - self.base {
            5 => self.index = value,
            6 if self.index == 0x4E => self.bank = value,
            6 => {
                let at = (self.bank as u16) << 8 | self.index as u16;
                self.regs.lock().unwrap().insert(at, value);
            }
            _ => {}
        }
    }
}

/// Index at base+5 that reads back, data at base+6, no banks.
pub struct Flat {
    pub base: u16,
    pub regs: Regs,
    index: u8,
    /// Whether reading the index port gives back what was written.
    pub echo: bool,
}

impl Flat {
    pub fn new(base: u16, regs: &Regs) -> Flat {
        Flat {
            base,
            regs: regs.clone(),
            index: 0,
            echo: true,
        }
    }
}

impl Device for Flat {
    fn inb(&mut self, port: u16) -> u8 {
        match port - self.base {
            5 if self.echo => self.index,
            6 => get(&self.regs, self.index as u16),
            _ => 0xFF,
        }
    }
    fn outb(&mut self, port: u16, value: u8) {
        match port - self.base {
            5 => self.index = value,
            6 => {
                self.regs.lock().unwrap().insert(self.index as u16, value);
            }
            _ => {}
        }
    }
}

pub fn log(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|s| s.to_string()).collect()
}
