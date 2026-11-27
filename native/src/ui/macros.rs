use std::cell::RefCell;
use std::time::Duration;

use foxden_macros::{Entry, Hotkey, Input, Macro, MediaKey, Mode, Step, keys, model};
use slint::{ComponentHandle, Timer, TimerMode};
use tauri::AppHandle;

use super::{AppWindow, MacroItem, MacroStep, MacrosPage, View, fill, tr, with_win};
use crate::macros::{self, Macros};

const TAB: i32 = 6;

#[derive(Default)]
struct Ui {
    selected: String,
    section: usize,
    catch_for: &'static str,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
    static POLL: Timer = Timer::default();
}

fn store(app: &AppHandle) -> Option<tauri::State<'static, Macros>> {
    macros::get(app).ok()
}

fn section(m: &mut Macro, i: usize) -> &mut Vec<Step> {
    match (m.mode, i) {
        (Mode::PressHoldRelease, 1) => &mut m.hold,
        (Mode::PressHoldRelease, 2) => &mut m.release,
        _ => &mut m.press,
    }
}

fn row(s: &Step) -> MacroStep {
    let (kind, value) = match s {
        Step::Delay { ms } => ("delay", ms.to_string()),
        Step::Text { text } => ("text", text.clone()),
        _ => ("", String::new()),
    };
    MacroStep { text: s.describe().into(), kind: kind.into(), value: value.into() }
}

/// A chord as steps: modifiers down, the input down and up, modifiers up.
pub fn chord_steps(h: Hotkey) -> Vec<Step> {
    let mods: Vec<u16> = [(model::CTRL, 0xE0), (model::SHIFT, 0xE1), (model::ALT, 0xE2), (model::WIN, 0xE3)]
        .into_iter()
        .filter(|(b, _)| h.mods & b != 0)
        .map(|(_, u)| u)
        .collect();
    let mut out: Vec<Step> = mods.iter().map(|&usage| Step::Key { usage, down: true }).collect();
    match h.input {
        Input::Key(usage) => out.extend([Step::Key { usage, down: true }, Step::Key { usage, down: false }]),
        Input::Button(button) => out.extend([Step::Button { button, down: true }, Step::Button { button, down: false }]),
    }
    out.extend(mods.iter().rev().map(|&usage| Step::Key { usage, down: false }));
    out
}

/// Drops the release of the click on Record and the waits before the first real step.
pub fn trim_start(mut steps: Vec<Step>) -> Vec<Step> {
    let skip = steps
        .iter()
        .take_while(|s| matches!(s, Step::Delay { .. } | Step::Button { down: false, .. } | Step::Key { down: false, .. }))
        .count();
    steps.drain(..skip);
    steps
}

pub fn apply(w: &AppWindow, app: &AppHandle, _view: &mut View) {
    refresh(w, app);
}

pub fn event(w: &AppWindow, app: &AppHandle, _view: &mut View, name: &str) {
    if name == "macros" {
        refresh(w, app);
    }
}

/// The list and what is running, recording or waiting for a key; leaves fields being typed in alone.
fn status(w: &AppWindow, m: &Macros) -> foxden_macros::Library {
    let g = w.global::<MacrosPage>();
    let lib = m.library();
    let running = m.host().running();
    let items: Vec<MacroItem> = lib
        .entries
        .iter()
        .map(|e| MacroItem {
            id: e.id.as_str().into(),
            name: e.macro_.name.as_str().into(),
            hotkey: e.hotkey.map(|h| h.label()).unwrap_or_default().into(),
            running: running.contains(&e.id),
        })
        .collect();
    fill(g.get_macros(), items, |x| g.set_macros(x));
    g.set_enabled(lib.enabled);
    g.set_warned(lib.noted);
    g.set_playing(!running.is_empty());
    g.set_recording(m.host().is_recording());
    let catching = m.host().is_catching();
    g.set_catching(catching);
    lib
}

