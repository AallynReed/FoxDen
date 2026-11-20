//! Temperatures and fan speeds from the ACPI embedded controller on ASUS boards. Read only.

use foxden_fan::FanBackend;
use foxden_pawnio::{Module, SharedLock};

use crate::lights::Pawn;
use crate::monitor::{Board, Layout, Monitor, Values};

const COMMAND: u64 = 0x66;
const DATA: u64 = 0x62;
const READ: u64 = 0x80;
const WRITE: u64 = 0x81;
const OUTPUT_FULL: u8 = 0x01;
const INPUT_FULL: u8 = 0x02;
const BANK: u8 = 0xFF;
const EC_MUTEX: &str = r"Global\Access_EC";
/// Sensors that read this have nothing plugged in.
const UNPLUGGED: i32 = -40;

#[derive(Clone, Copy, PartialEq)]
enum Family {
    Amd400,
    Amd500,
    Amd600,
    Amd800,
    Intel100,
    Intel300,
    Intel370,
    Intel400,
    Intel600,
    Intel700,
    Intel800,
}

#[derive(Clone, Copy, PartialEq)]
enum Input {
    Chipset,
    Cpu,
    CpuPackage,
    Board,
    TSensor,
    TSensorAlt,
    TSensor2,
    Vrm,
    VrmSoc,
    Dimm,
    WaterIn,
    WaterOut,
    WaterBlockIn,
    CpuOptFan,
    VrmFan,
    ChipsetFan,
    PumpFan,
}

use Family::*;
use Input::*;

/// Where an input lives on a family: register, and whether it is a two-byte fan speed.
fn place(family: Family, input: Input) -> Option<u16> {
    let old_amd = matches!(family, Amd400 | Amd500);
    let classic = old_amd || matches!(family, Intel100 | Intel300 | Intel370 | Intel400 | Intel600);
    Some(match input {
        Chipset if classic => 0x3A,
        Cpu if old_amd => 0x3B,
        Board if old_amd => 0x3C,
        TSensor if classic => 0x3D,
        Vrm if classic => 0x3E,
        Cpu if family == Amd800 => 0x30,
        CpuPackage if family == Amd800 => 0x31,
        Board if family == Amd800 => 0x32,
        Vrm if matches!(family, Amd800 | Intel700 | Intel800) => 0x33,
        Dimm if family == Amd800 => 0x55,
        VrmSoc if family == Amd800 => 0x418,
        TSensor if family == Amd800 => 0x36,
        TSensorAlt if family == Amd800 => 0x35,
        TSensor if family == Intel700 => 0x109,
        TSensor2 if family == Intel700 => 0x105,
        TSensor if family == Intel800 => 0x10C,
        WaterIn if family == Amd400 => 0x10D,
        WaterOut if family == Amd400 => 0x10B,
        WaterIn => 0x100,
        WaterOut => 0x101,
        WaterBlockIn if family == Intel600 => 0x102,
        CpuOptFan if matches!(family, Amd400 | Intel370) => 0xBC,
        CpuOptFan => 0xB0,
        VrmFan if old_amd => 0xB2,
        ChipsetFan if family == Amd500 => 0xB4,
        PumpFan if family == Intel100 => 0xBC,
        PumpFan if family == Intel370 => 0xBE,
        _ => return None,
    })
}

fn label(input: Input) -> &'static str {
    match input {
        Chipset => "Chipset",
        Cpu => "CPU",
        CpuPackage => "CPU package",
        Board => "Motherboard",
        TSensor | TSensorAlt => "T sensor",
        TSensor2 => "T sensor 2",
        Vrm => "VRM",
        VrmSoc => "VRM SOC",
        Dimm => "DIMM thermistor",
        WaterIn => "Water in",
        WaterOut => "Water out",
        WaterBlockIn => "Water block in",
        CpuOptFan => "CPU optional fan",
        VrmFan => "VRM heatsink fan",
        ChipsetFan => "Chipset fan",
        PumpFan => "Water pump",
    }
}

fn is_fan(input: Input) -> bool {
    matches!(input, CpuOptFan | VrmFan | ChipsetFan | PumpFan)
}

