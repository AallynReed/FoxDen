//! Board fans and temperatures through the Super I/O chip, and the CPU temperature, via PawnIO.

pub mod cpu;
pub mod nct6687;

use std::sync::Arc;

use foxrgb_fan::{Fan, FanBackend, Sensor};

/// Header names on MSI boards with the NCT6687D, as LibreHardwareMonitor and Fan Control show them.
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
/// The pump never goes below this, whatever a curve asks.
const PUMP_FLOOR: f32 = 0.7;
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
    if let Ok(chip) = nct6687::Chip::open() {
        found.push(Box::new(BoardBackend(Arc::new(chip))));
    }
    found
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
    /// The board's own command, put back on release. `None` until FoxRGB has taken the header.
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
            self.saved = Some(self.chip.command(self.index));
        }
        self.chip
            .set(self.index, (duty.clamp(0.0, 1.0) * 255.0).round() as u8);
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        if let Some(command) = self.saved.take() {
            self.chip.release(self.index, command);
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
