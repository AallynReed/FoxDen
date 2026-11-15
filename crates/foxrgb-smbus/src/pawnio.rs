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
    info: crate::i2c::BusInfo,
}

impl Bus {
    /// Opens the Intel SMBus. Fails without PawnIO, without administrator rights or on another chipset.
    pub fn open() -> Result<Bus, String> {
        let module = Module::open(SMBUS_I801).map_err(|e| format!("no Intel SMBus here ({e})"))?;
        let _ = module.execute("ioctl_set_sleep_mode", &[SLEEP_ALWAYS], 0);
        // The i801 module reports the controller's PCI ids, which PCI detectors match on.
        let info = match module.execute("ioctl_identity", &[0], 3) {
            Ok(out) if out.len() == 3 => crate::i2c::BusInfo {
                name: "Intel SMBus".into(),
                pci_vendor: out[2] as u16,
                pci_device: (out[2] >> 16) as u16,
                pci_subsystem_vendor: (out[2] >> 32) as u16,
                pci_subsystem_device: (out[2] >> 48) as u16,
            },
            _ => crate::i2c::BusInfo {
                name: "Intel SMBus".into(),
                ..Default::default()
            },
        };
        Ok(Bus {
            module,
            lock: SharedLock::new(BUS_MUTEX),
            info,
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

const BYTE: u64 = 1;
const I2C_BLOCK_DATA: u64 = 8;

impl crate::i2c::SmBus for Bus {
    fn info(&self) -> &crate::i2c::BusInfo {
        &self.info
    }
    fn write_quick(&self, addr: u8) -> Result<(), String> {
        Bus::write_quick(self, addr)
    }
    fn read_byte(&self, addr: u8) -> Result<u8, String> {
        let mut data = [0u8; 40];
        self.xfer(addr, true, 0, BYTE, &mut data)?;
        Ok(data[0])
    }
    fn write_byte(&self, addr: u8, value: u8) -> Result<(), String> {
        self.xfer(addr, false, value, BYTE, &mut [0; 40])
    }
    fn read_byte_data(&self, addr: u8, command: u8) -> Result<u8, String> {
        Bus::read_byte(self, addr, command)
    }
    fn write_byte_data(&self, addr: u8, command: u8, value: u8) -> Result<(), String> {
        Bus::write_byte(self, addr, command, value)
    }
    fn read_word_data(&self, addr: u8, command: u8) -> Result<u16, String> {
        let mut data = [0u8; 40];
        self.xfer(addr, true, command, WORD_DATA, &mut data)?;
        Ok(u16::from_le_bytes([data[0], data[1]]))
    }
    fn write_word_data(&self, addr: u8, command: u8, value: u16) -> Result<(), String> {
        self.write_word(addr, command, value)
    }
    fn read_block_data(&self, addr: u8, command: u8) -> Result<Vec<u8>, String> {
        let mut data = [0u8; 40];
        self.xfer(addr, true, command, BLOCK_DATA, &mut data)?;
        let n = (data[0] as usize).min(32);
        Ok(data[1..1 + n].to_vec())
    }
    fn write_block_data(&self, addr: u8, command: u8, data: &[u8]) -> Result<(), String> {
        self.write_block(addr, command, data)
    }
    fn i2c_read_block_data(&self, addr: u8, command: u8, len: u8) -> Result<Vec<u8>, String> {
        let mut data = [0u8; 40];
        let n = (len as usize).min(32);
        data[0] = n as u8;
        self.xfer(addr, true, command, I2C_BLOCK_DATA, &mut data)?;
        Ok(data[1..1 + n].to_vec())
    }
    fn i2c_write_block_data(&self, addr: u8, command: u8, bytes: &[u8]) -> Result<(), String> {
        let mut data = [0u8; 40];
        let n = bytes.len().min(32);
        data[0] = n as u8;
        data[1..1 + n].copy_from_slice(&bytes[..n]);
        self.xfer(addr, false, command, I2C_BLOCK_DATA, &mut data)
    }
}
