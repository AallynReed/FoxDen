//! GameSir Cyclone Pro on its dongle: the dongle is a wired Xbox 360 pad to Windows, but the pad
//! also answers GameSir's "base info" request over Bluetooth LE (service 0x8650, char 0x865F).
//! Connecting can take seconds, so the query runs on its own thread and the poll shows its result.

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::Devices::Bluetooth::BluetoothLEDevice;
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristicProperties, GattClientCharacteristicConfigurationDescriptorValue as Cccd,
    GattCommunicationStatus, GattValueChangedEventArgs, GattWriteOption,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter};
use windows::core::GUID;

use super::bt::nodes;
use crate::{Battery, Ctx, Kind};

const VID: u16 = 0x3537;
const DONGLE: u16 = 0x1015;
const SERVICE: GUID = GUID::from_u128(0x00008650_0000_1000_8000_00805f9b34fb);
const INFO: GUID = GUID::from_u128(0x0000865f_0000_1000_8000_00805f9b34fb);
const GET_BASE_INFO: [u8; 2] = [0x01, 0x01];

const EVERY: Duration = Duration::from_secs(120);
const RETRY: Duration = Duration::from_secs(60);
const STALE: Duration = Duration::from_secs(360);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    /// `None` while charging: the pad reports no level then.
    percent: Option<u8>,
    charging: bool,
}

/// The 19-byte base-info packet (some firmwares prefix C9); the battery is its last byte, FF = charging.
fn parse(p: &[u8]) -> Option<Reading> {
    let p = match p {
        [0xC9, rest @ ..] if p.len() == 20 => rest,
        _ => p,
    };
    if p.len() != 19 {
        return None;
    }
    match p[18] {
        0xFF => Some(Reading {
            percent: None,
            charging: true,
        }),
        v @ 0..=100 => Some(Reading {
            percent: Some(v),
            charging: false,
        }),
        _ => None,
    }
}

#[derive(Default)]
struct State {
    address: u64,
    running: bool,
    last_try: Option<Instant>,
    last_failed: bool,
    last_ok: Option<Instant>,
    reading: Option<Reading>,
}

static STATE: Mutex<State> = Mutex::new(State {
    address: 0,
    running: false,
    last_try: None,
    last_failed: false,
    last_ok: None,
    reading: None,
});

fn query(addr: u64) -> windows::core::Result<Option<Reading>> {
    let dev = BluetoothLEDevice::FromBluetoothAddressAsync(addr)?.join()?;
    let services = dev.GetGattServicesForUuidAsync(SERVICE)?.join()?;
    if services.Status()? != GattCommunicationStatus::Success || services.Services()?.Size()? == 0 {
        return Ok(None);
    }
    let svc = services.Services()?.GetAt(0)?;
    let chars = svc.GetCharacteristicsForUuidAsync(INFO)?.join()?;
    if chars.Status()? != GattCommunicationStatus::Success || chars.Characteristics()?.Size()? == 0
    {
        return Ok(None);
    }
    let ch = chars.Characteristics()?.GetAt(0)?;
    let props = ch.CharacteristicProperties()?;
    if !props.contains(GattCharacteristicProperties::Notify) {
        return Ok(None);
    }
    let (tx, rx) = mpsc::channel();
    let handler = TypedEventHandler::new(
        move |_, args: windows::core::Ref<GattValueChangedEventArgs>| {
            if let Some(a) = args.as_ref() {
                let buf = a.CharacteristicValue()?;
                let mut v = vec![0u8; buf.Length()? as usize];
                DataReader::FromBuffer(&buf)?.ReadBytes(&mut v)?;
                if let Some(r) = parse(&v) {
                    let _ = tx.send(r);
                }
            }
            Ok(())
        },
    );
    let token = ch.ValueChanged(&handler)?;
    let ask = || -> windows::core::Result<Option<Reading>> {
        if ch
            .WriteClientCharacteristicConfigurationDescriptorAsync(Cccd::Notify)?
            .join()?
            != GattCommunicationStatus::Success
        {
            return Ok(None);
        }
        let w = DataWriter::new()?;
        w.WriteBytes(&GET_BASE_INFO)?;
        let opt = if props.contains(GattCharacteristicProperties::WriteWithoutResponse) {
            GattWriteOption::WriteWithoutResponse
        } else {
            GattWriteOption::WriteWithResponse
        };
        if ch
            .WriteValueWithOptionAsync(&w.DetachBuffer()?, opt)?
            .join()?
            != GattCommunicationStatus::Success
        {
            return Ok(None);
        }
        Ok(rx.recv_timeout(Duration::from_secs(3)).ok())
    };
    let result = ask();
    if let Ok(op) = ch.WriteClientCharacteristicConfigurationDescriptorAsync(Cccd::None) {
        let _ = op.join();
    }
    let _ = ch.RemoveValueChanged(token);
    let _ = svc.Close();
    let _ = dev.Close();
    result
}

