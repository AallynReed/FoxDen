//! Lighting on the SMBus (memory modules, motherboards), through the PawnIO driver.
//! Needs administrator rights, or the installed service.

mod corsair;
mod families;
pub mod i2c;
mod pawnio;

use std::sync::{Arc, Mutex};

use foxden_core::Backend;

pub use pawnio::Bus;

/// Every SMBus lighting device found on every bus, or the reason no bus could be opened.
pub fn probe() -> Result<Vec<Box<dyn Backend>>, String> {
    let mut found = Vec::new();
    // Corsair memory talks to the bus directly; the other families go through the common interface.
    for (bus, own) in Bus::open_all()?.into_iter().zip(Bus::open_all()?) {
        found.extend(corsair::probe(Arc::new(Mutex::new(own))));
        let shared: i2c::SharedBus = Arc::new(Mutex::new(Box::new(bus) as Box<dyn i2c::SmBus>));
        for detect in families::all() {
            found.extend(detect(&shared));
        }
    }
    Ok(found)
}

/// Addresses the report reads, without 0x30-0x37 where SPD takes any access as a command.
fn scanned() -> impl Iterator<Item = u8> {
    (0x08..0x78u8).filter(|a| !(0x30..=0x37).contains(a))
}

/// What each SMBus answers, read only: which addresses reply, and the ID registers where boards
/// and memory keep their lighting chips. For device reports.
pub fn report() -> Vec<String> {
    use i2c::SmBus;
    let (buses, why) = Bus::open_each();
    let hex = |v: &[u8]| {
        v.iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut out: Vec<String> = why
        .into_iter()
        .map(|w| format!("not opened: {w}"))
        .collect();
    for bus in buses {
        let i = bus.info().clone();
        let alive: Vec<u8> = scanned()
            .filter(|&a| bus.read_byte_data(a, 0).is_ok())
            .collect();
        out.push(format!(
            "{} {:04x}:{:04x} sub {:04x}:{:04x}, answers at {}",
            i.name,
            i.pci_vendor,
            i.pci_device,
            i.pci_subsystem_vendor,
            i.pci_subsystem_device,
            if alive.is_empty() {
                "none".into()
            } else {
                hex(&alive)
            }
        ));
        let shared: i2c::SharedBus = Arc::new(Mutex::new(Box::new(bus) as Box<dyn i2c::SmBus>));
        for &a in alive
            .iter()
            .filter(|a| matches!(a, 0x40 | 0x4E | 0x4F | 0x70..=0x77))
        {
            let regs: Vec<u8> = (0xA0..0xB0u8)
                .map(|r| i2c::lock(&shared).read_byte_data(a, r).unwrap_or(0xFF))
                .collect();
            out.push(format!("  {a:02x} a0-af: {}", hex(&regs)));
            if regs.iter().enumerate().all(|(i, &v)| v == i as u8) {
                out.push(format!(
                    "  {a:02x} {}",
                    families::boards::ene::describe(&shared, a)
                ));
            }
        }
    }
    out
}

/// The SMBus families found on graphics cards, for the GPU crate to run on each card's bus.
pub fn gpu_families() -> Vec<i2c::Detect> {
    families::gpu()
}

/// PCI ids (vendor, device, subsystem vendor, subsystem device) of every card those families list.
pub fn gpu_cards() -> Vec<[u16; 4]> {
    families::gpu_cards()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_report_leaves_the_spd_control_addresses_alone() {
        let scanned: Vec<u8> = super::scanned().collect();
        assert!(scanned.iter().all(|a| !(0x30..=0x37).contains(a)));
        assert!(scanned.contains(&0x2F) && scanned.contains(&0x38) && scanned.contains(&0x50));
        assert_eq!(scanned.len(), 0x70 - 8);
    }
}