/// The boards whose EC sensors are known, with what each one wires up.
fn board(model: &str) -> Option<(Family, &'static [Input])> {
    let x870_full: &[Input] = &[Cpu, CpuPackage, Board, Vrm, TSensor, CpuOptFan];
    let c8: &[Input] = &[
        Chipset, Cpu, Board, TSensor, Vrm, WaterIn, WaterOut, CpuOptFan, ChipsetFan,
    ];
    let water: &[Input] = &[WaterIn, WaterOut, CpuOptFan];
    let z390: &[Input] = &[Vrm, Chipset, TSensor, CpuOptFan];
    let maximus_xi: &[Input] = &[Vrm, Chipset, WaterIn, WaterOut, TSensor, CpuOptFan];
    let tsensor_vrm: &[Input] = &[TSensor, Vrm];
    Some(match model {
        "ROG_STRIX_B850_E_GAMING_WIFI" => (Amd800, &[CpuPackage, Vrm, TSensorAlt]),
        "ROG_CROSSHAIR_X870E_APEX" => (Amd800, &[Vrm, VrmSoc, Dimm, CpuOptFan]),
        "ROG_CROSSHAIR_X870E_DARK_HERO" => (Amd800, &[TSensor]),
        "ROG_CROSSHAIR_X870E_HERO_BTF"
        | "ROG_CROSSHAIR_X870E_HERO"
        | "ROG_STRIX_X870E_E_GAMING_WIFI"
        | "ROG_STRIX_X870E_H_GAMING_WIFI7" => (Amd800, x870_full),
        "TUF_GAMING_X870_PLUS_WIFI" => (Amd800, &[Vrm, CpuOptFan]),
        "ROG_STRIX_X870_F_GAMING_WIFI" => (Amd800, &[Cpu, CpuPackage, Board, Vrm, TSensor]),
        "ROG_STRIX_X870_I_GAMING_WIFI" => (Amd800, &[Cpu, CpuPackage, Board, Vrm]),
        "PRIME_X470_PRO" => (Amd400, &[Chipset, Cpu, Board, Vrm, CpuOptFan]),
        "PRIME_X570_PRO" => (Amd500, &[Chipset, Cpu, Board, Vrm, TSensor, ChipsetFan]),
        "PROART_X570_CREATOR_WIFI" => (Amd500, &[Chipset, Cpu, Board, Vrm, TSensor, CpuOptFan]),
        "PRO_WS_X570_ACE" => (Amd500, &[Chipset, Cpu, Board, Vrm, ChipsetFan]),
        "ROG_CROSSHAIR_VIII_HERO"
        | "ROG_CROSSHAIR_VIII_HERO_WIFI"
        | "ROG_CROSSHAIR_VIII_FORMULA" => (Amd500, c8),
        "ROG_CROSSHAIR_VIII_DARK_HERO" => (Amd500, &c8[..8]),
        "ROG_CROSSHAIR_VIII_IMPACT" | "ROG_STRIX_X570_E_GAMING" => {
            (Amd500, &[Chipset, Cpu, Board, TSensor, Vrm, ChipsetFan])
        }
        "ROG_STRIX_B550_E_GAMING" => (Amd500, &[Chipset, Cpu, Board, TSensor, Vrm, CpuOptFan]),
        "ROG_STRIX_B550_I_GAMING" => (Amd500, &[Chipset, Cpu, Board, TSensor, Vrm, VrmFan]),
        "ROG_STRIX_X570_E_GAMING_WIFI_II" => (Amd500, &[Chipset, Cpu, Board, TSensor, Vrm]),
        "ROG_STRIX_X570_F_GAMING" => (Amd500, &[Chipset, Cpu, Board, TSensor, ChipsetFan]),
        "ROG_STRIX_X570_I_GAMING" => (Amd500, &[TSensor, VrmFan, ChipsetFan, Chipset, Vrm]),
        "ROG_CROSSHAIR_X670E_EXTREME"
        | "ROG_CROSSHAIR_X670E_HERO"
        | "ROG_CROSSHAIR_X670E_GENE"
        | "ROG_STRIX_X670E_E_GAMING_WIFI"
        | "ROG_STRIX_X670E_F_GAMING_WIFI" => (Amd600, water),
        "ROG_STRIX_Z370_G_GAMING" => (Intel370, &[Chipset, TSensor, CpuOptFan, PumpFan]),
        "ROG_STRIX_Z390_E_GAMING" | "ROG_STRIX_Z390_F_GAMING" => (Intel300, z390),
        "ROG_STRIX_Z390_I_GAMING" => (Intel300, &z390[..3]),
        "ROG_MAXIMUS_XI_FORMULA" | "ROG_MAXIMUS_XI_HERO" => (Intel300, maximus_xi),
        "ROG_MAXIMUS_XII_Z490_FORMULA" => (Intel400, &[TSensor, Vrm, WaterIn, WaterOut]),
        "ROG_MAXIMUS_XII_HERO_WIFI" => (
            Intel400,
            &[TSensor, Chipset, Vrm, WaterIn, WaterOut, CpuOptFan],
        ),
        "Z170_A" => (Intel100, &[TSensor, Chipset, PumpFan]),
        "Z170_PRO_GAMING" => (Intel100, &[Chipset, Vrm, TSensor]),
        "ROG_STRIX_Z690_A_GAMING_WIFI_D4" | "ROG_STRIX_Z690_G_GAMING_WIFI" | "PRIME_Z690_A" => {
            (Intel600, tsensor_vrm)
        }
        "ROG_MAXIMUS_Z690_HERO" => (Intel600, &[TSensor, WaterIn, WaterOut]),
        "ROG_MAXIMUS_Z690_FORMULA" => (Intel600, &[TSensor, Vrm, WaterIn, WaterOut, WaterBlockIn]),
        "ROG_MAXIMUS_Z690_EXTREME_GLACIAL" => (Intel600, &[Vrm, WaterIn, WaterOut, WaterBlockIn]),
        "ROG_MAXIMUS_Z790_HERO" | "ROG_MAXIMUS_Z790_DARK_HERO" => {
            (Intel700, &[Vrm, TSensor, WaterIn, WaterOut, CpuOptFan])
        }
        "ROG_STRIX_Z790_I_GAMING_WIFI" => (Intel700, &[TSensor, TSensor2]),
        "ROG_STRIX_Z790_E_GAMING_WIFI" => (Intel700, &[WaterIn]),
        "ROG_STRIX_Z790_E_GAMING_WIFI_II" => (Intel700, &[TSensor, Vrm, CpuOptFan]),
        "ROG_MAXIMUS_Z790_FORMULA" => (Intel700, &[WaterIn, WaterOut]),
        "ROG_STRIX_Z890_E_GAMING_WIFI" => (Intel800, tsensor_vrm),
        _ => return None,
    })
}