fn refresh(w: &AppWindow, app: &AppHandle) {
    let Some(m) = store(app) else { return };
    let g = w.global::<MacrosPage>();
    let lib = status(w, &m);
    let (selected, sec, catch_for) = UI.with_borrow_mut(|u| {
        if lib.get(&u.selected).is_none() {
            u.selected.clear();
        }
        (u.selected.clone(), u.section, u.catch_for)
    });
    g.set_catch_for(catch_for.into());
    g.set_selected(selected.as_str().into());
    let Some(e) = lib.get(&selected) else { return };
    if g.get_name().as_str() != e.macro_.name && g.get_problem().is_empty() {
        g.set_name(e.macro_.name.as_str().into());
    }
    g.set_mode(Mode::ALL.iter().position(|x| *x == e.macro_.mode).unwrap_or(0) as i32);
    g.set_hotkey(e.hotkey.map(|h| h.label()).unwrap_or_default().into());
    g.set_block(e.block);
    g.set_default_delay(e.macro_.default_delay_ms.map(|d| d.to_string()).unwrap_or_default().into());
    g.set_section(sec as i32);
    let mut copy = e.macro_.clone();
    let rows: Vec<MacroStep> = section(&mut copy, sec).iter().map(row).collect();
    fill(g.get_steps(), rows, |x| g.set_steps(x));
}

fn redraw(app: &AppHandle) {
    let a = app.clone();
    with_win(|w| refresh(w, &a));
}

fn problem(app: &AppHandle, r: Result<(), String>) {
    let text = r.err().map(|e| tr(app, &e)).unwrap_or_default();
    with_win(|w| w.global::<MacrosPage>().set_problem(text.into()));
    redraw(app);
}

fn selected() -> String {
    UI.with_borrow(|u| u.selected.clone())
}

/// Changes the selected macro and saves it.
fn edit(app: &AppHandle, f: impl FnOnce(&mut Entry)) {
    let Some(m) = store(app) else { return };
    let id = selected();
    problem(app, m.edit(&id, f));
}

fn edit_steps(app: &AppHandle, f: impl FnOnce(&mut Vec<Step>)) {
    let sec = UI.with_borrow(|u| u.section);
    edit(app, |e| f(section(&mut e.macro_, sec)));
}

fn add(app: &AppHandle, steps: Vec<Step>) {
    edit_steps(app, |s| s.extend(steps));
}

fn new_name(m: &Macros, base: &str) -> String {
    let lib = m.library();
    (1..)
        .map(|i| if i == 1 { base.to_string() } else { format!("{base} {i}") })
        .find(|n| !lib.entries.iter().any(|e| e.macro_.name.eq_ignore_ascii_case(n)))
        .unwrap_or_default()
}

fn catch(app: &AppHandle, what: &'static str) {
    let Some(m) = store(app) else { return };
    UI.with_borrow_mut(|u| u.catch_for = what);
    let a = app.clone();
    let r = m.catch_hotkey(move |h| {
        let _ = slint::invoke_from_event_loop(move || {
            UI.with_borrow_mut(|u| u.catch_for = "");
            match (what, h) {
                ("hotkey", Some(h)) => edit(&a, |e| e.hotkey = Some(h)),
                ("step", Some(h)) => add(&a, chord_steps(h)),
                _ => redraw(&a),
            }
        });
    });
    problem(app, r);
}

