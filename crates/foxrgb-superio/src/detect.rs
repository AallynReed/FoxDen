//! Finds Super I/O chips at 0x2E and 0x4E by chip id, writing nothing until one is recognised.

use crate::boards::Maker;
use crate::chip::{self, Board, Chip};
use crate::it87::{Controller, Firmware};
use crate::lights::Pawn;
use crate::port::Lpc;

const CHIP_ID: u8 = 0x20;
const CHIP_REV: u8 = 0x21;
const ITE_VERSION: u8 = 0x22;
const VENDOR_ID: u8 = 0x23;
const ACTIVATE: u8 = 0x30;
const BASE: u8 = 0x60;
const BASE_ALT: u8 = 0x64;
const FINTEK: u16 = 0x1934;
const ITE_MONITOR_LDN: u8 = 0x04;
const IT8705_GPIO_LDN: u8 = 0x05;
const ITE_GPIO_LDN: u8 = 0x07;
const ITE_SMFI_LDN: u8 = 0x0F;

pub(crate) struct Found {
    pub chip: Chip,
    pub lpc: Lpc,
    pub base: u16,
    pub version: u8,
    pub firmware: Firmware,
}

fn invalid(base: u16) -> bool {
    base < 0x100 || base & 0xF007 != 0
}

/// Reads a base address twice, a millisecond apart.
fn base_twice(lpc: &Lpc, reg: u8) -> (u16, u16) {
    let first = lpc.read_word(reg);
    lpc.sleep(1);
    (first, lpc.read_word(reg))
}

fn winbond_nuvoton_fintek(lpc: &Lpc, board: &Board) -> Option<(Chip, u16)> {
    lpc.nuvoton_enter();
    let id = lpc.read(CHIP_ID);
    let rev = lpc.read(CHIP_REV);
    let Some((chip, ldn)) = chip::winbond_family(id, rev, board) else {
        if id != 0 && id != 0xFF {
            lpc.nuvoton_exit();
        }
        return None;
    };
    lpc.find_bars();
    lpc.select(ldn);
    if chip == Chip::NCT6701D && lpc.read(ACTIVATE) == 0 {
        lpc.write(ACTIVATE, 0x01);
    }
    let (mut base, mut check) = base_twice(lpc, BASE);
    if chip == Chip::NCT6701D && base == check && invalid(base) {
        (base, check) = base_twice(lpc, BASE_ALT);
    }
    let vendor = lpc.read_word(VENDOR_ID);
    if base == check && chip.has_io_lock() && chip != Chip::NCT5585D {
        lpc.nuvoton_unlock();
    }
    lpc.nuvoton_exit();
    if base != check {
        return None;
    }
    if base & 0x07 == 0x05 {
        base &= 0xFFF8;
    }
    if invalid(base) || (chip.is_fintek() && vendor != FINTEK) {
        return None;
    }
    Some((chip, base))
}

fn ite(lpc: &Lpc, board: &Board) -> Option<(Chip, u16, u8, Firmware)> {
    // A second chip already in configuration mode answers at once and must not be entered again.
    let mut id = if lpc.port == 0x4E {
        lpc.read_word(CHIP_ID)
    } else {
        0xFFFF
    };
    if lpc.port != 0x4E || id == 0xFFFF {
        lpc.ite_enter();
        id = lpc.read_word(CHIP_ID);
    }
    let Some(chip) = chip::ite(id) else {
        if id != 0 && id != 0xFFFF {
            lpc.ite_exit();
        }
        return None;
    };
    lpc.find_bars();
    lpc.select(ITE_MONITOR_LDN);
    let (base, check) = base_twice(lpc, BASE);
    let version = lpc.read(ITE_VERSION) & 0x0F;
    let (gpio, gpio_check) = if chip == Chip::IT8705F {
        lpc.select(IT8705_GPIO_LDN);
        base_twice(lpc, BASE)
    } else {
        lpc.select(ITE_GPIO_LDN);
        base_twice(lpc, BASE + 2)
    };
    let firmware = gigabyte(lpc, chip, board);
    lpc.ite_exit();
    if base != check || invalid(base) || gpio != gpio_check || invalid(gpio) {
        return None;
    }
    Some((chip, base, version, firmware))
}

