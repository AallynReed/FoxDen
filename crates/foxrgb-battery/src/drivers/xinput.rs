//! Xbox controllers: a real percentage from Windows.Gaming.Input, else XInput's coarse levels.

use std::sync::OnceLock;
use std::time::Duration;

use windows::Gaming::Input::RawGameController;
use windows::System::Power::BatteryStatus;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::UI::Input::XboxController::{
    BATTERY_DEVTYPE, BATTERY_DEVTYPE_GAMEPAD, BATTERY_DEVTYPE_HEADSET, BATTERY_LEVEL_EMPTY,
    BATTERY_LEVEL_LOW, BATTERY_LEVEL_MEDIUM, BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_UNKNOWN,
    BATTERY_TYPE_WIRED, XINPUT_BATTERY_INFORMATION, XINPUT_CAPABILITIES, XINPUT_STATE,
    XInputGetBatteryInformation, XInputGetState,
};

use super::bt::nodes::wide;
use crate::util::clamp100;
use crate::{Battery, Ctx, Kind};

const MICROSOFT: u16 = 0x045E;
/// GameSir pads in Xbox (GIP) mode also report a real battery.
const GAMESIR: u16 = 0x3537;

/// Adds pads with a real level; returns (added, of which not Microsoft).
fn poll_gaming_input(ctx: &mut Ctx) -> (usize, usize) {
    static FIRST: OnceLock<()> = OnceLock::new();
    // The list fills in shortly after first use.
    if FIRST.set(()).is_ok() && RawGameController::RawGameControllers().is_ok() {
        std::thread::sleep(Duration::from_millis(300));
    }
    let Ok(list) = RawGameController::RawGameControllers() else {
        return (0, 0);
    };
    let (mut added, mut third) = (0, 0);
    for raw in list {
        let (Ok(vid), Ok(pid)) = (raw.HardwareVendorId(), raw.HardwareProductId()) else {
            continue;
        };
        if ctx.is_claimed(vid, pid) || !matches!(vid, MICROSOFT | GAMESIR) {
            continue;
        }
        let Ok(report) = raw.TryGetBatteryReport() else {
            continue;
        };
        let value = |r: windows::core::Result<windows::Foundation::IReference<i32>>| {
            r.and_then(|v| v.Value()).unwrap_or(0)
        };
        let remaining = value(report.RemainingCapacityInMilliwattHours());
        let full = value(report.FullChargeCapacityInMilliwattHours());
        let status = report.Status().unwrap_or(BatteryStatus::NotPresent);
        if full <= 0 || status == BatteryStatus::NotPresent {
            continue;
        }
        // Third-party pads show up as a generic Xbox controller.
        let name = if vid == GAMESIR {
            "GameSir controller".to_string()
        } else {
            raw.DisplayName()
                .map(|n| n.to_string())
                .ok()
                .filter(|n| !n.is_empty())
                .unwrap_or("Xbox controller".into())
        };
        let b = Battery {
            key: ctx.stable_key(&format!("xbox-{vid:04x}:{pid:04x}")),
            name,
            via: "Xbox Wireless".into(),
            kind: Kind::Gamepad,
            percent: Some(clamp100((remaining as i64 * 100 / full as i64) as i32)),
            charging: status == BatteryStatus::Charging,
            ..Default::default()
        };
        ctx.add(b, vid, pid);
        added += 1;
        if vid != MICROSOFT {
            third += 1;
        }
    }
    (added, third)
}

/// Capabilities plus the pad's USB ids, from xinput1_4's unnamed export 108.
#[repr(C)]
#[derive(Default)]
struct CapsEx {
    caps: XINPUT_CAPABILITIES,
    vid: u16,
    pid: u16,
    version: u16,
    unk1: u16,
    unk2: u32,
}

type CapsExFn = unsafe extern "system" fn(u32, u32, u32, *mut CapsEx) -> u32;