struct Source {
    name: &'static str,
    reg: u16,
    fan: bool,
    unplugged: bool,
}

pub(crate) struct Ec {
    pawn: Box<dyn Pawn>,
    temps: Vec<Source>,
    fans: Vec<Source>,
    sleep: fn(u64),
    misses: u32,
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

impl Ec {
    fn new(pawn: Box<dyn Pawn>, family: Family, inputs: &[Input]) -> Ec {
        let mut sources: Vec<(Input, Source)> = Vec::new();
        for &input in inputs {
            if sources.iter().any(|(i, _)| *i == input) {
                continue;
            }
            let Some(reg) = place(family, input) else {
                continue;
            };
            let unplugged = matches!(
                input,
                TSensor | TSensorAlt | TSensor2 | WaterIn | WaterOut | WaterBlockIn
            );
            let source = Source {
                name: label(input),
                reg,
                fan: is_fan(input),
                unplugged,
            };
            sources.push((input, source));
        }
        sources.sort_by_key(|(_, s)| s.reg);
        let (fans, temps): (Vec<_>, Vec<_>) =
            sources.into_iter().map(|(_, s)| s).partition(|s| s.fan);
        Ec {
            pawn,
            temps,
            fans,
            sleep: sleep_ms,
            misses: 0,
        }
    }

    fn inb(&self, port: u64) -> u8 {
        self.pawn
            .read("ioctl_pio_read", &[port])
            .map_or(0xFF, |v| v as u8)
    }

