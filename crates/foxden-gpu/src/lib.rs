//! Graphics card lighting over the card's own I2C bus, through the AMD and NVIDIA drivers.

mod amd;
mod families;
mod illumination;
mod nvidia;

use foxden_core::Backend;
use foxden_smbus::i2c::SharedBus;

/// The I2C buses of every graphics card the drivers expose.
pub fn buses() -> Vec<SharedBus> {
    let mut buses = amd::buses();
    buses.extend(nvidia::buses());
    buses
}

/// Every graphics card lighting device found.
pub fn probe() -> Vec<Box<dyn Backend>> {
    let mut found = Vec::new();
    for bus in buses() {
        for detect in families::all() {
            found.extend(detect(&bus));
        }
    }
    found.extend(illumination::probe());
    found
}

/// Each card's PCI ids and the I2C addresses that answer on its bus, for device reports.
pub fn report() -> Vec<String> {
    let mut out = Vec::new();
    for bus in buses() {
        let b = foxden_smbus::i2c::lock(&bus);
        let i = b.info().clone();
        out.push(format!(
            "{} {:04x}:{:04x} sub {:04x}:{:04x}",
            i.name, i.pci_vendor, i.pci_device, i.pci_subsystem_vendor, i.pci_subsystem_device
        ));
        // Display data and memory addresses are left alone.
        let answers: Vec<String> = (0x08..0x78u8)
            .filter(|a| !(0x30..0x38).contains(a) && !(0x50..0x60).contains(a))
            .filter(|&a| b.read_byte(a).is_ok())
            .map(|a| format!("{a:02x}"))
            .collect();
        out.push(format!("  answers at {}", answers.join(" ")));
    }
    out
}

/// Whether a driver here lists the card with these PCI ids: vendor, device, subsystem vendor and
/// subsystem device. Touches no hardware.
pub fn known(ids: [u16; 4]) -> bool {
    families::known(ids) || illumination::known(ids)
}
