//! Battery levels of wireless mice, keyboards, headsets and controllers.

pub mod corsair_catalog;
mod ctx;
mod drivers;
pub mod hid;
pub mod util;

pub use ctx::Ctx;
pub use drivers::corsair::Channel as Bragi;

/// Read-only HID++ answers of a Logitech receiver and its devices, for device reports.
pub fn logitech_diagnose(pid: u16) -> Vec<String> {
    drivers::logitech_diagnose(pid)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Other,
    Mouse,
    Headset,
    Keyboard,
    Gamepad,
    Phone,
    Microphone,
}

/// One battery-powered device, as seen in one poll.
#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    /// Stays the same across polls and USB ports.
    pub key: String,
    pub name: String,
    /// "2.4 GHz receiver", "Bluetooth", "USB cable"...
    pub via: String,
    pub kind: Kind,
    /// 0..=100, `None` when unknown right now.
    pub percent: Option<u8>,
    /// Extra breakdown, e.g. "L 80%, R 75%, case 40%".
    pub detail: String,
    /// Coarse level for devices without a percentage ("Low", "Full").
    pub level: String,
    pub charging: bool,
    pub full: bool,
    /// The receiver answered but the device sleeps: keep the last value.
    pub asleep: bool,
    /// False when the receiver is there but the device is off.
    pub connected: bool,
    pub millivolts: u32,
    /// Keep showing the last value this long if the device stops answering.
    pub grace_secs: u32,
    pub vid: u16,
}

impl Default for Battery {
    fn default() -> Self {
        Self {
            key: String::new(),
            name: String::new(),
            via: String::new(),
            kind: Kind::Other,
            percent: None,
            detail: String::new(),
            level: String::new(),
            charging: false,
            full: false,
            asleep: false,
            connected: true,
            millivolts: 0,
            grace_secs: 0,
            vid: 0,
        }
    }
}

/// Asks every supported device. Blocks for a second or more: call it off the UI thread.
pub fn poll() -> Vec<Battery> {
    let mut ctx = Ctx::new(hid::enumerate());
    for driver in drivers::ORDER {
        driver(&mut ctx);
    }
    ctx.out
}
