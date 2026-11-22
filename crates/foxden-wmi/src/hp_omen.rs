// SPDX-License-Identifier: GPL-2.0-or-later

use std::collections::HashMap;

use foxden_core::{Backend, Color, Device, DeviceKind};
use wmi::{IWbemClassWrapper, Variant, WMIConnection};

pub(crate) const COMMAND_INFO: u32 = 131080;
const COMMAND_LIGHTING: u32 = 131081;
const TYPE_SUPPORTED: u32 = 1;
const TYPE_GET_COLORS: u32 = 2;
const TYPE_SET_COLORS: u32 = 3;
const TYPE_MODE: u32 = 5;
const TYPE_KEYBOARD: u32 = 43;
const MODE_DIRECT: [u8; 4] = [228, 0, 0, 0];
/// Keyboard type without numpad.
const WITHOUT_NUMPAD: u8 = 2;
const SIGN: [u8; 4] = [83, 69, 67, 85];
const COLORS_OFFSET: usize = 25;

/// hpqBIntM.hpqBIOSInt128; tests swap in a logger.
pub(crate) trait Bios: Send {
    fn execute(
        &self,
        command: u32,
        kind: u32,
        input: &[u8],
        output: bool,
    ) -> Result<Vec<u8>, String>;

    /// Like `execute` with output, but only when the BIOS reports success.
    fn query(&self, _command: u32, _kind: u32, _input: &[u8]) -> Result<Vec<u8>, String> {
        Err("not supported".into())
    }

    /// The baseboard product name.
    fn board(&self) -> Option<String> {
        None
    }
}

pub(crate) struct Wmi;

fn e(e: wmi::WMIError) -> String {
    e.to_string()
}

impl Wmi {
    /// A fresh connection per call; WMI connections are tied to a thread.
    fn call(command: u32, kind: u32, input: &[u8]) -> Result<Option<IWbemClassWrapper>, String> {
        let con = WMIConnection::with_namespace_path("ROOT\\WMI").map_err(e)?;
        let params = con
            .get_object("hpqBIntM")
            .map_err(e)?
            .get_method("hpqBIOSInt128")
            .map_err(e)?
            .ok_or("hpqBIOSInt128 takes no input")?
            .spawn_instance()
            .map_err(e)?;
        let data_in = con
            .get_object("hpqBDataIn")
            .map_err(e)?
            .spawn_instance()
            .map_err(e)?;
        data_in.put_property("Sign", SIGN.to_vec()).map_err(e)?;
        data_in.put_property("Command", command as i32).map_err(e)?;
        data_in
            .put_property("CommandType", kind as i32)
            .map_err(e)?;
        data_in
            .put_property("Size", input.len() as i32)
            .map_err(e)?;
        data_in
            .put_property("hpqBData", input.to_vec())
            .map_err(e)?;
        params.put_property("InData", data_in).map_err(e)?;
        let path = r"hpqBIntM.InstanceName='ACPI\PNP0C14\0_0'";
        let out = con
            .exec_method(path, "hpqBIOSInt128", Some(&params))
            .map_err(e)?;
        let Some(Variant::Object(data)) = out
            .map(|o| o.get_property("OutData"))
            .transpose()
            .map_err(e)?
        else {
            return Ok(None);
        };
        Ok(Some(data))
    }
}

impl Bios for Wmi {
    fn execute(
        &self,
        command: u32,
        kind: u32,
        input: &[u8],
        output: bool,
    ) -> Result<Vec<u8>, String> {
        let data = Wmi::call(command, kind, input)?;
        if !output {
            return Ok(Vec::new());
        }
        let data = data.ok_or("no OutData")?;
        data.get_property("Data").map_err(e)?.try_into().map_err(e)
    }

    fn query(&self, command: u32, kind: u32, input: &[u8]) -> Result<Vec<u8>, String> {
        let data = Wmi::call(command, kind, input)?.ok_or("no OutData")?;
        match data.get_property("rwReturnCode").map_err(e)? {
            Variant::UI4(0) | Variant::I4(0) => {}
            code => return Err(format!("BIOS returned {code:?}")),
        }
        data.get_property("Data").map_err(e)?.try_into().map_err(e)
    }

    fn board(&self) -> Option<String> {
        let con = WMIConnection::new().ok()?;
        let rows: Vec<HashMap<String, Variant>> =
            con.raw_query("SELECT Product FROM Win32_BaseBoard").ok()?;
        match rows.first()?.get("Product")? {
            Variant::String(name) => Some(name.trim().to_string()),
            _ => None,
        }
    }
}

struct Omen {
    bios: Box<dyn Bios>,
    device: Device,
    sent: Option<Vec<Color>>,
}

impl Omen {
    /// changeMode(DIRECT).
    fn change_mode(&self) {
        let _ = self
            .bios
            .execute(COMMAND_LIGHTING, TYPE_MODE, &MODE_DIRECT, false);
    }

