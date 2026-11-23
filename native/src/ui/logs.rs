//! The log window: every entry kept in memory, filtered by level and text, following new ones.

use std::cell::RefCell;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};
use tauri::AppHandle;

use super::{I18n, LogRow, LogsPage, LogsWindow, Theme, fill, light_theme, tr};
use crate::log::{Entry, Level};

struct Open {
    win: LogsWindow,
    _poll: Timer,
    seen: (usize, u64),
    entries: Vec<Entry>,
}

thread_local! {
    static LOGS: RefCell<Option<Open>> = const { RefCell::new(None) };
}

fn rank(level: Level) -> i32 {
    match level {
        Level::Info => 0,
        Level::Warn => 1,
        Level::Error => 2,
    }
}

fn short(level: Level) -> &'static str {
    match level {
        Level::Info => "INFO",
        Level::Warn => "WARN",
        Level::Error => "ERR",
    }
}

/// Minutes to add to UTC for local time, from the clock now.
fn offset_ms() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    let (mut local, mut utc): (SYSTEMTIME, SYSTEMTIME) = unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
    unsafe {
        GetLocalTime(&mut local);
        GetSystemTime(&mut utc);
    }
    let ms = |t: &SYSTEMTIME| (t.wHour as i64 * 60 + t.wMinute as i64) * 60_000;
    let mut diff = ms(&local) - ms(&utc);
    if local.wDay != utc.wDay {
        diff += if (local.wYear, local.wMonth, local.wDay) > (utc.wYear, utc.wMonth, utc.wDay) {
            86_400_000
        } else {
            -86_400_000
        };
    }
    diff
}

fn time(at: u64, offset: i64) -> String {
    let secs = (at as i64 + offset).rem_euclid(86_400_000) / 1000;
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

fn shown(o: &Open) -> Vec<&Entry> {
    let g = o.win.global::<LogsPage>();
    let least = g.get_level();
    let text = g.get_filter().trim().to_lowercase();
    o.entries
        .iter()
        .filter(|e| rank(e.level) >= least)
        .filter(|e| text.is_empty() || format!("{} {}", e.source, e.text).to_lowercase().contains(&text))
        .collect()
}

fn render(app: &AppHandle, o: &Open) {
    let g = o.win.global::<LogsPage>();
    let offset = offset_ms();
    let rows: Vec<LogRow> = shown(o)
        .into_iter()
        .map(|e| LogRow {
            at: time(e.at, offset).into(),
            level: short(e.level).into(),
            source: e.source.as_str().into(),
            text: e.text.as_str().into(),
        })
        .collect();
    let total = o.entries.len();
    let count = if rows.len() == total {
        format!("{total} entries")
    } else {
        format!("{} of {total}", rows.len())
    };
    g.set_count(tr(app, &count).into());
    g.set_empty(rows.is_empty());
    fill(g.get_rows(), rows, |m| g.set_rows(m));
}

fn reload(app: &AppHandle) {
    LOGS.with_borrow_mut(|slot| {
        let Some(o) = slot else { return };
        let entries = crate::log::get_logs();
        let seen = (entries.len(), entries.last().map_or(0, |e| e.at));
        if seen == o.seen {
            return;
        }
        o.seen = seen;
        o.entries = entries;
        render(app, o);
    });
}

fn plain(e: &Entry, offset: i64) -> String {
    format!("{} {} {}: {}", time(e.at, offset), short(e.level).to_lowercase(), e.source, e.text)
}

fn copy_text(text: &str) -> bool {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        let mut ok = false;
        if !mem.is_null() {
            let to = GlobalLock(mem) as *mut u16;
            if !to.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), to, wide.len());
                GlobalUnlock(mem);
                ok = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
            }
        }
        CloseClipboard();
        ok
    }
}

/// Follows a language change while the window is open.
pub fn set_lang(app: &AppHandle, lang: &str) {
    LOGS.with_borrow(|slot| {
        let Some(o) = slot else { return };
        let i18n = o.win.global::<I18n>();
        if i18n.get_lang() != lang {
            i18n.set_lang(lang.into());
            render(app, o);
        }
    });
}

pub fn open(app: &AppHandle) {
    let shown_already = LOGS.with_borrow(|slot| {
        slot.as_ref().is_some_and(|o| {
            let _ = o.win.show();
            o.win.window().set_minimized(false);
            true
        })
    });
    if shown_already {
        return;
    }
    let win = match LogsWindow::new() {
        Ok(win) => win,
        Err(e) => {
            crate::log::warn("app", format!("the log window could not open: {e}"));
            return;
        }
    };
    win.global::<Theme>().set_light(light_theme());
    let lang = crate::i18n::language(&crate::state::lock(&super::shared(app)).config.settings.language);
    win.global::<I18n>().set_lang(lang.into());
    let a = app.clone();
    win.global::<I18n>().on_tr(move |_, text| tr(&a, &text).into());

    let g = win.global::<LogsPage>();
    let a = app.clone();
    g.on_refilter(move || {
        LOGS.with_borrow(|slot| {
            if let Some(o) = slot {
                render(&a, o);
            }
        })
    });
    let a = app.clone();
    g.on_copy(move || {
        LOGS.with_borrow(|slot| {
            let Some(o) = slot else { return };
            let offset = offset_ms();
            let text: Vec<String> = shown(o).into_iter().map(|e| plain(e, offset)).collect();
            if copy_text(&text.join("\n")) {
                o.win.global::<LogsPage>().set_count(tr(&a, "Copied").into());
            }
        })
    });
    let a = app.clone();
    g.on_clear(move || {
        crate::log::clear_logs();
        LOGS.with_borrow_mut(|slot| {
            if let Some(o) = slot {
                o.entries.clear();
                o.seen = (0, 0);
                render(&a, o);
            }
        })
    });
    win.window().on_close_requested(|| {
        Timer::single_shot(Duration::ZERO, || {
            LOGS.with_borrow_mut(|slot| slot.take());
        });
        slint::CloseRequestResponse::HideWindow
    });

    let poll = Timer::default();
    let a = app.clone();
    poll.start(TimerMode::Repeated, Duration::from_millis(300), move || reload(&a));
    win.window().set_size(slint::LogicalSize::new(760.0, 480.0));
    let _ = win.show();
    LOGS.with_borrow_mut(|slot| {
        *slot = Some(Open {
            win,
            _poll: poll,
            seen: (usize::MAX, 0),
            entries: Vec::new(),
        })
    });
    reload(app);
}
