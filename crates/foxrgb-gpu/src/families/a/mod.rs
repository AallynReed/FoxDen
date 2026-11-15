//! Ported from OpenRGB (GPL-2.0-or-later).

use foxrgb_smbus::i2c::{BusInfo, Detect};

mod asus_aura;
#[cfg(test)]
mod fake;
mod gigabyte_fusion;

/// A REGISTER_I2C_PCI_DETECTOR line: name, vendor, device, subsystem vendor, subsystem device, address.
type Entry = (&'static str, u16, u16, u16, u16, u8);

/// Where every graphics card sits on the canvas.
const RECT: [f32; 4] = [0.06, 0.30, 0.40, 0.05];

/// The four-id comparison DetectionManager::BackgroundDetectI2CPCIDevices makes.
fn matching<'a>(info: &'a BusInfo, table: &'a [Entry]) -> impl Iterator<Item = &'a Entry> {
    table.iter().filter(|&&(_, ven, dev, sub_ven, sub_dev, _)| {
        info.pci_vendor == ven
            && info.pci_device == dev
            && info.pci_subsystem_vendor == sub_ven
            && info.pci_subsystem_device == sub_dev
    })
}

/// Whether any table here has a card with these PCI ids.
pub fn known(ids: [u16; 4]) -> bool {
    [asus_aura::DEVICES, gigabyte_fusion::DEVICES]
        .iter()
        .flat_map(|t| t.iter())
        .any(|&(_, ven, dev, sub_ven, sub_dev, _)| [ven, dev, sub_ven, sub_dev] == ids)
}

/// Every card listed, with the driver file it belongs to.
#[cfg(test)]
pub fn cards() -> Vec<(&'static str, [u16; 4])> {
    [
        ("asus_aura", asus_aura::DEVICES),
        ("gigabyte_fusion", gigabyte_fusion::DEVICES),
    ]
    .iter()
    .flat_map(|&(name, t)| t.iter().map(move |e| (name, [e.1, e.2, e.3, e.4])))
    .collect()
}

pub fn all() -> Vec<Detect> {
    vec![asus_aura::detect, gigabyte_fusion::detect]
}