/// The second ITE chip on Gigabyte boards may run a fan controller of the board's own.
fn gigabyte(lpc: &Lpc, chip: Chip, board: &Board) -> Firmware {
    if board.maker != Maker::Gigabyte
        || lpc.port != 0x4E
        || !matches!(chip, Chip::IT8790E | Chip::IT8792E | Chip::IT87952E)
    {
        return Firmware::Absent;
    }
    lpc.select(ITE_SMFI_LDN);
    let first = lpc.read(ACTIVATE);
    lpc.sleep(1);
    let mapped = first == lpc.read(ACTIVATE) && first != 0;
    if chip == Chip::IT8792E
        && board.amd
        && let Some(controller) = Controller::find(lpc)
    {
        return Firmware::Pausable(controller);
    }
    // Reaching the controller through memory needs a PawnIO module FoxRGB does not ship.
    if mapped {
        Firmware::Unreachable
    } else {
        Firmware::Absent
    }
}

/// Every recognised chip, each with its own PawnIO instance on its port.
pub(crate) fn detect(open: &dyn Fn() -> Option<Box<dyn Pawn>>, board: &Board) -> Vec<Found> {
    let mut found = Vec::new();
    for port in [0x2E, 0x4E] {
        let Some(pawn) = open() else { continue };
        let lpc = Lpc::new(pawn, port);
        if let Some((chip, base)) = winbond_nuvoton_fintek(&lpc, board) {
            found.push(Found {
                chip,
                lpc,
                base,
                version: 0,
                firmware: Firmware::Absent,
            });
        } else if let Some((chip, base, version, firmware)) = ite(&lpc, board) {
            found.push(Found {
                chip,
                lpc,
                base,
                version,
                firmware,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{Bus, log};

    fn run(bus: &Bus, board: &Board) -> Vec<Found> {
        let b = bus.clone();
        detect(&move || Some(Box::new(b.clone()) as Box<dyn Pawn>), board)
    }

    #[test]
    fn nuvoton_found_unlocked_and_left() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0xD4)
            .config(0, 0x21, 0x2B)
            .config(0, 0x28, 0x10)
            .word(0x0B, 0x60, 0x0290)
            .word(0, 0x23, 0xFFFF);
        // this fake answers the same on both ports
        let found = run(&bus, &Board::default());
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].chip, found[0].base), (Chip::NCT6798D, 0x290));
        let want = log(&[
            "select_slot 00",
            "pio_outb 2e 87",
            "pio_outb 2e 87",
            "superio_inb 20",
            "superio_inb 21",
            "find_bars",
            "superio_outb 07 0b",
            "superio_inw 60",
            "superio_inw 60",
            "superio_inw 23",
            "superio_inb 28",
            "superio_outb 28 00",
            "pio_outb 2e aa",
        ]);
        let log = bus.take();
        assert_eq!(log[..13], want[..]);
        assert_eq!(log[13], "select_slot 01");
    }

    #[test]
    fn winbond_found_without_unlocking() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0xA0)
            .config(0, 0x21, 0x21)
            .config(0, 0x28, 0x10)
            .word(0x0B, 0x60, 0x0295);
        let found = run(&bus, &Board::default());
        assert_eq!((found[0].chip, found[0].base), (Chip::W83627DHG, 0x290));
        let want = log(&[
            "select_slot 00",
            "pio_outb 2e 87",
            "pio_outb 2e 87",
            "superio_inb 20",
            "superio_inb 21",
            "find_bars",
            "superio_outb 07 0b",
            "superio_inw 60",
            "superio_inw 60",
            "superio_inw 23",
            "pio_outb 2e aa",
            "select_slot 01",
        ]);
        assert_eq!(bus.take()[..12], want[..]);
    }

    #[test]
    fn fintek_needs_its_vendor_id() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0x07)
            .config(0, 0x21, 0x23)
            .word(0x04, 0x60, 0x0A05)
            .word(0, 0x23, 0x1934);
        let found = run(&bus, &Board::default());
        assert_eq!((found[0].chip, found[0].base), (Chip::F71889F, 0xA00));
        bus.word(0, 0x23, 0x1234);
        assert!(
            run(&bus, &Board::default())
                .iter()
                .all(|f| !f.chip.is_fintek())
        );
    }

    #[test]
    fn unknown_ids_are_left_alone() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0x12).config(0, 0x21, 0x34);
        assert!(run(&bus, &Board::default()).is_empty());
        let seen = bus.take();
        let first = log(&[
            "select_slot 00",
            "pio_outb 2e 87",
            "pio_outb 2e 87",
            "superio_inb 20",
            "superio_inb 21",
            "pio_outb 2e aa",
            "pio_outb 2e 87",
            "pio_outb 2e 01",
            "pio_outb 2e 55",
            "pio_outb 2e 55",
            "superio_inw 20",
            "pio_outb 2e 02",
            "pio_outb 2f 02",
        ]);
        assert_eq!(seen[..13], first[..]);
        assert!(
            !seen
                .iter()
                .any(|l| l.starts_with("superio_outb") || l == "find_bars")
        );
    }

    #[test]
    fn ite_on_both_ports() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0x86)
            .config(0, 0x21, 0x88)
            .config(0, 0x22, 0x13)
            .word(0x04, 0x60, 0x0A40)
            .word(0x07, 0x62, 0x0A00);
        let found = run(&bus, &Board::default());
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].chip, found[0].base, found[0].version),
            (Chip::IT8688E, 0xA40, 3)
        );
        let seen = bus.take();
        let first = log(&[
            "select_slot 00",
            "pio_outb 2e 87",
            "pio_outb 2e 87",
            "superio_inb 20",
            "superio_inb 21",
            "pio_outb 2e aa",
            "pio_outb 2e 87",
            "pio_outb 2e 01",
            "pio_outb 2e 55",
            "pio_outb 2e 55",
            "superio_inw 20",
            "find_bars",
            "superio_outb 07 04",
            "superio_inw 60",
            "superio_inw 60",
            "superio_inb 22",
            "superio_outb 07 07",
            "superio_inw 62",
            "superio_inw 62",
            "pio_outb 2e 02",
            "pio_outb 2f 02",
        ]);
        assert_eq!(seen[..21], first[..]);
        // the second chip answers before being entered, so it is not entered again
        let second = &seen[21..];
        let at = second.iter().position(|l| l == "superio_inw 20").unwrap();
        assert_eq!(second[at + 1], "find_bars");
        assert!(!second.iter().any(|l| l == "pio_outb 4e 02"));
    }

    #[test]
    fn board_tells_apart_shared_ids() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0xD5)
            .config(0, 0x21, 0x92)
            .word(0x0B, 0x60, 0x0A20);
        let msi = Board {
            maker: Maker::MSI,
            product: "MAG X870E TOMAHAWK WIFI (MS-7E59)".into(),
            ..Board::default()
        };
        assert_eq!(run(&bus, &msi)[0].chip, Chip::NCT6687DR);
        assert_eq!(run(&bus, &Board::default())[0].chip, Chip::NCT6687D);
        let nova = Board {
            model: Some("X870E_NOVA_WIFI"),
            ..Board::default()
        };
        bus.config(0, 0x20, 0xD4).config(0, 0x21, 0x2A);
        assert_eq!(run(&bus, &nova)[0].chip, Chip::NCT5585D);
        assert_eq!(run(&bus, &Board::default())[0].chip, Chip::NCT6796DR);
    }

    #[test]
    fn bad_base_addresses_are_refused() {
        let bus = Bus::default();
        bus.config(0, 0x20, 0xD4)
            .config(0, 0x21, 0x2B)
            .word(0x0B, 0x60, 0x0050);
        assert!(
            run(&bus, &Board::default())
                .iter()
                .all(|f| f.chip != Chip::NCT6798D)
        );
    }
}
