//! The sheet where the person at the PC approves or rejects fan curves an AI assistant proposed.

use std::cell::Cell;
use std::time::Duration;

use slint::{ComponentHandle, Timer};
use tauri::AppHandle;

use super::{AppWindow, ApprovalFan, FanApproval, fill, tr, with_win};
use crate::fan_proposal::{Ask, DESK, Decision, Drive, NO_WINDOW};

/// A short pause before Apply works, so a click meant for another window does not approve.
const ARM: Duration = Duration::from_millis(800);
const GRAPH_W: f32 = 160.0;
pub const SAVE_MACRO: &str = "{0} wants to save a macro";
pub const PLAY_MACRO: &str = "{0} wants to play a macro";
pub const MACROS_ON: &str = "{0} wants to turn on macro hotkeys";
pub const MOUSE_MACRO: &str = "{0} wants to change a button in your mouse's memory";
pub const RECORD_MIC: &str = "{0} wants to record your microphone";
pub const CLOSE_APPS: &str = "{0} wants to close conflicting software";
const GRAPH_H: f32 = 80.0;

thread_local! {
    static SHOWING: Cell<Option<u64>> = const { Cell::new(None) };
}

pub fn bind(w: &AppWindow) {
    let g = w.global::<FanApproval>();
    g.on_apply(|| answer(Decision::Apply));
    g.on_reject(|| answer(Decision::Reject));
}

fn answer(decision: Decision) {
    if let Some(id) = SHOWING.take() {
        DESK.decide(id, decision);
    }
    hide();
}

fn hide() {
    with_win(|w| {
        let g = w.global::<FanApproval>();
        g.set_open(false);
        g.set_armed(false);
    });
}

/// Closing the window answers no.
pub fn window_closing() {
    if let Some(id) = SHOWING.take() {
        DESK.decide(id, Decision::Reject);
    }
}

/// Closes the sheet of proposal `id` when it is still open.
pub fn close(id: u64) {
    if SHOWING.get() == Some(id) {
        SHOWING.set(None);
        hide();
    }
}

/// True while a proposal waits on screen, when other prompts must not take the focus.
pub fn busy() -> bool {
    SHOWING.get().is_some()
}

/// Opens the window, or brings it up from the tray, with the sheet on top.
pub fn open(app: &AppHandle, ask: Ask) -> Result<(), String> {
    SHOWING.set(Some(ask.id));
    super::show(app);
    let mut shown = false;
    with_win(|w| {
        let g = w.global::<FanApproval>();
        let who = if ask.client.is_empty() { tr(app, "An AI assistant") } else { ask.client.clone() };
        let title = match &ask.other {
            Some((title, _)) => title,
            None if ask.settings.is_empty() => "{0} wants to change your fan curves",
            None => "{0} wants to change FoxDen's settings",
        };
        g.set_title(tr(app, title).replace("{0}", &who).into());
        let mut lines: Vec<slint::SharedString> = ask
            .settings
            .iter()
            .map(|&(label, on)| format!("{}: {}", tr(app, label), tr(app, if on { "On" } else { "Off" })).into())
            .collect();
        lines.extend(ask.other.iter().flat_map(|(_, l)| l.iter().map(|t| t.as_str().into())));
        fill(g.get_lines(), lines, |m| g.set_lines(m));
        g.set_reason(ask.reason.clone().into());
        let fans: Vec<ApprovalFan> = ask
            .diff
            .changed
            .iter()
            .map(|c| ApprovalFan {
                name: tr(app, &c.name).into(),
                before: tr(app, "Before: {0}").replace("{0}", &describe(app, c.before.as_ref())).into(),
                after: tr(app, "After: {0}").replace("{0}", &describe(app, c.after.as_ref())).into(),
                before_path: path(c.before.as_ref()).into(),
                after_path: path(c.after.as_ref()).into(),
            })
            .collect();
        fill(g.get_fans(), fans, |m| g.set_fans(m));
        let unchanged = match ask.diff.unchanged {
            0 => String::new(),
            1 => tr(app, "1 fan unchanged"),
            n => tr(app, "{0} fans unchanged").replace("{0}", &n.to_string()),
        };
        g.set_unchanged(unchanged.into());
        let loose = if ask.diff.loose { tr(app, "Curves no fan follows change too.") } else { String::new() };
        g.set_loose(loose.into());
        g.set_armed(false);
        g.set_open(true);
        shown = true;
    });
    if !shown {
        SHOWING.set(None);
        return Err(NO_WINDOW.into());
    }
    let id = ask.id;
    Timer::single_shot(ARM, move || {
        if SHOWING.get() == Some(id) {
            with_win(|w| w.global::<FanApproval>().set_armed(true));
        }
    });
    // The window may only exist once the event loop has run.
    Timer::single_shot(Duration::from_millis(50), || bring_to_front(20));
    Ok(())
}

