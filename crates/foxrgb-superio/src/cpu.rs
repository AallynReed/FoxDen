//! Intel CPU package temperature from MSRs.

use foxrgb_pawnio::{INTEL_MSR, Module};

const TEMPERATURE_TARGET: u64 = 0x1A2;
const PACKAGE_THERM_STATUS: u64 = 0x1B1;

pub struct CpuTemp {
    module: Module,
    tj_max: f32,
}

impl CpuTemp {
    pub fn open() -> Result<CpuTemp, String> {
        let module = Module::open(INTEL_MSR)?;
        let target = read(&module, TEMPERATURE_TARGET)?;
        let tj_max = ((target >> 16) & 0xFF) as f32;
        if !(60.0..=125.0).contains(&tj_max) {
            return Err(format!("implausible TjMax {tj_max}"));
        }
        Ok(CpuTemp { module, tj_max })
    }

    /// Package temperature in degrees Celsius.
    pub fn read(&self) -> Option<f32> {
        let status = read(&self.module, PACKAGE_THERM_STATUS).ok()?;
        // bit 31: reading valid
        if status & (1 << 31) == 0 {
            return None;
        }
        Some(self.tj_max - ((status >> 16) & 0x7F) as f32)
    }
}

fn read(module: &Module, msr: u64) -> Result<u64, String> {
    module
        .execute("ioctl_read_msr", &[msr], 1)?
        .first()
        .copied()
        .ok_or("no value".into())
}
