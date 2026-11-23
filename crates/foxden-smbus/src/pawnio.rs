//! The chipset SMBus controllers (Intel, AMD, Nuvoton SuperIO) through PawnIO.

use foxden_pawnio::{Module, SMBUS_I801, SMBUS_NCT6793, SMBUS_PIIX4, SharedLock};

/// Shared with other SMBus software so two programs never interleave transfers.
const BUS_MUTEX: &str = r"Global\Access_SMBUS.HTP.Method";
/// Polls without sleeping: a sleep waits for a timer tick, up to 15 ms a transfer.
const BUSY_WAIT: u64 = 0;

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
    /// The AMD controller has two ports behind one set of registers; this bus's port is chosen before each transfer.
    port: Option<u64>,
}

impl Bus {
    /// The first SMBus found. Fails without PawnIO, without administrator rights or without a known controller.
    pub fn open() -> Result<Bus, String> {
        Self::open_all()?
            .into_iter()
            .next()
            .ok_or_else(|| "no SMBus here".into())
    }

    /// Every SMBus this PC has: Intel, both AMD ports, and the Nuvoton SuperIO one.
    pub fn open_all() -> Result<Vec<Bus>, String> {
        let (buses, why) = Self::open_each();
        if buses.is_empty() {
            return Err(format!("no SMBus here ({})", why.join("; ")));
        }
        Ok(buses)
    }

    /// The buses that opened, and why each of the others did not.
    pub fn open_each() -> (Vec<Bus>, Vec<String>) {
        let mut buses = Vec::new();
        let mut why = Vec::new();
        let mut add = |name: &str, code: &[u8], port: Option<u64>| match Module::open(code) {
            Ok(module) => {
                if let Some(p) = port
                    && module.execute("ioctl_piix4_port_sel", &[p], 1).is_err()
                {
                    why.push(format!("{name}: no such port"));
                    return;
                }
                buses.push(Bus::new(module, name, port));
            }
            Err(e) => why.push(format!("{name}: {e}")),
        };
        add("Intel SMBus", SMBUS_I801, None);
        add("AMD SMBus port 0", SMBUS_PIIX4, Some(0));
        add("AMD SMBus port 1", SMBUS_PIIX4, Some(1));
        add("Nuvoton SMBus", SMBUS_NCT6793, None);
        (buses, why)
    }

    fn new(module: Module, name: &str, port: Option<u64>) -> Bus {
        let _ = module.execute("ioctl_set_sleep_mode", &[BUSY_WAIT], 0);
        // The module reports the controller's PCI ids, which PCI detectors match on.
        let mut info = crate::i2c::BusInfo {
            name: name.into(),
            ..Default::default()
        };
        if let Ok(out) = module.execute("ioctl_identity", &[0], 3)
            && out.len() == 3
        {
            info.pci_vendor = out[2] as u16;
            info.pci_device = (out[2] >> 16) as u16;
            info.pci_subsystem_vendor = (out[2] >> 32) as u16;
            info.pci_subsystem_device = (out[2] >> 48) as u16;
        }
        Bus {
            module,
            lock: SharedLock::new(BUS_MUTEX),
            info,
            port,
        }
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
        let out = self.lock.run(|| {
            if let Some(p) = self.port {
                self.module.execute("ioctl_piix4_port_sel", &[p], 1)?;
            }
            self.module.execute("ioctl_smbus_xfer", &input, 5)
        })??;
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
