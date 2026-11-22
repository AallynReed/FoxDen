// SPDX-License-Identifier: GPL-2.0-or-later

use super::common::{Io, delay};

pub const RESERVED: u8 = 0;
pub const DDR4: u8 = 12;
pub const DDR4E: u8 = 14;
pub const LPDDR4: u8 = 16;
pub const LPDDR4X: u8 = 17;
pub const DDR5: u8 = 18;
pub const LPDDR5: u8 = 19;
pub const UNKNOWN: u8 = 20;

pub const JEDEC_KINGSTON: u16 = 0x0117;
pub const JEDEC_CORSAIR: u16 = 0x021D;
pub const JEDEC_TEAMGROUP: u16 = 0x046E;
pub const JEDEC_KINGSTON_2: u16 = 0x300F;
pub const JEDEC_KINGSTON_3: u16 = 0x3011;
pub const JEDEC_PATRIOT: u16 = 0x8501;

const IO_DELAY: u64 = 1;
const DDR5_VIRTUAL_PAGE: u8 = 0x0B;

fn clamp(t: u8) -> u8 {
    t.min(UNKNOWN)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ddr4,
    Ddr5,
}

/// DDR4DirectAccessor / DDR5DirectAccessor, each caching the page it last selected.
struct Accessor {
    kind: Kind,
    addr: u8,
    page: u8,
}

impl Accessor {
    fn new(kind: Kind, addr: u8) -> Self {
        Accessor {
            kind,
            addr,
            page: 0xFF,
        }
    }

    fn set_page(&mut self, io: &Io, page: u8) {
        if self.page != page {
            match self.kind {
                Kind::Ddr4 => io.write_byte_data(0x36 + page, 0x00, 0xFF),
                Kind::Ddr5 => io.write_byte_data(self.addr, DDR5_VIRTUAL_PAGE, page),
            };
            self.page = page;
            delay(IO_DELAY);
        }
    }

    fn at(&mut self, io: &Io, addr: u16) -> u8 {
        let (len, shift, offset) = match self.kind {
            Kind::Ddr4 => (512, 8, (addr & 0xFF) as u8),
            Kind::Ddr5 => (2048, 7, (addr & 0x7F) as u8 | 0x80),
        };
        if addr >= len {
            return 0xFF;
        }
        self.set_page(io, (addr >> shift) as u8);
        let value = io.read_byte_data(self.addr, offset);
        delay(IO_DELAY);
        value as u8
    }

    fn memory_type(&mut self, io: &Io) -> u8 {
        clamp(self.at(io, 0x02))
    }

    fn jedec_id(&mut self, io: &Io) -> u16 {
        let a = match self.kind {
            Kind::Ddr4 => 0x140,
            Kind::Ddr5 => 0x200,
        };
        let hi = (self.at(io, a) as u16) << 8;
        let lo = (self.at(io, a + 1) & 0x7F) as u16;
        hi.wrapping_add(lo).wrapping_sub(1)
    }

    fn manufacturer_data(&mut self, io: &Io, index: u16) -> u8 {
        let (start, len) = match self.kind {
            Kind::Ddr4 => (0x161, 0x17D - 0x161 + 1),
            Kind::Ddr5 => (0x22B, 0x27F - 0x22B + 1),
        };
        if index > len - 1 {
            return 0;
        }
        self.at(io, start + index)
    }
}

/// DDR4DirectAccessor::isAvailable
fn ddr4_available(io: &Io, addr: u8) -> bool {
    io.write_byte(0x36, 0x00);
    delay(IO_DELAY);
    io.read_byte_data(addr, 0x00) == 0x23
}

/// DDR5DirectAccessor::isAvailable
fn ddr5_available(io: &Io, addr: u8) -> bool {
    let mut retry = true;
    loop {
        delay(IO_DELAY);
        let magic = io.read_byte_data(addr, 0x00);
        delay(IO_DELAY);
        let sensor = io.read_byte_data(addr, 0x01);
        delay(IO_DELAY);
        if magic < 0 || sensor < 0 {
            return false;
        }
        if magic == 0x51 && (sensor & 0xEF) == 0x08 {
            return true;
        }
        let page = io.read_byte_data(addr, DDR5_VIRTUAL_PAGE);
        delay(IO_DELAY);
        if retry && page > 0 && page < (2048 >> 7) {
            io.write_byte_data(addr, DDR5_VIRTUAL_PAGE, 0);
            delay(IO_DELAY);
            retry = false;
        } else {
            return false;
        }
    }
}