    fn outb(&self, port: u64, value: u64) {
        let _ = self.pawn.exec("ioctl_pio_write", &[port, value], 0);
    }

    fn input_empty(&self) -> bool {
        for _ in 0..50 {
            if self.inb(COMMAND) & INPUT_FULL == 0 {
                return true;
            }
            (self.sleep)(1);
        }
        false
    }

    /// Data waiting, or on boards that never raise the flag, the input taken.
    fn output_ready(&mut self) -> bool {
        if self.misses > 20 {
            return true;
        }
        for _ in 0..5 {
            if self.inb(COMMAND) & OUTPUT_FULL != 0 {
                self.misses = 0;
                return true;
            }
            (self.sleep)(1);
        }
        if self.input_empty() {
            self.misses = 0;
            return true;
        }
        self.misses += 1;
        false
    }

    fn read_byte(&mut self, reg: u8) -> u8 {
        for _ in 0..5 {
            if !self.input_empty() {
                continue;
            }
            self.outb(COMMAND, READ);
            if !self.input_empty() {
                continue;
            }
            self.outb(DATA, reg as u64);
            if self.input_empty() && self.output_ready() {
                return self.inb(DATA);
            }
        }
        0
    }

    fn write_byte(&mut self, reg: u8, value: u8) {
        for _ in 0..5 {
            if !self.input_empty() {
                continue;
            }
            self.outb(COMMAND, WRITE);
            if !self.input_empty() {
                continue;
            }
            self.outb(DATA, reg as u64);
            if self.input_empty() {
                self.outb(DATA, value as u64);
                return;
            }
        }
    }

    fn bank(&mut self, bank: u8) -> u8 {
        let before = self.read_byte(BANK);
        self.write_byte(BANK, bank);
        before
    }

    /// Reads registers in order of address, switching banks as needed and back after.
    fn read_all(&mut self, regs: &[u16]) -> Vec<u8> {
        let mut bank = 0;
        let before = self.bank(0);
        let mut out = Vec::with_capacity(regs.len());
        for &reg in regs {
            let want = (reg >> 8) as u8;
            if want != bank {
                self.bank(want);
                bank = want;
            }
            out.push(self.read_byte(reg as u8));
        }
        self.bank(before);
        out
    }
}

impl Monitor for Ec {
    fn size(&self) -> (usize, usize, usize) {
        (self.temps.len(), self.fans.len(), 0)
    }

    fn update(&mut self, values: &mut Values) {
        let mut regs: Vec<u16> = Vec::new();
        for s in self.temps.iter().chain(&self.fans) {
            regs.push(s.reg);
            if s.fan {
                regs.push(s.reg + 1);
            }
        }
        let mut order: Vec<usize> = (0..regs.len()).collect();
        order.sort_by_key(|&i| regs[i]);
        let sorted: Vec<u16> = order.iter().map(|&i| regs[i]).collect();
        let data = self.read_all(&sorted);
        let at = |reg: u16| data[sorted.iter().position(|&r| r == reg).unwrap_or(0)];
        for (out, s) in values.temps.iter_mut().zip(&self.temps) {
            let t = at(s.reg) as i8 as i32;
            *out = (!(s.unplugged && t == UNPLUGGED)).then_some(t as f32);
        }
        for (out, s) in values.fans.iter_mut().zip(&self.fans) {
            let rpm = ((at(s.reg) as u16) << 8 | at(s.reg + 1) as u16) as i16;
            *out = Some(rpm as f32);
        }
    }

    fn set_control(&mut self, _index: usize, _value: Option<u8>) -> Result<(), String> {
        Err("the embedded controller is read only".into())
    }
}

fn names(sources: &[Source]) -> Vec<(String, usize)> {
    sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.to_string(), i))
        .collect()
}

fn build(
    pawn: Box<dyn Pawn>,
    model: &str,
    lock: Option<SharedLock>,
) -> Option<Box<dyn FanBackend>> {
    let (family, inputs) = board(model)?;
    let ec = Ec::new(pawn, family, inputs);
    let layout = Layout {
        temps: names(&ec.temps),
        fans: names(&ec.fans),
        controls: Vec::new(),
    };
    let board = Board::new(
        "asus ec".into(),
        "board/ec".into(),
        Box::new(ec),
        layout,
        lock,
    );
    Some(Box::new(board))
}