fn describe(app: &AppHandle, drive: Option<&Drive>) -> String {
    let Some(d) = drive else {
        return tr(app, "Not controlled");
    };
    let points: Vec<String> = d
        .points
        .iter()
        .map(|&(t, duty)| format!("{} °C {}%", trim(t), (duty * 100.0).round() as i64))
        .collect();
    let limits = tr(app, "duty {0} to {1}%")
        .replace("{0}", &trim(d.min * 100.0))
        .replace("{1}", &trim(d.max * 100.0));
    format!("\"{}\" ({}), {limits}", d.curve, points.join(", "))
}

fn trim(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r:.1}") }
}

/// The curve as a path in a `GRAPH_W` by `GRAPH_H` box, 20 to 100 °C across.
fn path(drive: Option<&Drive>) -> String {
    let Some(points) = drive.map(|d| &d.points).filter(|p| !p.is_empty()) else {
        return String::new();
    };
    let x = |t: f32| 4.0 + (t.clamp(20.0, 100.0) - 20.0) / 80.0 * (GRAPH_W - 8.0);
    let y = |d: f32| GRAPH_H - 4.0 - d.clamp(0.0, 1.0) * (GRAPH_H - 8.0);
    let mut line = vec![(4.0, y(points[0].1))];
    line.extend(points.iter().map(|&(t, d)| (x(t), y(d))));
    line.push((GRAPH_W - 4.0, y(points[points.len() - 1].1)));
    line.iter()
        .enumerate()
        .map(|(i, p)| format!("{} {:.1} {:.1}", if i == 0 { "M" } else { "L" }, p.0, p.1))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Raises and focuses the window, trying again while a window just opened is not up yet.
/// Windows may still refuse focus, so it flashes then.
fn bring_to_front(tries: u32) {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx,
        GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsIconic,
        IsWindowVisible, SW_RESTORE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos,
        ShowWindow,
    };

    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> windows_sys::core::BOOL {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != GetCurrentProcessId() || IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut buf = [0u16; 16];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            if String::from_utf16_lossy(&buf[..n]) == "FoxDen" {
                *(found as *mut HWND) = hwnd;
                return 0;
            }
        }
        1
    }

    let mut hwnd: HWND = null_mut();
    unsafe {
        EnumWindows(Some(each), &mut hwnd as *mut HWND as LPARAM);
        if hwnd.is_null() {
            if tries > 0 {
                Timer::single_shot(Duration::from_millis(100), move || bring_to_front(tries - 1));
            }
            return;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
        SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
        // Sharing input with the window in front lets Windows hand focus over.
        let front = GetWindowThreadProcessId(GetForegroundWindow(), null_mut());
        let me = GetCurrentThreadId();
        let joined = front != 0 && front != me && AttachThreadInput(me, front, 1) != 0;
        BringWindowToTop(hwnd);
        let focused = SetForegroundWindow(hwnd) != 0;
        if joined {
            AttachThreadInput(me, front, 0);
        }
        if !focused {
            let info = FLASHWINFO {
                cbSize: size_of::<FLASHWINFO>() as u32,
                hwnd,
                dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                uCount: 0,
                dwTimeout: 0,
            };
            FlashWindowEx(&info);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_draw_inside_the_box_across_the_whole_width() {
        let d = Drive { curve: "c".into(), points: vec![(20.0, 0.0), (100.0, 1.0)], min: 0.0, max: 1.0 };
        assert_eq!(path(Some(&d)), "M 4.0 76.0 L 4.0 76.0 L 156.0 4.0 L 156.0 4.0");
        assert_eq!(path(None), "");
    }
}
