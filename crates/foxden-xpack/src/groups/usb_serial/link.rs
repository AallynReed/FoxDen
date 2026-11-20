//! Raw USB (WinUSB bulk) and USB serial transports behind the shared `Io` trait.

use crate::hid::{Io, padded};
use foxden_hid::winusb::{self, WinUsb};
use serialport::{SerialPort, SerialPortType};
use std::io::Write;
use std::time::Duration;

/// One WinUSB-bound interface of a raw USB device, writing to one bulk OUT endpoint.
pub struct Bulk {
    handle: WinUsb,
}

impl Io for Bulk {
    fn write(&mut self, data: &[u8], len: usize) -> Result<(), String> {
        self.handle.write_pipe(&padded(data, len))
    }
    fn feature(&mut self, _data: &[u8], _len: usize) -> Result<(), String> {
        Err("no feature reports on a bulk pipe".into())
    }
    fn get_feature(&mut self, _first: u8, _len: usize) -> Result<Vec<u8>, String> {
        Err("no feature reports on a bulk pipe".into())
    }
    fn read(&mut self, _len: usize, _timeout_ms: i32) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

/// A raw USB device that was opened, with an id stable across runs.
pub struct Found {
    pub io: Bulk,
    pub id: String,
}

/// Opens every attached `vid:pid` whose WinUSB-bound interface owns bulk OUT `endpoint`.
pub fn open_bulk(family: &str, vid: u16, pid: u16, endpoint: u8) -> Vec<Found> {
    WinUsb::paths(vid, pid)
        .into_iter()
        .filter_map(|path| {
            let handle = WinUsb::at(&path, endpoint, endpoint | 0x80).ok()?;
            handle.has_pipe(endpoint).then(|| Found {
                io: Bulk { handle },
                id: format!(
                    "usb:{family}-{vid:04x}{pid:04x}-{}",
                    winusb::instance(&path).replace('&', ".")
                ),
            })
        })
        .collect()
}

/// A USB serial port; frames are written as they are, without report ids or padding.
pub struct Serial {
    port: Box<dyn SerialPort>,
}

impl Io for Serial {
    fn write(&mut self, data: &[u8], _len: usize) -> Result<(), String> {
        self.port.write_all(data).map_err(|e| e.to_string())?;
        self.port.flush().map_err(|e| e.to_string())
    }
    fn feature(&mut self, _data: &[u8], _len: usize) -> Result<(), String> {
        Err("no feature reports on a serial port".into())
    }
    fn get_feature(&mut self, _first: u8, _len: usize) -> Result<Vec<u8>, String> {
        Err("no feature reports on a serial port".into())
    }
    fn read(&mut self, _len: usize, _timeout_ms: i32) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }
}

pub struct Port {
    pub io: Serial,
    pub id: String,
}

/// Opens every serial port of a USB device with one of `pids`, at `baud` 8N1.
pub fn open_serial(family: &str, vid: u16, pids: &[u16], baud: u32) -> Vec<Port> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for p in ports {
        let SerialPortType::UsbPort(u) = &p.port_type else {
            continue;
        };
        // macOS lists each port twice; the call-out device is the one to open
        if u.vid != vid || !pids.contains(&u.pid) || p.port_name.starts_with("/dev/tty.") {
            continue;
        }
        let tail = u
            .serial_number
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| p.port_name.clone());
        if seen.contains(&tail) {
            continue;
        }
        let Ok(port) = serialport::new(&p.port_name, baud)
            .timeout(Duration::from_millis(500))
            .open()
        else {
            continue;
        };
        seen.push(tail.clone());
        out.push(Port {
            io: Serial { port },
            id: format!("serial:{family}-{vid:04x}{:04x}-{tail}", u.pid),
        });
    }
    out
}
