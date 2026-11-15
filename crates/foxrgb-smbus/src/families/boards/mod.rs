//! Ported from OpenRGB pci_ids/pci_ids.h and the SMBus motherboard detectors (GPL-2.0-or-later).

use std::sync::Arc;
use std::time::Duration;

use foxrgb_core::{Backend, Color, Device};

use crate::i2c::{BusInfo, Detect, SharedBus, lock};

mod asrock;
mod dmi;
mod ene;
mod ene_gpus;
mod evga;
#[cfg(test)]
mod fake;
mod fusion;
mod fusion2;

pub fn all() -> Vec<Detect> {
    vec![
        ene::detect_dram,
        ene::detect_board,
        asrock::detect,
        fusion2::detect,
        fusion::detect,
        evga::detect,
    ]
}

/// Families that live on a graphics card's own I2C bus.
pub fn gpu() -> Vec<Detect> {
    vec![ene::detect_gpu]
}

pub fn gpu_cards() -> Vec<[u16; 4]> {
    ene_gpus::ENE_GPUS
        .iter()
        .map(|g| [g.1, g.2, g.3, g.4])
        .collect()
}

const AMD_VEN: u16 = 0x1022;
const INTEL_VEN: u16 = 0x8086;
const AMD_FCH_SMBUS_DEV: u16 = 0x790B;
const INTEL_SMBUS_DEVS: [u16; 11] = [
    0x3A30, 0xA123, 0x2085, 0xA2A3, 0xA323, 0x06A3, 0xA3A3, 0x43A3, 0x7AA3, 0x7A23, 0x7F23,
];

pub(crate) const ASUS_SUB_VEN: u16 = 0x1043;
pub(crate) const ASROCK_SUB_VEN: u16 = 0x1849;
pub(crate) const EVGA_SUB_VEN: u16 = 0x3842;
pub(crate) const GIGABYTE_SUB_VEN: u16 = 0x1458;

/// IF_DRAM_SMBUS.
fn dram_bus(info: &BusInfo) -> bool {
    (info.pci_vendor == AMD_VEN && info.pci_device == AMD_FCH_SMBUS_DEV)
        || (info.pci_vendor == INTEL_VEN && INTEL_SMBUS_DEVS.contains(&info.pci_device))
}

/// IF_MOBO_SMBUS.
fn mobo_bus(info: &BusInfo) -> bool {
    info.pci_vendor == 0
        || (info.pci_vendor == 0xFFFF && info.pci_device == 0xFFFF)
        || dram_bus(info)
}

pub(crate) type Sleep = Arc<dyn Fn(u64) + Send + Sync>;

/// The bus plus the sleep OpenRGB does between transactions; tests swap the sleep for a logger.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub bus: SharedBus,
    pub sleep: Sleep,
}

impl Ctx {
    fn real(bus: &SharedBus) -> Self {
        Ctx {
            bus: bus.clone(),
            sleep: Arc::new(|ms| std::thread::sleep(Duration::from_millis(ms))),
        }
    }

    pub fn info(&self) -> BusInfo {
        lock(&self.bus).info().clone()
    }

    pub fn ms(&self, ms: u64) {
        (self.sleep)(ms)
    }
}

/// One family's protocol: `first` is the push that takes software control.
pub(crate) trait Driver: Send {
    fn write(&mut self, colors: &[Color], first: bool) -> Result<(), String>;
}

/// The Backend every board family shares: id check, first-push setup, unchanged frames skipped.
pub(crate) struct Board<D> {
    name: &'static str,
    device: Device,
    driver: D,
    started: bool,
    sent: Option<Vec<Color>>,
}

impl<D: Driver + 'static> Board<D> {
    pub fn boxed(name: &'static str, device: Device, driver: D) -> Box<dyn Backend> {
        Box::new(Board {
            name,
            device,
            driver,
            started: false,
            sent: None,
        })
    }
}

impl<D: Driver> Backend for Board<D> {
    fn name(&self) -> &str {
        self.name
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let mut colors = colors.to_vec();
        colors.resize(self.device.leds.len(), Color::BLACK);
        if self.sent.as_ref() == Some(&colors) {
            return Ok(());
        }
        self.driver.write(&colors, !self.started)?;
        self.started = true;
        self.sent = Some(colors);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // IF_DRAM_SMBUS and IF_MOBO_SMBUS.
    #[test]
    fn pci_gates_match_the_macros() {
        let bus = |pci_vendor, pci_device| BusInfo {
            pci_vendor,
            pci_device,
            ..Default::default()
        };
        assert!(dram_bus(&bus(0x1022, 0x790B)));
        assert!(dram_bus(&bus(0x8086, 0x7A23)));
        assert!(!dram_bus(&bus(0x8086, 0x1234)));
        assert!(!dram_bus(&bus(0, 0)));
        assert!(mobo_bus(&bus(0, 0)));
        assert!(mobo_bus(&bus(0xFFFF, 0xFFFF)));
        assert!(!mobo_bus(&bus(0xFFFF, 0x0000)));
        assert!(mobo_bus(&bus(0x8086, 0x3A30)));
        assert!(!mobo_bus(&bus(0x10DE, 0x2507)));
    }
}
