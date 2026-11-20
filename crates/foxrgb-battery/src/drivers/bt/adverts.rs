//! A passive BLE advertisement watcher running on its own thread. It only listens and hands Apple
//! manufacturer data and Fast Pair service data to the drivers that store them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::Devices::Bluetooth::Advertisement::{
    BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher,
    BluetoothLEAdvertisementWatcherStatus,
};
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, IBuffer};

const APPLE: u16 = 0x004C;
const SERVICE_DATA_16: u8 = 0x16;
const FAST_PAIR: [u8; 2] = [0x2C, 0xFE];

/// Starts the watcher once. True only on the call that started it, so the caller can wait a moment
/// for the first broadcasts (devices send one every second or two).
pub fn ensure() -> bool {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return false;
    }
    let (tx, rx) = mpsc::channel();
    if std::thread::Builder::new()
        .name("ble-adverts".into())
        .spawn(move || run(tx))
        .is_err()
    {
        return false;
    }
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
}

fn run(started: mpsc::Sender<bool>) {
    let Ok(w) = BluetoothLEAdvertisementWatcher::new() else {
        let _ = started.send(false);
        return;
    };
    let handler = TypedEventHandler::new(
        |_, args: windows::core::Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
            if let Some(a) = args.as_ref() {
                let _ = received(a);
            }
            Ok(())
        },
    );
    if w.Received(&handler).is_err() {
        let _ = started.send(false);
        return;
    }
    let _ = started.send(w.Start().is_ok());
    // Restart after the radio was switched off and on again.
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let st = w
            .Status()
            .unwrap_or(BluetoothLEAdvertisementWatcherStatus::Stopped);
        if st == BluetoothLEAdvertisementWatcherStatus::Stopped
            || st == BluetoothLEAdvertisementWatcherStatus::Aborted
        {
            let _ = w.Start();
        }
    }
}

fn bytes(buf: &IBuffer) -> windows::core::Result<Vec<u8>> {
    let mut v = vec![0u8; buf.Length()? as usize];
    DataReader::FromBuffer(buf)?.ReadBytes(&mut v)?;
    Ok(v)
}

fn received(args: &BluetoothLEAdvertisementReceivedEventArgs) -> windows::core::Result<()> {
    let addr = args.BluetoothAddress()?;
    let rssi = args.RawSignalStrengthInDBm()?;
    let adv = args.Advertisement()?;
    for md in adv.GetManufacturerDataByCompanyId(APPLE)? {
        super::super::airpods::heard(addr, rssi, &bytes(&md.Data()?)?);
    }
    for sec in adv.GetSectionsByType(SERVICE_DATA_16)? {
        let data = bytes(&sec.Data()?)?;
        if let Some(rest) = data.strip_prefix(&FAST_PAIR) {
            super::super::generic::heard_fast_pair(addr, rest);
        }
    }
    Ok(())
}