fn caps_ex() -> Option<CapsExFn> {
    static F: OnceLock<Option<CapsExFn>> = OnceLock::new();
    *F.get_or_init(|| unsafe {
        let m = LoadLibraryExW(
            wide("xinput1_4.dll").as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if m.is_null() {
            return None;
        }
        GetProcAddress(m, 108 as *const u8).map(|f| std::mem::transmute::<_, CapsExFn>(f))
    })
}

/// Coarse XInput level as a percentage and its name.
fn coarse(level: u8) -> (u8, &'static str) {
    match level {
        BATTERY_LEVEL_EMPTY => (5, "Empty"),
        BATTERY_LEVEL_LOW => (25, "Low"),
        BATTERY_LEVEL_MEDIUM => (60, "Medium"),
        _ => (100, "Full"),
    }
}

fn battery(user: u32, ty: BATTERY_DEVTYPE) -> Option<XINPUT_BATTERY_INFORMATION> {
    let mut bi = XINPUT_BATTERY_INFORMATION::default();
    let ok = unsafe { XInputGetBatteryInformation(user, ty, &mut bi) } == ERROR_SUCCESS;
    (ok && bi.BatteryType != BATTERY_TYPE_DISCONNECTED && bi.BatteryType != BATTERY_TYPE_WIRED)
        .then_some(bi)
}

pub fn poll_x_input(ctx: &mut Ctx) {
    let bt_xbox = ctx
        .out
        .iter()
        .any(|b| b.via == "Bluetooth" && b.name.contains("Xbox"));
    let (precise, third) = if bt_xbox {
        (0, 0)
    } else {
        poll_gaming_input(ctx)
    };
    // GameSir pads answer the headset query with their own level even with nothing plugged in.
    let skip_headset = precise > 0 && third == precise;
    for user in 0..4u32 {
        // The wireless adapter answers for empty slots too: only look at slots with a pad.
        let mut state = XINPUT_STATE::default();
        if unsafe { XInputGetState(user, &mut state) } != ERROR_SUCCESS {
            continue;
        }
        if let Some(f) = caps_ex() {
            let mut cx = CapsEx::default();
            if unsafe { f(1, user, 0, &mut cx) } == ERROR_SUCCESS
                && cx.vid != 0
                && ctx.is_claimed(cx.vid, cx.pid)
            {
                continue;
            }
        }
        let pad = battery(user, BATTERY_DEVTYPE_GAMEPAD);
        let mut found = Vec::new();
        if !bt_xbox
            && precise == 0
            && let Some(p) = pad
        {
            found.push((BATTERY_DEVTYPE_GAMEPAD, p));
        }
        if !skip_headset
            && let Some(h) = battery(user, BATTERY_DEVTYPE_HEADSET)
            && h.BatteryType != BATTERY_TYPE_UNKNOWN
            // Xbox One and Series pads echo their own level for the headset query.
            && !pad.is_some_and(|p| p.BatteryType == h.BatteryType && p.BatteryLevel == h.BatteryLevel)
        {
            found.push((BATTERY_DEVTYPE_HEADSET, h));
        }
        for (ty, bi) in found {
            let headset = ty == BATTERY_DEVTYPE_HEADSET;
            let (pct, level) = coarse(bi.BatteryLevel);
            let b = Battery {
                key: ctx.stable_key(&format!(
                    "xinput-{}{}",
                    user + 1,
                    if headset { "-headset" } else { "" }
                )),
                name: if headset {
                    "Xbox headset".into()
                } else {
                    format!("Xbox controller {}", user + 1)
                },
                via: "Xbox Wireless".into(),
                kind: if headset {
                    Kind::Headset
                } else {
                    Kind::Gamepad
                },
                percent: Some(pct),
                level: level.into(),
                ..Default::default()
            };
            ctx.out.push(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_levels() {
        assert_eq!(coarse(BATTERY_LEVEL_EMPTY), (5, "Empty"));
        assert_eq!(coarse(BATTERY_LEVEL_MEDIUM), (60, "Medium"));
        assert_eq!(coarse(3), (100, "Full"));
    }
}