/// The EC sensors of a known ASUS board, through the PawnIO module `LpcACPIEC.bin`.
pub fn backend(module: &[u8]) -> Option<Box<dyn FanBackend>> {
    let dmi = crate::lights::dmi();
    if crate::boards::maker(&dmi.manufacturer) != crate::boards::Maker::ASUS {
        return None;
    }
    let model = crate::boards::model(&dmi.mainboard)?;
    board(model)?;
    let pawn = Box::new(Module::open(module).ok()?);
    build(pawn, model, Some(SharedLock::new(EC_MUTEX)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{Bus, Device, Regs, regs};

    /// An EC that answers each command at once.
    struct Acpi {
        mem: Regs,
        bank: u8,
        step: Vec<u8>,
        out: Option<u8>,
    }

    impl Device for Acpi {
        fn inb(&mut self, port: u16) -> u8 {
            match port as u64 {
                COMMAND => u8::from(self.out.is_some()),
                _ => self.out.take().unwrap_or(0),
            }
        }
        fn outb(&mut self, port: u16, value: u8) {
            if port as u64 == COMMAND {
                self.step = vec![value];
                return;
            }
            self.step.push(value);
            match (self.step[0] as u64, self.step.len()) {
                (READ, 2) if value == BANK => self.out = Some(self.bank),
                (READ, 2) => {
                    let at = (self.bank as u16) << 8 | value as u16;
                    self.out = Some(self.mem.lock().unwrap().get(&at).copied().unwrap_or(0));
                }
                (WRITE, 3) if self.step[1] == BANK => self.bank = value,
                _ => {}
            }
        }
    }

    /// The bus fake names its calls after the Super I/O module; this one speaks the EC module.
    struct Renamed(Bus);

    impl Pawn for Renamed {
        fn exec(&self, name: &str, input: &[u64], outputs: usize) -> Result<Vec<u64>, String> {
            let name = match name {
                "ioctl_pio_read" => "ioctl_pio_inb",
                "ioctl_pio_write" => "ioctl_pio_outb",
                other => other,
            };
            self.0.exec(name, input, outputs)
        }
    }

    #[test]
    fn reads_known_board_sensors_across_banks() {
        let mem = regs(&[
            (0x3A, 51),
            (0x3D, 0xD8),
            (0x100, 28),
            (0xB0, 0x04),
            (0xB1, 0xB0),
        ]);
        let bus = Bus::default();
        bus.device(
            0x62,
            0x66,
            Acpi {
                mem: mem.clone(),
                bank: 0,
                step: Vec::new(),
                out: None,
            },
        );
        let mut ec = Ec::new(
            Box::new(Renamed(bus.clone())),
            Amd500,
            board("ROG_CROSSHAIR_VIII_HERO").unwrap().1,
        );
        ec.sleep = |_| {};
        assert_eq!(ec.size(), (7, 2, 0));
        let mut v = Values {
            temps: vec![None; 7],
            fans: vec![None; 2],
        };
        ec.update(&mut v);
        let names: Vec<&str> = ec.temps.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "Chipset",
                "CPU",
                "Motherboard",
                "T sensor",
                "VRM",
                "Water in",
                "Water out"
            ]
        );
        assert_eq!(v.temps[0], Some(51.0));
        assert_eq!(v.temps[3], None);
        assert_eq!(v.temps[5], Some(28.0));
        assert_eq!(v.fans[0], Some(1200.0));
        let log = bus.take();
        let first = crate::testbus::log(&[
            "pio_inb 66",
            "pio_outb 66 80",
            "pio_inb 66",
            "pio_outb 62 ff",
            "pio_inb 66",
            "pio_inb 66",
            "pio_inb 62",
        ]);
        assert_eq!(log[..7], first);
        assert!(log.contains(&"pio_outb 62 01".to_string()));
        assert!(ec.set_control(0, Some(1)).is_err());
    }
}
