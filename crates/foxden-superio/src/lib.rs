//! Board fans and temperatures through the Super I/O chip, and the CPU temperature, via PawnIO.
//! Also board and case lighting reached the same way.

pub mod asus_ec;
mod board_names;
mod boards;
mod chip;
pub mod cpu;
mod detect;
mod f718xx;
mod gigabyte_sio;
mod it87;
mod lights;
mod monitor;
mod msi_rgb;
pub mod nct6687;
mod nct677x;
mod port;
#[cfg(test)]
mod testbus;
mod valve;
mod w836xx;

pub use lights::lights;

use std::sync::Arc;

use foxden_fan::{Fan, FanBackend, Sensor};
use foxden_pawnio::{LPC_IO, Module, SharedLock};

use crate::chip::Chip;
use crate::detect::Found;
use crate::lights::Pawn;
use crate::monitor::{Layout, Monitor, PUMP_FLOOR};
use crate::nct6687::ISA_MUTEX;

/// Header names on MSI boards with the NCT6687D, as other fan tools show them.
const HEADERS: [&str; 8] = [
    "CPU fan",
    "Pump",
    "System fan 1",
    "System fan 2",
    "System fan 3",
    "System fan 4",
    "System fan 5",
    "System fan 6",
];
const PUMP: u8 = 1;
const BOARD_TEMPS: [(&str, &str, u16); 7] = [
    ("board/cpu", "CPU (board)", 0x100),
    ("board/system", "System", 0x102),
    ("board/mos", "VRM", 0x104),
    ("board/pch", "Chipset", 0x106),
    ("board/socket", "CPU socket", 0x108),
    ("board/pcie", "PCIe", 0x10A),
    ("board/m2", "M.2", 0x10C),
];

/// The board's fans and temperatures and the CPU temperature, where PawnIO and the chips allow.
pub fn backends() -> Vec<Box<dyn FanBackend>> {
    let mut found: Vec<Box<dyn FanBackend>> = Vec::new();
    if let Ok(cpu) = cpu::CpuTemp::open() {
        found.push(Box::new(CpuBackend(Arc::new(cpu))));
    }
    found.extend(boards());
    found
}

/// Every Super I/O chip on the board that is positively identified, as a fan backend.
fn boards() -> Vec<Box<dyn FanBackend>> {
    let dmi = lights::dmi();
    let board = chip::Board {
        maker: boards::maker(&dmi.manufacturer),
        model: boards::model(&dmi.mainboard),
        product: dmi.mainboard,
        amd: lights::cpu_is_amd(),
    };
    let open = || {
        Module::open(LPC_IO)
            .ok()
            .map(|m| Box::new(m) as Box<dyn Pawn>)
    };
    let lock = SharedLock::new(ISA_MUTEX);
    let Ok(found) = lock.run(|| detect::detect(&open, &board)) else {
        return Vec::new();
    };
    let mut seen: Vec<Chip> = Vec::new();
    let mut backends = Vec::new();
    for f in found {
        let index = seen.iter().filter(|&&c| c == f.chip).count();
        seen.push(f.chip);
        backends.extend(lock.run(|| backend(f, &board, index)).ok().flatten());
    }
    backends
}

fn backend(f: Found, board: &chip::Board, index: usize) -> Option<Box<dyn FanBackend>> {
    let Found {
        chip,
        lpc,
        base,
        version,
        firmware,
    } = f;
    let port = lpc.port;
    let monitor: Box<dyn Monitor> = match chip {
        Chip::NCT6687D => {
            let chip = nct6687::Chip::new(lpc, base, Some(SharedLock::new(ISA_MUTEX)));
            return Some(Box::new(BoardBackend(Arc::new(chip))));
        }
        c if c.is_paged() => Box::new(nct6687::Paged::new(lpc, c, base)),
        c if c.is_ite() => Box::new(it87::It87::new(lpc, c, base, version, firmware)?),
        c if c.is_fintek() => Box::new(f718xx::F718xx::new(lpc, c, base)),
        c if c.is_winbond() => Box::new(w836xx::W836xx::new(lpc, c, base)?),
        c => Box::new(nct677x::Nct677x::new(lpc, c, base)?),
    };
    let layout = boards::layout(chip, board.maker, board.model, index)
        .unwrap_or_else(|| Layout::generic(monitor.size()));
    let name = chip.name();
    Some(Box::new(monitor::Board::new(
        name.clone(),
        format!("board/{name}-{port:x}"),
        monitor,
        layout,
        Some(SharedLock::new(ISA_MUTEX)),
    )))
}

