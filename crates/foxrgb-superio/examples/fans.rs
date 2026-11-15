use foxrgb_superio::{cpu::CpuTemp, nct6687::Chip};

fn main() {
    match CpuTemp::open() {
        Ok(cpu) => println!("cpu package: {:?} C", cpu.read()),
        Err(e) => println!("cpu: {e}"),
    }
    match Chip::open() {
        Ok(chip) => {
            println!("NCT6687D at {:#x}", chip.base());
            for (i, reg) in (0x100u16..=0x114).step_by(2).enumerate() {
                println!("temp {i} ({reg:#05x}): {:?}", chip.temperature(reg));
            }
            for (i, reg) in (0x140u16..=0x15E).step_by(2).enumerate() {
                println!("fan {i} ({reg:#05x}): {} rpm", chip.rpm(reg));
            }
            for (i, reg) in (0x160u16..=0x167).enumerate() {
                println!("duty out {i} ({reg:#05x}): {}", chip.read(reg));
            }
            println!("mode 0xA00: {:#04x}", chip.read(0xA00));
            for (i, reg) in (0xA28u16..=0xA2F).enumerate() {
                println!("duty command {i} ({reg:#05x}): {}", chip.read(reg));
            }
        }
        Err(e) => println!("chip: {e}"),
    }

}