fn worker(addr: u64, done: mpsc::Sender<()>) {
    // The first GATT calls after the link comes up often fail; one retry covers it.
    let r = query(addr).ok().flatten().or_else(|| {
        std::thread::sleep(Duration::from_millis(1500));
        query(addr).ok().flatten()
    });
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if s.address == addr {
        s.last_failed = r.is_none();
        if let Some(mut r) = r {
            if r.percent.is_none() {
                r.percent = s.reading.and_then(|o| o.percent);
            }
            s.reading = Some(r);
            s.last_ok = Some(Instant::now());
        }
    }
    s.running = false;
    let _ = done.send(());
}

/// Address of the first paired Cyclone pad.
fn find_pad() -> Option<u64> {
    nodes::list("BTHLE")
        .into_iter()
        .find(|n| n.is_device() && n.name.to_lowercase().starts_with("gamesir-cyclone"))
        .and_then(|n| n.address())
}

pub fn poll_game_sir_ble(ctx: &mut Ctx) {
    let Some(addr) = find_pad() else { return };
    let dongle = ctx.colls.iter().any(|c| c.vid == VID && c.pid == DONGLE);
    let mut first = None;
    {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if s.address != addr {
            *s = State {
                address: addr,
                ..State::default()
            };
        }
        let wait = if s.last_failed { RETRY } else { EVERY };
        if !s.running && s.last_try.is_none_or(|t| t.elapsed() >= wait) {
            let is_first = s.last_try.is_none();
            s.last_try = Some(Instant::now());
            let (tx, rx) = mpsc::channel();
            if std::thread::Builder::new()
                .name("gamesir-ble".into())
                .spawn(move || worker(addr, tx))
                .is_ok()
            {
                s.running = true;
                if is_first {
                    first = Some(rx);
                }
            }
        }
    }
    // Only the very first read is waited for; later ones show on the next poll.
    if let Some(rx) = first {
        let _ = rx.recv_timeout(Duration::from_secs(4));
    }
    // The dongle's XInput level is always "Full": keep it out of the fallbacks.
    if dongle {
        ctx.claim(VID, DONGLE);
    }
    let s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let (Some(ok), Some(r)) = (s.last_ok, s.reading) else {
        return;
    };
    if ok.elapsed() >= STALE {
        return;
    }
    let b = Battery {
        key: ctx.stable_key(&format!("gamesir-{addr:012x}")),
        name: "GameSir Cyclone Pro".into(),
        via: if dongle {
            "2.4 GHz receiver"
        } else {
            "Bluetooth"
        }
        .into(),
        kind: Kind::Gamepad,
        percent: r.percent,
        charging: r.charging,
        grace_secs: 600,
        vid: VID,
        ..Default::default()
    };
    ctx.add(b, VID, if dongle { DONGLE } else { 0 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_info_battery_is_the_last_byte() {
        let mut p = [0u8; 19];
        p[18] = 64;
        assert_eq!(
            parse(&p),
            Some(Reading {
                percent: Some(64),
                charging: false
            })
        );
        p[18] = 0xFF;
        assert_eq!(
            parse(&p),
            Some(Reading {
                percent: None,
                charging: true
            })
        );
        let mut prefixed = vec![0xC9];
        prefixed.extend_from_slice(&[0u8; 18]);
        prefixed.push(12);
        assert_eq!(parse(&prefixed).and_then(|r| r.percent), Some(12));
        p[18] = 150;
        assert_eq!(parse(&p), None);
        assert_eq!(parse(&[0u8; 18]), None);
    }
}