/// SPDDetector: the slot's memory type when it holds DDR4 or DDR5.
fn detect(io: &Io, addr: u8, mem_type: u8) -> Option<u8> {
    let ddr4ish = [RESERVED, DDR4, DDR4E, LPDDR4, LPDDR4X].contains(&mem_type);
    let ddr5ish = [RESERVED, DDR5, LPDDR5].contains(&mem_type);
    let mut acc = if ddr4ish && ddr4_available(io, addr) {
        Accessor::new(Kind::Ddr4, addr)
    } else if ddr5ish && ddr5_available(io, addr) {
        Accessor::new(Kind::Ddr5, addr)
    } else if mem_type == RESERVED {
        let value = io.read_byte_data(addr, 0x02);
        let ok =
            value >= 0 && [DDR4, DDR4E, LPDDR4, LPDDR4X, DDR5, LPDDR5].contains(&(value as u8));
        return ok.then_some(value as u8);
    } else {
        return None;
    };
    Some(clamp(acc.memory_type(io)))
}

/// SPDWrapper: one occupied slot, its JEDEC id read once and cached.
pub struct Slot {
    pub addr: u8,
    pub jedec: u16,
    acc: Option<Accessor>,
}

impl Slot {
    pub fn index(&self) -> u8 {
        self.addr - 0x50
    }

    pub fn manufacturer_data(&mut self, io: &Io, index: u16) -> u8 {
        match self.acc.as_mut() {
            Some(acc) => acc.manufacturer_data(io, index),
            None => 0,
        }
    }
}

/// DetectionManager::BackgroundDetectI2CDRAMDevices: slots 0x50..0x57 and the bus's memory type.
pub fn scan(io: &Io) -> (Vec<Slot>, u8) {
    let mut slots = Vec::new();
    let mut dram_type = RESERVED;
    for addr in 0x50..0x58 {
        let Some(t) = detect(io, addr, dram_type) else {
            continue;
        };
        let mut acc = match t {
            DDR4 => Some(Accessor::new(Kind::Ddr4, addr)),
            DDR5 => Some(Accessor::new(Kind::Ddr5, addr)),
            _ => None,
        };
        let jedec = acc.as_mut().map_or(0, |a| a.jedec_id(io));
        dram_type = t;
        slots.push(Slot { addr, jedec, acc });
    }
    (slots, dram_type)
}

#[cfg(test)]
pub mod tests {
    use super::super::fake::{Op::*, *};
    use super::*;

    /// The bus calls the scan makes on an empty slot after a DDR4 module was found.
    pub fn ddr4_empty(a: u8) -> Vec<Op> {
        vec![Wb(0x36, 0), Sleep(1), Rbd(a, 0)]
    }

    /// The bus calls that find a DDR4 module and read its JEDEC id.
    pub fn ddr4_found(a: u8) -> Vec<Op> {
        vec![
            Wb(0x36, 0),
            Sleep(1),
            Rbd(a, 0),
            Wbd(0x36, 0, 0xFF),
            Sleep(1),
            Rbd(a, 2),
            Sleep(1),
            Wbd(0x37, 0, 0xFF),
            Sleep(1),
            Rbd(a, 0x40),
            Sleep(1),
            Rbd(a, 0x41),
            Sleep(1),
        ]
    }

    /// A full scan with DDR4 modules in the given slots (and none before the first).
    pub fn ddr4_scan(used: &[u8]) -> Vec<Op> {
        let mut ops = Vec::new();
        let mut seen = false;
        for a in 0x50..0x58u8 {
            if used.contains(&a) {
                ops.extend(ddr4_found(a));
                seen = true;
            } else if seen {
                ops.extend(ddr4_empty(a));
            } else {
                ops.extend(reserved_empty(a));
            }
        }
        ops
    }

    fn reserved_empty(a: u8) -> Vec<Op> {
        vec![
            Wb(0x36, 0),
            Sleep(1),
            Rbd(a, 0),
            Sleep(1),
            Rbd(a, 0),
            Sleep(1),
            Rbd(a, 1),
            Sleep(1),
            Rbd(a, 2),
        ]
    }

