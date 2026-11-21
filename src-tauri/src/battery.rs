//! A tray icon per wireless device, showing its battery percentage.

use std::cell::RefCell;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use foxden_battery::Battery;
use tauri::AppHandle;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::state::{Shared, lock};

const EVERY: Duration = Duration::from_secs(20);
const TICK: Duration = Duration::from_secs(2);
const PREFIX: &str = "battery:";

thread_local! {
    /// The battery icons, owned by the main thread.
    static ICONS: RefCell<HashMap<String, TrayIcon>> = RefCell::new(HashMap::new());
}

/// What a tray icon currently shows.
#[derive(Clone, PartialEq)]
struct Shown {
    tip: String,
    text: String,
    bar: Option<u8>,
    tint: Tint,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tint {
    Normal,
    Charging,
    Low,
    Critical,
}

pub fn spawn(app: AppHandle, shared: Shared) {
    app.on_tray_icon_event(|app, event| {
        if let TrayIconEvent::Click {
            id,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
            && id.as_ref().starts_with(PREFIX)
        {
            crate::tray::show_window(app);
        }
    });
    thread::spawn(move || {
        let mut known: HashMap<String, (Battery, Instant)> = HashMap::new();
        let mut shown: HashMap<String, Shown> = HashMap::new();
        let mut last: Option<Instant> = None;
        let mut plugged = foxden_battery::hid::fingerprint();
        loop {
            let on = lock(&shared).config.settings.battery_tray;
            // A cable plugged in or pulled out shows up as a changed HID list: look again right away.
            let now_plugged = foxden_battery::hid::fingerprint();
            let replugged = std::mem::replace(&mut plugged, now_plugged) != plugged;
            if !on {
                known.clear();
                last = None;
                sync(&app, &mut shown, HashMap::new());
            } else if replugged || last.is_none_or(|t| t.elapsed() >= EVERY) {
                let now = Instant::now();
                let polled = foxden_battery::poll();
                merge(&mut known, polled, now);
                let wanted = known
                    .iter()
                    .map(|(k, (b, _))| (k.clone(), show(b)))
                    .collect();
                sync(&app, &mut shown, wanted);
                last = Some(now);
            }
            thread::sleep(TICK);
        }
    });
}

/// Takes in one poll. A sleeping device keeps its last level; one that stops answering stays for its grace time.
fn merge(known: &mut HashMap<String, (Battery, Instant)>, polled: Vec<Battery>, now: Instant) {
    for b in polled.into_iter().filter(|b| b.connected) {
        match known.get_mut(&b.key) {
            Some((old, seen)) if b.asleep || b.percent.is_none() && old.percent.is_some() => {
                old.asleep = b.asleep;
                *seen = now;
            }
            _ => {
                known.insert(b.key.clone(), (b, now));
            }
        }
    }
    known.retain(|_, (b, seen)| {
        let grace = Duration::from_secs(b.grace_secs.into()).max(EVERY * 2);
        now.duration_since(*seen) < grace
    });
}

fn show(b: &Battery) -> Shown {
    let text = match b.percent {
        Some(p) => p.to_string(),
        None if !b.level.is_empty() => b.level.chars().take(3).collect(),
        None => "?".into(),
    };
    let tint = match b.percent {
        _ if b.charging && !b.full => Tint::Charging,
        Some(p) if p <= 10 => Tint::Critical,
        Some(p) if p <= 25 => Tint::Low,
        _ => Tint::Normal,
    };
    let level = match b.percent {
        Some(p) => format!("{p}%"),
        None if !b.level.is_empty() => b.level.clone(),
        None => "level unknown".into(),
    };
    let mut state = Vec::new();
    if b.full {
        state.push("full");
    } else if b.charging {
        state.push("charging");
    }
    if b.asleep {
        state.push("asleep");
    }
    let mut tip = format!("{}: {level}", b.name);
    if !state.is_empty() {
        tip += &format!(", {}", state.join(", "));
    }
    if !b.detail.is_empty() {
        tip += &format!("\n{}", b.detail);
    }
    if !b.via.is_empty() {
        tip += &format!("\n{}", b.via);
    }
    Shown {
        tip: tip.chars().take(127).collect(),
        text,
        bar: b.percent,
        tint,
    }
}

/// Adds, updates and removes tray icons so they match `wanted`.
fn sync(app: &AppHandle, shown: &mut HashMap<String, Shown>, wanted: HashMap<String, Shown>) {
    let gone: Vec<String> = shown
        .keys()
        .filter(|k| !wanted.contains_key(*k))
        .cloned()
        .collect();
    let changed: Vec<(String, Shown)> = wanted
        .into_iter()
        .filter(|(k, s)| shown.get(k) != Some(s))
        .collect();
    if gone.is_empty() && changed.is_empty() {
        return;
    }
    for k in &gone {
        shown.remove(k);
    }
    for (k, s) in &changed {
        shown.insert(k.clone(), s.clone());
    }
    let _ = app.run_on_main_thread(move || {
        ICONS.with_borrow_mut(|icons| {
            for k in gone {
                icons.remove(&k);
            }
            let light = icon::light_taskbar();
            for (k, s) in changed {
                let (rgba, size) = icon::render(&s.text, s.bar, s.tint, light);
                let Ok(image) = Icon::from_rgba(rgba, size, size) else {
                    continue;
                };
                if let Some(tray) = icons.get(&k) {
                    let _ = tray.set_icon(Some(image));
                    let _ = tray.set_tooltip(Some(&s.tip));
                    continue;
                }
                let built = TrayIconBuilder::new()
                    .with_id(format!("{PREFIX}{k}"))
                    .with_icon(image)
                    .with_tooltip(&s.tip)
                    .with_guid(guid(&k))
                    .build();
                match built {
                    Ok(tray) => {
                        icons.insert(k, tray);
                    }
                    Err(e) => crate::log::warn("battery", e.to_string()),
                }
            }
        });
    });
}

/// A fixed identity per device, so Windows remembers whether its icon sits on the taskbar.
/// Windows ties an unsigned app's icon identities to its path, so the path is part of it too.
fn guid(key: &str) -> u128 {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
    for b in exe
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .chain([0])
        .chain(key.bytes())
    {
        h ^= b as u128;
        h = h.wrapping_mul(0x0000000001000000000000000000013B);
    }
    // Version 8 (custom) and the RFC variant, so it is a well-formed GUID.
    (h & !(0xF << 76) & !(0x3 << 62)) | (0x8 << 76) | (0x2 << 62)
}

mod icon {
    use super::Tint;
    use windows_sys::Win32::Graphics::Gdi::{
        ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, FW_BOLD, GdiFlush, OUT_DEFAULT_PRECIS,
        SelectObject, SetBkMode, SetTextCharacterExtra, SetTextColor, TRANSPARENT, TextOutW,
    };
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SM_CXSMICON};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn light_taskbar() -> bool {
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let name = wide("SystemUsesLightTheme");
        let mut v = 0u32;
        let mut size = 4u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut v as *mut u32 as _,
                &mut size,
            )
        };
        ok == 0 && v == 1
    }

    fn icon_size() -> u32 {
        unsafe {
            let tray = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            let dpi = if tray.is_null() {
                0
            } else {
                GetDpiForWindow(tray)
            };
            let dpi = if dpi == 0 { GetDpiForSystem() } else { dpi };
            GetSystemMetricsForDpi(SM_CXSMICON, dpi).clamp(16, 64) as u32
        }
    }

    /// White-on-black glyph coverage of `text` at `px`, in a `w`×`h` buffer.
    fn glyphs(text: &str, px: i32, w: i32, h: i32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h) as usize];
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(dc);
                return out;
            }
            let old = SelectObject(dc, bmp);
            let face = wide("Segoe UI");
            let font = CreateFontW(
                -px,
                0,
                0,
                0,
                FW_BOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                DEFAULT_PITCH as u32,
                face.as_ptr(),
            );
            let old_font = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00FF_FFFF);
            SetTextCharacterExtra(dc, if text.chars().count() >= 3 { -1 } else { 0 });
            let t: Vec<u16> = text.encode_utf16().collect();
            TextOutW(dc, 2, 0, t.as_ptr(), t.len() as i32);
            GdiFlush();
            let px = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
            for (o, p) in out.iter_mut().zip(px) {
                *o = (*p & 0xFF) as u8;
            }
            SelectObject(dc, old_font);
            DeleteObject(font);
            SelectObject(dc, old);
            DeleteObject(bmp);
            DeleteDC(dc);
        }
        out
    }

    fn bounds(mask: &[u8], w: i32) -> Option<(i32, i32, i32, i32)> {
        let mut b: Option<(i32, i32, i32, i32)> = None;
        for (i, &a) in mask.iter().enumerate() {
            if a == 0 {
                continue;
            }
            let (x, y) = (i as i32 % w, i as i32 / w);
            b = Some(match b {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
        b
    }

    /// The icon as straight-alpha RGBA, and its side length.
    pub fn render(text: &str, bar: Option<u8>, tint: Tint, light: bool) -> (Vec<u8>, u32) {
        let s = icon_size() as i32;
        let bar_h = (s / 8).max(2);
        let text_h = s - bar_h - 1;
        let (w, h) = (s * 3, s * 2);
        let mut best = None;
        for px in (6..=text_h * 2).rev() {
            let mask = glyphs(text, px, w, h);
            let Some(b) = bounds(&mask, w) else { break };
            if b.2 - b.0 < s && b.3 - b.1 < text_h {
                best = Some((mask, b));
                break;
            }
        }
        let ink = if light {
            [0u8, 0, 0]
        } else {
            [255u8, 255, 255]
        };
        let fill = match tint {
            Tint::Charging => [0x3D, 0xA5, 0xF4],
            Tint::Critical => [0xE5, 0x39, 0x35],
            Tint::Low => [0xF5, 0xA6, 0x23],
            Tint::Normal => [0x4C, 0xAF, 0x50],
        };
        let mut out = vec![0u8; (s * s * 4) as usize];
        let mut put = |x: i32, y: i32, c: [u8; 3], a: u8| {
            if x < 0 || y < 0 || x >= s || y >= s || a == 0 {
                return;
            }
            let i = ((y * s + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], a]);
        };
        if let Some((mask, (x0, y0, x1, y1))) = best {
            let (gw, gh) = (x1 - x0 + 1, y1 - y0 + 1);
            let (ox, oy) = ((s - gw) / 2, (text_h - gh) / 2);
            let text_ink = if tint == Tint::Critical { fill } else { ink };
            for y in 0..gh {
                for x in 0..gw {
                    put(
                        ox + x,
                        oy + y,
                        text_ink,
                        mask[((y0 + y) * w + x0 + x) as usize],
                    );
                }
            }
        }
        let filled = bar.map_or(0, |p| ((p as i32 * s + 50) / 100).max((p > 0) as i32));
        for y in s - bar_h..s {
            for x in 0..s {
                if x < filled {
                    put(x, y, fill, 255);
                } else {
                    put(x, y, ink, 70);
                }
            }
        }
        (out, s as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(key: &str, percent: Option<u8>) -> Battery {
        Battery {
            key: key.into(),
            name: "Mouse".into(),
            percent,
            ..Default::default()
        }
    }

    #[test]
    fn a_sleeping_device_keeps_its_last_level() {
        let mut known = HashMap::new();
        let t = Instant::now();
        merge(&mut known, vec![device("m", Some(80))], t);
        let asleep = Battery {
            asleep: true,
            percent: None,
            ..device("m", None)
        };
        merge(&mut known, vec![asleep], t + EVERY);
        assert_eq!(known["m"].0.percent, Some(80));
        assert!(known["m"].0.asleep);
    }

    #[test]
    fn a_silent_device_goes_after_its_grace() {
        let mut known = HashMap::new();
        let t = Instant::now();
        merge(&mut known, vec![device("m", Some(50))], t);
        merge(&mut known, vec![], t + EVERY);
        assert!(known.contains_key("m"));
        merge(&mut known, vec![], t + EVERY * 3);
        assert!(known.is_empty());
    }

    #[test]
    fn switched_off_devices_are_not_shown() {
        let mut known = HashMap::new();
        merge(
            &mut known,
            vec![Battery {
                connected: false,
                ..device("m", Some(9))
            }],
            Instant::now(),
        );
        assert!(known.is_empty());
    }

    #[test]
    fn each_device_keeps_one_tray_identity() {
        assert_eq!(guid("logitech-1"), guid("logitech-1"));
        assert_ne!(guid("logitech-1"), guid("logitech-2"));
    }

    #[test]
    fn the_tooltip_says_level_and_state() {
        let b = Battery {
            charging: true,
            via: "USB cable".into(),
            ..device("m", Some(42))
        };
        let s = show(&b);
        assert_eq!(s.text, "42");
        assert_eq!(s.tint, Tint::Charging);
        assert_eq!(s.tip, "Mouse: 42%, charging\nUSB cable");
    }
}