struct CpuBackend(Arc<cpu::CpuTemp>);

struct CpuSensor(Arc<cpu::CpuTemp>);

impl Sensor for CpuSensor {
    fn id(&self) -> &str {
        "cpu/package"
    }
    fn name(&self) -> &str {
        "CPU"
    }
    fn read(&mut self) -> Option<f32> {
        self.0.read()
    }
}

impl FanBackend for CpuBackend {
    fn name(&self) -> &str {
        "intel cpu"
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        vec![Box::new(CpuSensor(self.0.clone()))]
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        Vec::new()
    }
}

struct BoardBackend(Arc<nct6687::Chip>);

struct BoardSensor {
    chip: Arc<nct6687::Chip>,
    id: &'static str,
    name: &'static str,
    register: u16,
}

impl Sensor for BoardSensor {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        self.name
    }
    fn read(&mut self) -> Option<f32> {
        self.chip.temperature(self.register)
    }
}

struct Header {
    chip: Arc<nct6687::Chip>,
    index: u8,
    id: String,
    /// The board's own command, put back on release. `None` until FoxDen has taken the header.
    saved: Option<u8>,
}

impl Fan for Header {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        HEADERS[self.index as usize]
    }
    fn rpm(&mut self) -> Option<u32> {
        Some(self.chip.rpm(0x140 + self.index as u16 * 2)).filter(|&rpm| rpm > 0)
    }
    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        let duty = if self.index == PUMP {
            duty.max(PUMP_FLOOR)
        } else {
            duty
        };
        if self.saved.is_none() {
            self.saved = Some(self.chip.command(self.index)?);
        }
        self.chip
            .set(self.index, (duty.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
    fn release(&mut self) -> Result<(), String> {
        if let Some(command) = self.saved {
            self.chip.release(self.index, command)?;
            self.saved = None;
        }
        Ok(())
    }
}

impl FanBackend for BoardBackend {
    fn name(&self) -> &str {
        "nct6687d"
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        BOARD_TEMPS
            .iter()
            .filter(|(.., register)| self.0.temperature(*register).is_some())
            .map(|&(id, name, register)| {
                Box::new(BoardSensor {
                    chip: self.0.clone(),
                    id,
                    name,
                    register,
                }) as Box<dyn Sensor>
            })
            .collect()
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        (0..8u8)
            .map(|index| {
                Box::new(Header {
                    chip: self.0.clone(),
                    index,
                    id: format!("board/fan{index}"),
                    saved: None,
                }) as Box<dyn Fan>
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::it87::Firmware;
    use crate::testbus::{Bus, Flat, regs};

    fn found(bus: &Bus, chip: Chip, base: u16) -> Found {
        Found {
            chip,
            lpc: bus.lpc(0x2E),
            base,
            version: 0,
            firmware: Firmware::Absent,
        }
    }

    #[test]
    fn nct6687d_keeps_its_ids_and_others_carry_chip_and_port() {
        let bus = Bus::default();
        let mut old = backend(
            found(&bus, Chip::NCT6687D, 0xA20),
            &chip::Board::default(),
            0,
        )
        .unwrap();
        let fans: Vec<String> = old.fans().iter().map(|f| f.id().to_string()).collect();
        assert_eq!(fans[0], "board/fan0");
        assert_eq!(fans.len(), 8);
        assert_eq!(old.sensors()[0].id(), "board/cpu");

        let values = regs(&[(0x58, 0x90), (0x00, 0x10), (0x0C, 0x30)]);
        bus.device(0xA40, 0xA47, Flat::new(0xA40, &values));
        let mut ite = backend(
            found(&bus, Chip::IT8688E, 0xA40),
            &chip::Board::default(),
            0,
        )
        .unwrap();
        assert_eq!(ite.fans()[0].id(), "board/it8688e-2e/fan0");
        assert_eq!(ite.sensors()[0].id(), "board/it8688e-2e/temp0");
        let gone = regs(&[]);
        bus.device(0xB40, 0xB47, Flat::new(0xB40, &gone));
        assert!(
            backend(
                found(&bus, Chip::IT8688E, 0xB40),
                &chip::Board::default(),
                0
            )
            .is_none()
        );
    }
}