    // SPDDetector + SPDWrapper on two DDR4 Kingston modules in slots 1 and 3
    #[test]
    fn ddr4_scan_reads_type_and_jedec() {
        let mut sim = Sim::default();
        sim.ddr4.insert(0x51, ddr4_spd([0x01, 0x98], &[]));
        sim.ddr4.insert(0x53, ddr4_spd([0x01, 0x98], &[]));
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (slots, t) = scan(&io);
        assert_eq!(t, DDR4);
        assert_eq!(slots.len(), 2);
        assert_eq!((slots[0].addr, slots[0].jedec), (0x51, JEDEC_KINGSTON));
        assert_eq!(slots[1].index(), 3);
        assert_eq!(take(), ddr4_scan(&[0x51, 0x53]));
    }

    // DDR5DirectAccessor::isAvailable, memory_type and jedec_id on one module
    #[test]
    fn ddr5_scan_pages_through_the_hub() {
        let mut sim = Sim::default();
        sim.ddr5.insert(0x52, ddr5_spd([0x30, 0x12], &[]));
        let bus = bus(sim);
        let io = Io::new(&bus);
        let (slots, t) = scan(&io);
        assert_eq!(t, DDR5);
        assert_eq!(slots[0].jedec, JEDEC_KINGSTON_3);
        let ops = take();
        let mut want = Vec::new();
        for a in [0x50, 0x51] {
            want.extend(reserved_empty(a));
        }
        want.extend([
            Wb(0x36, 0),
            Sleep(1),
            Rbd(0x52, 0),
            Sleep(1),
            Rbd(0x52, 0),
            Sleep(1),
            Rbd(0x52, 1),
            Sleep(1),
            Wbd(0x52, 0x0B, 0),
            Sleep(1),
            Rbd(0x52, 0x82),
            Sleep(1),
            Wbd(0x52, 0x0B, 4),
            Sleep(1),
            Rbd(0x52, 0x80),
            Sleep(1),
            Rbd(0x52, 0x81),
            Sleep(1),
        ]);
        for a in 0x53..0x58 {
            want.extend([Sleep(1), Rbd(a, 0), Sleep(1), Rbd(a, 1), Sleep(1)]);
        }
        assert_eq!(ops, want);
    }

    // DDR5DirectAccessor::isAvailable resets a hub left on another page, once
    #[test]
    fn ddr5_hub_on_wrong_page_is_reset() {
        let sim = Sim {
            hook: Some(Box::new(|op| match op {
                Rbd(0x50, 0) => Some(Ok(0x51)),
                Rbd(0x50, 1) => Some(Ok(0x00)),
                Rbd(0x50, 0x0B) => Some(Ok(3)),
                Rbd(0x50, _) | Wbd(0x50, ..) | Wb(..) => Some(Ok(0)),
                _ => Some(Err("nack".into())),
            })),
            ..Default::default()
        };
        let bus = bus(sim);
        let io = Io::new(&bus);
        assert!(!ddr5_available(&io, 0x50));
        assert_eq!(
            take(),
            vec![
                Sleep(1),
                Rbd(0x50, 0),
                Sleep(1),
                Rbd(0x50, 1),
                Sleep(1),
                Rbd(0x50, 0x0B),
                Sleep(1),
                Wbd(0x50, 0x0B, 0),
                Sleep(1),
                Sleep(1),
                Rbd(0x50, 0),
                Sleep(1),
                Rbd(0x50, 1),
                Sleep(1),
                Rbd(0x50, 0x0B),
                Sleep(1),
            ]
        );
    }

    // DDR4Accessor::jedec_id wraps: a blank SPD reads 0xFF7E, which Patriot Viper Steel registers
    #[test]
    fn blank_ddr4_jedec_wraps() {
        let mut sim = Sim::default();
        let mut e = ddr4_spd([0xFF, 0xFF], &[]);
        e[0x140] = 0xFF;
        sim.ddr4.insert(0x50, e);
        let bus = bus(sim);
        let (slots, _) = scan(&Io::new(&bus));
        assert_eq!(slots[0].jedec, 0xFF7E);
        let mut sim = Sim::default();
        sim.ddr4.insert(0x50, ddr4_spd([0x00, 0x00], &[]));
        let bus = super::super::fake::bus(sim);
        let (slots, _) = scan(&Io::new(&bus));
        assert_eq!(slots[0].jedec, 0xFFFF);
        take();
    }
}