pub fn bind(w: &AppWindow, app: &AppHandle) {
    let g = w.global::<MacrosPage>();
    let modes: Vec<slint::SharedString> = Mode::ALL.iter().map(|m| tr(app, m.label()).into()).collect();
    g.set_modes(slint::ModelRc::new(slint::VecModel::from(modes)));
    let buttons: Vec<slint::SharedString> = (1..=5).map(|b| tr(app, model::button_name(b)).into()).collect();
    g.set_buttons(slint::ModelRc::new(slint::VecModel::from(buttons)));
    let media: Vec<slint::SharedString> = MediaKey::ALL.iter().map(|k| tr(app, k.label()).into()).collect();
    g.set_media(slint::ModelRc::new(slint::VecModel::from(media)));

    let a = app.clone();
    g.on_set_enabled(move |on| {
        if let Some(m) = store(&a) {
            problem(&a, m.set_enabled(on));
        }
    });
    let a = app.clone();
    g.on_dismiss_note(move || {
        if let Some(m) = store(&a) {
            problem(&a, m.note_read());
        }
    });
    let a = app.clone();
    g.on_new_macro(move || {
        let Some(m) = store(&a) else { return };
        let name = new_name(&m, &tr(&a, "New macro"));
        let entry = Entry { id: String::new(), macro_: Macro { name, ..Default::default() }, hotkey: None, block: false };
        match m.put(entry) {
            Ok(id) => {
                UI.with_borrow_mut(|u| {
                    u.selected = id;
                    u.section = 0;
                });
                with_win(|w| w.global::<MacrosPage>().set_name("".into()));
                problem(&a, Ok(()));
            }
            Err(e) => problem(&a, Err(e)),
        }
    });
    let a = app.clone();
    g.on_select(move |id| {
        UI.with_borrow_mut(|u| {
            u.selected = id.to_string();
            u.section = 0;
        });
        with_win(|w| w.global::<MacrosPage>().set_name("".into()));
        problem(&a, Ok(()));
    });
    let a = app.clone();
    g.on_rename(move |name| {
        let name = name.trim().to_string();
        with_win(|w| w.global::<MacrosPage>().set_problem("".into()));
        edit(&a, |e| e.macro_.name = name);
    });
    let a = app.clone();
    g.on_set_mode(move |i| {
        UI.with_borrow_mut(|u| u.section = 0);
        edit(&a, |e| e.macro_.mode = Mode::ALL[(i.max(0) as usize).min(3)]);
    });
    let a = app.clone();
    g.on_catch_hotkey(move || catch(&a, "hotkey"));
    let a = app.clone();
    g.on_catch_step(move || catch(&a, "step"));
    let a = app.clone();
    g.on_cancel_catch(move || {
        if let Some(m) = store(&a) {
            m.cancel_catch();
        }
    });
    let a = app.clone();
    g.on_clear_hotkey(move || edit(&a, |e| e.hotkey = None));
    let a = app.clone();
    g.on_set_block(move |on| edit(&a, |e| e.block = on));
    let a = app.clone();
    g.on_set_default_delay(move |t| {
        let t = t.trim().to_string();
        if t.is_empty() {
            return edit(&a, |e| e.macro_.default_delay_ms = None);
        }
        match t.parse::<u32>() {
            Ok(ms) => edit(&a, |e| e.macro_.default_delay_ms = (ms > 0).then_some(ms)),
            Err(_) => problem(&a, Err("Enter the wait in milliseconds.".into())),
        }
    });
    let a = app.clone();
    g.on_set_section(move |i| {
        UI.with_borrow_mut(|u| u.section = i.clamp(0, 2) as usize);
        redraw(&a);
    });
    let a = app.clone();
    g.on_add_key(move |name| match keys::by_name(&name) {
        Some(usage) => add(&a, vec![Step::Key { usage, down: true }, Step::Key { usage, down: false }]),
        None => problem(&a, Err(tr(&a, "FoxDen doesn't know that key."))),
    });
    let a = app.clone();
    g.on_add_text(move |text| {
        if !text.is_empty() {
            add(&a, vec![Step::Text { text: text.to_string() }]);
        }
    });
    let a = app.clone();
    g.on_add_delay(move |t| match t.trim().parse::<u32>() {
        Ok(ms) => add(&a, vec![Step::Delay { ms }]),
        Err(_) => problem(&a, Err("Enter the wait in milliseconds.".into())),
    });
    let a = app.clone();
    g.on_add_button(move |b| {
        let button = b.clamp(1, 5) as u8;
        add(&a, vec![Step::Button { button, down: true }, Step::Button { button, down: false }]);
    });
    let a = app.clone();
    g.on_add_wheel(move |ticks| add(&a, vec![Step::Wheel { horizontal: false, ticks }]));
    let a = app.clone();
    g.on_add_media(move |i| add(&a, vec![Step::Media { key: MediaKey::ALL[(i.max(0) as usize).min(6)] }]));
    let a = app.clone();
    g.on_edit_step(move |i, value| {
        let i = i.max(0) as usize;
        let value = value.to_string();
        let parsed = value.trim().parse::<u32>();
        edit_steps(&a, |s| match s.get_mut(i) {
            Some(Step::Delay { ms }) => {
                if let Ok(v) = parsed {
                    *ms = v;
                }
            }
            Some(Step::Text { text }) => *text = value,
            _ => {}
        });
    });
    let a = app.clone();
    g.on_move_step(move |i, by| {
        edit_steps(&a, |s| {
            let i = i.max(0) as usize;
            let j = i as i64 + by as i64;
            if i < s.len() && (0..s.len() as i64).contains(&j) {
                s.swap(i, j as usize);
            }
        })
    });
    let a = app.clone();
    g.on_remove_step(move |i| {
        edit_steps(&a, |s| {
            if (i.max(0) as usize) < s.len() {
                s.remove(i as usize);
            }
        })
    });
    let a = app.clone();
    g.on_clear_steps(move || edit_steps(&a, Vec::clear));
    let a = app.clone();
    g.on_record(move || {
        if let Some(m) = store(&a) {
            problem(&a, m.start_recording());
        }
    });
    let a = app.clone();
    g.on_stop_record(move || {
        let Some(m) = store(&a) else { return };
        let steps = trim_start(m.stop_recording());
        if steps.is_empty() {
            return redraw(&a);
        }
        add(&a, steps);
    });
    let a = app.clone();
    g.on_play(move || {
        let Some(m) = store(&a) else { return };
        problem(&a, m.play(&selected(), Duration::from_secs(3)));
    });
    let a = app.clone();
    g.on_stop_all(move || {
        if let Some(m) = store(&a) {
            m.stop_all();
        }
        redraw(&a);
    });
    let a = app.clone();
    g.on_delete(move || {
        let Some(m) = store(&a) else { return };
        let r = m.remove(&selected()).map(|_| ());
        UI.with_borrow_mut(|u| u.selected.clear());
        problem(&a, r);
    });

    let weak = w.as_weak();
    let a = app.clone();
    POLL.with(|t| {
        t.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            if let Some(w) = weak.upgrade().filter(|w| w.get_tab() == TAB && w.window().is_visible())
                && let Some(m) = store(&a)
            {
                status(&w, &m);
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_wrap_the_input_in_its_modifiers() {
        let h = Hotkey { input: Input::Key(0x06), mods: model::CTRL | model::SHIFT };
        let s = chord_steps(h);
        assert_eq!(s.len(), 6);
        assert_eq!(s[0], Step::Key { usage: 0xE0, down: true });
        assert_eq!(s[2], Step::Key { usage: 0x06, down: true });
        assert_eq!(s[5], Step::Key { usage: 0xE0, down: false });
        let click = chord_steps(Hotkey { input: Input::Button(4), mods: 0 });
        assert_eq!(click, vec![Step::Button { button: 4, down: true }, Step::Button { button: 4, down: false }]);
    }

    #[test]
    fn recordings_lose_the_record_click() {
        let s = vec![
            Step::Button { button: 1, down: false },
            Step::Delay { ms: 300 },
            Step::Key { usage: 4, down: true },
            Step::Delay { ms: 20 },
            Step::Key { usage: 4, down: false },
        ];
        assert_eq!(trim_start(s).len(), 3);
        assert!(trim_start(vec![Step::Delay { ms: 1 }]).is_empty());
    }

    #[test]
    fn sections_follow_the_mode() {
        let mut m = Macro { mode: Mode::PressHoldRelease, ..Default::default() };
        section(&mut m, 2).push(Step::Delay { ms: 1 });
        assert_eq!(m.release.len(), 1);
        m.mode = Mode::Toggle;
        section(&mut m, 2).push(Step::Delay { ms: 1 });
        assert_eq!(m.press.len(), 1);
    }
}
