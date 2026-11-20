//! One request at a time on HID channels that lighting and battery reads share.
//! Windows hands every input report to every open handle, so each side drains its own queue first.

use std::sync::{Mutex, MutexGuard};

static TURN: Mutex<()> = Mutex::new(());

pub fn take() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|e| e.into_inner())
}
