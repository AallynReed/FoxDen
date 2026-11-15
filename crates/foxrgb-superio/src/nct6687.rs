//! Nuvoton NCT6687D on MSI boards. Registers from LibreHardwareMonitor's Nct677X.cs (MPL-2.0).

use foxrgb_pawnio::{LPC_IO, Module, SharedLock};

pub(crate) const ISA_MUTEX: &str = r"Global\Access_ISABUS.HTP.Method";
const CHIP_ID: u16 = 0xD592;
const HWM_LDN: u64 = 0x0B;
const EC_PAGE: u16 = 0x04;
const EC_INDEX: u16 = 0x05;
const EC_DATA: u16 = 0x06;
const EC_FREE: u8 = 0xFF;

pub struct Chip {
    module: Module,
    lock: SharedLock,
    base: u16,
}

impl Chip {
    /// Finds the chip at 0x2E or 0x4E. Reads only, apart from entering and leaving config mode.
    pub fn open() -> Result<Chip, String> {
        let module = Module::open(LPC_IO)?;
        let lock = SharedLock::new(ISA_MUTEX);
        let base = lock.run(|| find(&module))?;
        Ok(Chip { module, lock, base })
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    fn inb(&self, port: u16) -> u8 {
        self.module
            .execute("ioctl_pio_inb", &[port as u64], 1)
            .ok()
            .and_then(|v| v.first().copied())
            .unwrap_or(0) as u8
    }

    fn outb(&self, port: u16, value: u8) {
        let _ = self
            .module
            .execute("ioctl_pio_outb", &[port as u64, value as u64], 0);
    }

    /// One byte of the chip's EC space, as `page << 8 | index`.
    pub fn read(&self, address: u16) -> u8 {
        self.lock.run(|| {
            self.claim();
            self.outb(self.base + EC_PAGE, (address >> 8) as u8);
            self.outb(self.base + EC_INDEX, address as u8);
            let value = self.inb(self.base + EC_DATA);
            self.outb(self.base + EC_PAGE, EC_FREE);
            value
        })
    }

    pub fn write(&self, address: u16, value: u8) {
        self.lock.run(|| {
            self.claim();
            self.outb(self.base + EC_PAGE, (address >> 8) as u8);
            self.outb(self.base + EC_INDEX, address as u8);
            self.outb(self.base + EC_DATA, value);
            self.outb(self.base + EC_PAGE, EC_FREE);
        })
    }

    /// Waits up to half a second for the EC page register to be free, then takes it.
    fn claim(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while self.inb(self.base + EC_PAGE) != EC_FREE {
            if std::time::Instant::now() > deadline {
                self.outb(self.base + EC_PAGE, EC_FREE);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Temperature at a register pair, in degrees Celsius.
    pub fn temperature(&self, register: u16) -> Option<f32> {
        let whole = self.read(register) as i8 as f32;
        let half = (self.read(register + 1) >> 7) as f32 * 0.5;
        let t = whole + half;
        (t > -55.0 && t < 125.0 && t != 0.0).then_some(t)
    }

    /// Fan speed in rpm at a register pair, 0 when stopped or absent.
    pub fn rpm(&self, register: u16) -> u32 {
        let high = self.read(register) as u32;
        let low = self.read(register + 1) as u32;
        if high == 0xFF && low == 0xF8 {
            return 0;
        }
        // The NCT6687D reports rpm directly as a 16-bit value.
        (high << 8) | low
    }
}

fn find(module: &Module) -> Result<u16, String> {
    let mut seen = Vec::new();
    for slot in [0u64, 1] {
        let port = if slot == 0 { 0x2E } else { 0x4E };
        module.execute("ioctl_select_slot", &[slot], 0)?;
        let out = |p: u64, v: u64| module.execute("ioctl_pio_outb", &[p, v], 0);
        out(port, 0x87)?;
        out(port, 0x87)?;
        let id = module
            .execute("ioctl_superio_inw", &[0x20], 1)?
            .first()
            .copied()
            .unwrap_or(0) as u16;
        seen.push(format!("{port:#x}: {id:#06x}"));
        if id == CHIP_ID {
            module.execute("ioctl_superio_outb", &[0x07, HWM_LDN], 0)?;
            let base = module
                .execute("ioctl_superio_inw", &[0x60], 1)?
                .first()
                .copied()
                .unwrap_or(0) as u16;
            module.execute("ioctl_find_bars", &[], 0)?;
            out(port, 0xAA)?;
            if base == 0 || base == 0xFFFF {
                return Err(format!(
                    "NCT6687D has no hardware monitor address ({base:#x})"
                ));
            }
            return Ok(base & 0xFFF8);
        }
        out(port, 0xAA)?;
    }
    Err(format!("no NCT6687D found ({})", seen.join(", ")))
}

const MODE: u16 = 0xA00;
const REQUEST: u16 = 0xA01;
const COMMAND: u16 = 0xA28;
const CFG_REQUEST: u8 = 0x80;
const CFG_COMPLETE: u8 = 0x40;

impl Chip {
    /// The duty the board itself is using on a header, 0 to 255.
    pub fn duty(&self, header: u8) -> u8 {
        self.read(0x160 + header as u16)
    }

    /// Whether FoxRGB has a header in manual mode.
    pub fn is_manual(&self, header: u8) -> bool {
        self.read(MODE) & (1 << header) != 0
    }

    /// Takes a header into manual mode at `duty` (0 to 255), as LibreHardwareMonitor does.
    pub fn set(&self, header: u8, duty: u8) {
        self.write(REQUEST, CFG_REQUEST);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mode = self.read(MODE) | (1 << header);
        self.write(MODE, mode);
        self.write(COMMAND + header as u16, duty);
        self.write(REQUEST, CFG_COMPLETE);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    /// Hands a header back to the board's own fan curve, restoring the command it had.
    pub fn release(&self, header: u8, command: u8) {
        self.write(REQUEST, CFG_REQUEST);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mode = self.read(MODE) & !(1 << header);
        self.write(MODE, mode);
        self.write(COMMAND + header as u16, command);
        self.write(REQUEST, CFG_COMPLETE);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    pub fn command(&self, header: u8) -> u8 {
        self.read(COMMAND + header as u16)
    }
}
