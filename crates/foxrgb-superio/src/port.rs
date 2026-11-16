//! One Super I/O configuration port, 0x2E or 0x4E, and the chip's I/O ports behind it.

use std::time::{Duration, Instant};

use crate::lights::Pawn;

pub(crate) struct Lpc {
    pawn: Box<dyn Pawn>,
    pub port: u16,
    sleep: fn(Duration),
}

impl Lpc {
    pub fn new(pawn: Box<dyn Pawn>, port: u16) -> Lpc {
        let _ = pawn.exec("ioctl_select_slot", &[u64::from(port == 0x4E)], 0);
        Lpc {
            pawn,
            port,
            sleep: std::thread::sleep,
        }
    }

    #[cfg(test)]
    pub fn without_sleep(mut self) -> Lpc {
        self.sleep = |_| {};
        self
    }

    pub fn sleep(&self, ms: u64) {
        (self.sleep)(Duration::from_millis(ms))
    }

    /// Polls about once a millisecond until `ready`, for at most `ms` milliseconds.
    pub fn until(&self, ms: u64, mut ready: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if ready() {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            self.sleep(1);
        }
    }

    pub fn inb(&self, port: u16) -> u8 {
        self.pawn
            .read("ioctl_pio_inb", &[port as u64])
            .map_or(0xFF, |v| v as u8)
    }

    pub fn outb(&self, port: u16, value: u8) {
        let _ = self
            .pawn
            .exec("ioctl_pio_outb", &[port as u64, value as u64], 0);
    }

    pub fn read(&self, reg: u8) -> u8 {
        self.pawn
            .read("ioctl_superio_inb", &[reg as u64])
            .map_or(0xFF, |v| v as u8)
    }

    pub fn read_word(&self, reg: u8) -> u16 {
        self.pawn
            .read("ioctl_superio_inw", &[reg as u64])
            .map_or(0xFFFF, |v| v as u16)
    }

    pub fn write(&self, reg: u8, value: u8) {
        let _ = self
            .pawn
            .exec("ioctl_superio_outb", &[reg as u64, value as u64], 0);
    }

    pub fn find_bars(&self) {
        let _ = self.pawn.exec("ioctl_find_bars", &[], 0);
    }

    pub fn select(&self, ldn: u8) {
        self.write(0x07, ldn);
    }

    pub fn nuvoton_enter(&self) {
        self.outb(self.port, 0x87);
        self.outb(self.port, 0x87);
    }

    pub fn nuvoton_exit(&self) {
        self.outb(self.port, 0xAA);
    }

    /// Clears the hardware monitor I/O space lock on chips that have one.
    pub fn nuvoton_unlock(&self) {
        let options = self.read(0x28);
        if options & 0x10 != 0 {
            self.write(0x28, options & !0x10);
        }
    }

    pub fn ite_enter(&self) {
        for key in [
            0x87,
            0x01,
            0x55,
            if self.port == 0x4E { 0xAA } else { 0x55 },
        ] {
            self.outb(self.port, key);
        }
    }

    /// The second chip on 0x4E stays in configuration mode.
    pub fn ite_exit(&self) {
        if self.port != 0x4E {
            self.outb(self.port, 0x02);
            self.outb(self.port + 1, 0x02);
        }
    }
}
