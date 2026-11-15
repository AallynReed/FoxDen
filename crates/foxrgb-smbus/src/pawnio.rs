//! The Intel chipset SMBus through PawnIO. Bus calls from OpenRGB's i2c_smbus_pawnio.cpp.

use foxrgb_pawnio::{Module, SMBUS_I801, SharedLock};

/// Shared with other SMBus software so two programs never interleave transfers.
const BUS_MUTEX: &str = r"Global\Access_SMBUS.HTP.Method";
const SLEEP_ALWAYS: u64 = 2;

const READ: u64 = 1;
const WRITE: u64 = 0;
const QUICK: u64 = 0;
const BYTE_DATA: u64 = 2;
const WORD_DATA: u64 = 3;
const BLOCK_DATA: u64 = 5;

pub struct Bus {
    module: Module,
    lock: SharedLock,
}

impl Bus {
    /// Opens the Intel SMBus. Fails without PawnIO, without administrator rights or on another chipset.
    pub fn open() -> Result<Bus, String> {
        let module = Module::open(SMBUS_I801).map_err(|e| format!("no Intel SMBus here ({e})"))?;
        let _ = module.execute("ioctl_set_sleep_mode", &[SLEEP_ALWAYS], 0);
        Ok(Bus {
            module,
            lock: SharedLock::new(BUS_MUTEX),
        })
    }

    /// One SMBus transfer. `data` is the 34-byte i2c_smbus_data union.
    fn xfer(
        &self,
        addr: u8,
        read: bool,
        command: u8,
        size: u64,
        data: &mut [u8; 40],
    ) -> Result<(), String> {
        let mut input = vec![
            addr as u64,
            if read { READ } else { WRITE },
            command as u64,
            size,
        ];
        input.extend(
            data.chunks_exact(8)
                .map(|b| u64::from_le_bytes(b.try_into().unwrap())),
        );
        let out = self
            .lock
            .run(|| self.module.execute("ioctl_smbus_xfer", &input, 5))?;
        for (i, value) in out.iter().enumerate().take(5) {
            data[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }

    pub fn write_quick(&self, addr: u8) -> Result<(), String> {
        self.xfer(addr, false, 0, QUICK, &mut [0; 40])
    }

    pub fn read_byte(&self, addr: u8, command: u8) -> Result<u8, String> {
        let mut data = [0u8; 40];
        self.xfer(addr, true, command, BYTE_DATA, &mut data)?;
        Ok(data[0])
    }

    pub fn write_byte(&self, addr: u8, command: u8, value: u8) -> Result<(), String> {
        let mut data = [0u8; 40];
        data[0] = value;
        self.xfer(addr, false, command, BYTE_DATA, &mut data)
    }

    pub fn write_word(&self, addr: u8, command: u8, value: u16) -> Result<(), String> {
        let mut data = [0u8; 40];
        data[..2].copy_from_slice(&value.to_le_bytes());
        self.xfer(addr, false, command, WORD_DATA, &mut data)
    }

    pub fn write_block(&self, addr: u8, command: u8, bytes: &[u8]) -> Result<(), String> {
        let mut data = [0u8; 40];
        let n = bytes.len().min(32);
        data[0] = n as u8;
        data[1..1 + n].copy_from_slice(&bytes[..n]);
        self.xfer(addr, false, command, BLOCK_DATA, &mut data)
    }
}