    /// setColors: the current table read back, zones written right to left.
    fn set_colors(&self, colors: &[Color]) {
        let Ok(mut data) = self
            .bios
            .execute(COMMAND_LIGHTING, TYPE_GET_COLORS, &[], true)
        else {
            return;
        };
        if data.len() < COLORS_OFFSET + 12 {
            return;
        }
        for i in 0..4 {
            let c = colors[3 - i];
            data[COLORS_OFFSET + i * 3..][..3].copy_from_slice(&[c.0, c.1, c.2]);
        }
        let _ = self
            .bios
            .execute(COMMAND_LIGHTING, TYPE_SET_COLORS, &data, false);
    }
}

impl Backend for Omen {
    fn name(&self) -> &str {
        "hp omen"
    }

    fn devices(&self) -> Vec<Device> {
        vec![self.device.clone()]
    }

    fn push(&mut self, device_id: &str, colors: &[Color]) -> Result<(), String> {
        if device_id != self.device.id {
            return Err(format!("unknown device {device_id}"));
        }
        let mut colors = colors.to_vec();
        colors.resize(4, Color::BLACK);
        if self.sent.as_ref() == Some(&colors) {
            return Ok(());
        }
        if self.sent.is_none() {
            self.change_mode();
        }
        self.set_colors(&colors);
        self.sent = Some(colors);
        Ok(())
    }
}

pub fn detect() -> Vec<Box<dyn Backend>> {
    detect_with(Box::new(Wmi))
}

/// Lighting supported, and the 4-zone keyboard without a numpad.
fn detect_with(bios: Box<dyn Bios>) -> Vec<Box<dyn Backend>> {
    let first = |command, kind| {
        bios.execute(command, kind, &[], true)
            .ok()
            .and_then(|data| data.first().copied())
    };
    let supported = first(COMMAND_LIGHTING, TYPE_SUPPORTED).is_some_and(|b| b & 1 == 1);
    if !supported || first(COMMAND_INFO, TYPE_KEYBOARD) != Some(WITHOUT_NUMPAD) {
        return Vec::new();
    }
    let device = Device::grid(
        "wmi:hp-omen",
        "Omen 4-Zone Laptop Keyboard",
        DeviceKind::Keyboard,
        [0.10, 0.55, 0.80, 0.30],
        4,
        1,
    );
    vec![Box::new(Omen {
        bios,
        device,
        sent: None,
    })]
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    type Log = Arc<Mutex<Vec<(u32, u32, Vec<u8>)>>>;

    struct Fake {
        log: Log,
        supported: u8,
        keyboard: u8,
    }

    impl Bios for Fake {
        fn execute(
            &self,
            command: u32,
            kind: u32,
            input: &[u8],
            output: bool,
        ) -> Result<Vec<u8>, String> {
            self.log
                .lock()
                .unwrap()
                .push((command, kind, input.to_vec()));
            if !output {
                return Ok(Vec::new());
            }
            Ok(match (command, kind) {
                (COMMAND_LIGHTING, TYPE_SUPPORTED) => vec![self.supported; 128],
                (COMMAND_INFO, TYPE_KEYBOARD) => vec![self.keyboard; 128],
                (COMMAND_LIGHTING, TYPE_GET_COLORS) => (0..128).collect(),
                _ => return Err("unexpected".into()),
            })
        }
    }

    fn run(supported: u8, keyboard: u8) -> (Log, Vec<Box<dyn Backend>>) {
        let log = Log::default();
        let fake = Fake {
            log: log.clone(),
            supported,
            keyboard,
        };
        (log.clone(), detect_with(Box::new(fake)))
    }

    fn take(log: &Log) -> Vec<(u32, u32, Vec<u8>)> {
        std::mem::take(&mut *log.lock().unwrap())
    }

    #[test]
    fn detection() {
        let (log, found) = run(0x03, 2);
        assert_eq!(found.len(), 1);
        assert_eq!(take(&log), [(131081, 1, vec![]), (131080, 43, vec![])]);
        let (log, found) = run(0x02, 2);
        assert!(found.is_empty());
        assert_eq!(take(&log), [(131081, 1, vec![])]);
        let (_, found) = run(0x01, 1);
        assert!(found.is_empty());
    }

    // Direct mode then the colours: read, colors[3 - i] at 25 + 3i, write back.
    #[test]
    fn direct_frame() {
        let (log, mut found) = run(1, 2);
        take(&log);
        let colors = [
            Color(1, 2, 3),
            Color(4, 5, 6),
            Color(7, 8, 9),
            Color(10, 11, 12),
        ];
        found[0].push("wmi:hp-omen", &colors).unwrap();
        let mut table: Vec<u8> = (0..128).collect();
        table[25..37].copy_from_slice(&[10, 11, 12, 7, 8, 9, 4, 5, 6, 1, 2, 3]);
        assert_eq!(
            take(&log),
            [
                (131081, 5, vec![228, 0, 0, 0]),
                (131081, 2, vec![]),
                (131081, 3, table),
            ]
        );
        found[0].push("wmi:hp-omen", &colors).unwrap();
        assert!(take(&log).is_empty());
        found[0].push("wmi:hp-omen", &[Color::BLACK; 4]).unwrap();
        assert_eq!(take(&log).len(), 2);
    }
}
