//! Device priority: FoxDen plays on, and records from, the highest listed device that is really
//! there. A wireless headset's endpoint stays active while only its receiver is plugged in, so a
//! device with a known wireless link must also report that link up.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use foxden_battery::Battery;
use foxden_mixer::system::{self, Device};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::mixer::Mixer;
use crate::state::lock;

const IDLE: Duration = Duration::from_secs(5);
/// Endpoint notifications come in bursts; this lets one burst settle into one look.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PriorityConfig {
    pub on: bool,
    /// Moves the mixer's output.
    pub mixer: bool,
    /// Sets Windows' default playback and recording devices.
    pub windows: bool,
    pub outputs: Vec<Entry>,
    pub inputs: Vec<Entry>,
}

impl Default for PriorityConfig {
    fn default() -> Self {
        PriorityConfig {
            on: false,
            mixer: true,
            windows: false,
            outputs: Vec::new(),
            inputs: Vec::new(),
        }
    }
}

impl PriorityConfig {
    pub fn tidy(mut self) -> Self {
        for list in [&mut self.outputs, &mut self.inputs] {
            let mut seen: Vec<String> = Vec::new();
            list.retain(|e| {
                !e.id.is_empty() && !seen.contains(&e.id) && {
                    seen.push(e.id.clone());
                    true
                }
            });
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Available,
    /// The endpoint is there but its wireless device is switched off.
    Off,
    Unplugged,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    id: String,
    name: String,
    status: Status,
    /// The device in use.
    chosen: bool,
}

/// USB vendor and product id to whether its wireless device is on.
pub type Links = BTreeMap<(u16, u16), bool>;

/// "Headphones (3- Arctis Nova 7)" and "Headphones (Arctis Nova 7)" are one device on another port.
fn plain(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(i) = rest.find('(') {
        out.push_str(&rest[..=i]);
        rest = &rest[i + 1..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with("- ") {
            rest = &rest[digits + 2..];
        }
    }
    out.push_str(rest);
    out
}

/// The device an entry means: by id, else by name, a connected one first.
fn find<'a>(e: &Entry, devices: &'a [Device]) -> Option<&'a Device> {
    let rank = |d: &Device| {
        if d.id == e.id {
            Some(0)
        } else if e.name.is_empty() {
            None
        } else if d.name == e.name {
            Some(1)
        } else if plain(&d.name) == plain(&e.name) {
            Some(2)
        } else {
            None
        }
    };
    devices
        .iter()
        .filter_map(|d| rank(d).map(|r| ((!d.active, r), d)))
        .min_by_key(|(k, _)| *k)
        .map(|(_, d)| d)
}

fn status(d: Option<&Device>, links: &Links) -> Status {
    match d {
        Some(d) if d.active => match d.usb.and_then(|u| links.get(&u)) {
            Some(false) => Status::Off,
            _ => Status::Available,
        },
        _ => Status::Unplugged,
    }
}

pub struct Choice {
    pub rows: Vec<Row>,
    pub pick: Option<Device>,
}

/// The first available entry not named in `skip`.
pub fn choose(list: &[Entry], devices: &[Device], links: &Links, skip: &[String]) -> Choice {
    let mut pick: Option<Device> = None;
    let rows = list
        .iter()
        .map(|e| {
            let d = find(e, devices);
            let status = status(d, links);
            let chosen = pick.is_none()
                && status == Status::Available
                && d.is_some_and(|d| !skip.contains(&d.name));
            if chosen {
                pick = d.cloned();
            }
            Row {
                id: e.id.clone(),
                name: d.map_or_else(|| e.name.clone(), |d| d.name.clone()),
                status,
                chosen,
            }
        })
        .collect();
    Choice { rows, pick }
}

/// `1038:22A1` in a battery key, when its vendor id is the battery's own.
fn battery_usb(b: &Battery) -> Option<(u16, u16)> {
    let s = b.key.as_bytes();
    let hex = |r: &[u8]| {
        std::str::from_utf8(r)
            .ok()
            .and_then(|t| u16::from_str_radix(t, 16).ok())
    };
    (0..s.len().saturating_sub(8)).find_map(|i| {
        let (vid, pid) = (hex(&s[i..i + 4])?, hex(&s[i + 5..i + 9])?);
        (s[i + 4] == b':' && vid == b.vid && b.vid != 0).then_some((vid, pid))
    })
}

static WAKE: Mutex<Option<Sender<()>>> = Mutex::new(None);
static HEADSETS: Mutex<Links> = Mutex::new(BTreeMap::new());
static BATTERIES: Mutex<Links> = Mutex::new(BTreeMap::new());
static VIEW: Mutex<Option<View>> = Mutex::new(None);
static APPLIED: Mutex<Applied> = Mutex::new(Applied {
    config: None,
    output: None,
    input: None,
});

/// What was last made Windows' default, so a device the user picks by hand stays until the
/// priority's choice changes.
struct Applied {
    config: Option<PriorityConfig>,
    output: Option<String>,
    input: Option<String>,
}

/// Asks for a fresh look soon.
pub fn nudge() {
    if let Some(tx) = lock(&WAKE).as_ref() {
        let _ = tx.send(());
    }
}

/// The SteelSeries headsets seen now, by "1038:22A1" key, with `None` when one did not answer.
pub fn set_headsets(now: impl IntoIterator<Item = (String, Option<bool>)>) {
    let mut map = lock(&HEADSETS);
    let mut next = Links::new();
    for (key, on) in now {
        let Some(usb) = key.split_once(':').and_then(|(v, p)| {
            Some((
                u16::from_str_radix(v, 16).ok()?,
                u16::from_str_radix(p, 16).ok()?,
            ))
        }) else {
            continue;
        };
        if let Some(on) = on.or_else(|| map.get(&usb).copied()) {
            next.insert(usb, on);
        }
    }
    if *map != next {
        *map = next;
        drop(map);
        nudge();
    }
}

/// Links from the last battery poll, for devices foxden-headset does not cover.
pub fn set_batteries(polled: &[Battery]) {
    let next: Links = polled
        .iter()
        .filter_map(|b| battery_usb(b).map(|u| (u, b.connected)))
        .collect();
    let mut map = lock(&BATTERIES);
    if *map != next {
        *map = next;
        drop(map);
        nudge();
    }
}

fn links() -> Links {
    let mut out = lock(&BATTERIES).clone();
    out.extend(lock(&HEADSETS).iter().map(|(k, v)| (*k, *v)));
    out
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct View {
    config: PriorityConfig,
    outputs: Vec<Row>,
    inputs: Vec<Row>,
    /// Connected devices to add.
    choices_out: Vec<Entry>,
    choices_in: Vec<Entry>,
}

fn entries(devices: &[Device]) -> Vec<Entry> {
    devices
        .iter()
        .filter(|d| d.active)
        .map(|d| Entry {
            id: d.id.clone(),
            name: d.name.clone(),
        })
        .collect()
}

/// Makes `pick` Windows' default once each time the choice changes.
fn apply_default(last: &mut Option<String>, pick: Option<&Device>, capture: bool) {
    let Some(d) = pick else { return };
    if last.as_deref() == Some(d.id.as_str()) {
        return;
    }
    *last = Some(d.id.clone());
    if system::default_id(capture).as_deref() == Some(d.id.as_str()) {
        return;
    }
    let what = if capture { "recording" } else { "playback" };
    match foxden_vcable::set_default(&d.id) {
        Ok(()) => crate::log::info("audio", format!("default {what} device: {}", d.name)),
        Err(e) => crate::log::warn("audio", format!("default {what} device not set: {e}")),
    }
}

/// One look: which devices are there, then the mixer and Windows' defaults follow.
fn tick(app: &AppHandle) -> Option<View> {
    let mixer = app.try_state::<Mixer>()?;
    system::com_init();
    let mut applied = lock(&APPLIED);
    let cfg = mixer.priority();
    let links = links();
    let hide: Vec<String> = foxden_vcable::endpoints()
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.id)
        .collect();
    let list = |capture| {
        let mut d = system::devices(capture).unwrap_or_default();
        d.retain(|x| !hide.contains(&x.id));
        d
    };
    let (outs, ins) = (list(false), list(true));
    let out = choose(&cfg.outputs, &outs, &links, &mixer.avoid());
    let inp = choose(&cfg.inputs, &ins, &links, &[]);

    if !cfg.on || !cfg.mixer {
        mixer.follow(None);
    } else if let Some(d) = &out.pick {
        mixer.follow(Some(d.name.clone()));
    }
    if applied.config.as_ref() != Some(&cfg) {
        (applied.output, applied.input) = (None, None);
        applied.config = Some(cfg.clone());
    }
    if cfg.on && cfg.windows {
        apply_default(&mut applied.output, out.pick.as_ref(), false);
        apply_default(&mut applied.input, inp.pick.as_ref(), true);
    }
    let view = View {
        config: cfg,
        outputs: out.rows,
        inputs: inp.rows,
        choices_out: entries(&outs),
        choices_in: entries(&ins),
    };
    let mut last = lock(&VIEW);
    if last.as_ref() != Some(&view) {
        *last = Some(view.clone());
        let _ = app.emit("audio-priority", &view);
    }
    Some(view)
}

/// Watches endpoints and wireless links and follows the priority.
pub fn spawn(app: AppHandle) {
    let (tx, rx) = channel();
    *lock(&WAKE) = Some(tx);
    thread::Builder::new()
        .name("audio-priority".into())
        .spawn(move || {
            system::com_init();
            let _watch = system::watch(nudge)
                .map_err(|e| crate::log::warn("audio", format!("device changes not watched: {e}")))
                .ok();
            loop {
                tick(&app);
                match rx.recv_timeout(IDLE) {
                    Ok(()) => {
                        thread::sleep(SETTLE);
                        while rx.try_recv().is_ok() {}
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("audio priority thread");
}

#[tauri::command(async)]
pub fn get_priority(app: AppHandle) -> Result<View, String> {
    tick(&app).ok_or_else(|| "the mixer has not started".into())
}

#[tauri::command(async)]
pub fn set_priority(
    app: AppHandle,
    mixer: State<'_, Mixer>,
    priority: PriorityConfig,
) -> Result<View, String> {
    mixer.set_priority(priority.tidy());
    tick(&app).ok_or_else(|| "the mixer has not started".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOVA: (u16, u16) = (0x1038, 0x22A1);

    fn dev(id: &str, name: &str, active: bool, usb: Option<(u16, u16)>) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            active,
            usb,
        }
    }

    fn entry(id: &str, name: &str) -> Entry {
        Entry {
            id: id.into(),
            name: name.into(),
        }
    }

    fn setup() -> (Vec<Entry>, Vec<Device>) {
        (
            vec![
                entry("hp3", "Headphones (3- Arctis Nova 7)"),
                entry("spk", "Speakers (Realtek(R) Audio)"),
            ],
            vec![
                dev("hp3", "Headphones (3- Arctis Nova 7)", true, Some(NOVA)),
                dev("spk", "Speakers (Realtek(R) Audio)", true, None),
            ],
        )
    }

    fn picked(c: &Choice) -> Option<&str> {
        c.pick.as_ref().map(|d| d.id.as_str())
    }

    #[test]
    fn the_headset_wins_while_it_is_on() {
        let (list, devs) = setup();
        let on = Links::from([(NOVA, true)]);
        assert_eq!(picked(&choose(&list, &devs, &on, &[])), Some("hp3"));
        // Nothing known about the link: an active endpoint counts.
        assert_eq!(
            picked(&choose(&list, &devs, &Links::new(), &[])),
            Some("hp3")
        );
    }

    #[test]
    fn a_switched_off_headset_falls_back_and_comes_back() {
        let (list, devs) = setup();
        let off = choose(&list, &devs, &Links::from([(NOVA, false)]), &[]);
        assert_eq!(picked(&off), Some("spk"));
        assert_eq!(off.rows[0].status, Status::Off);
        assert!(!off.rows[0].chosen && off.rows[1].chosen);
        let back = choose(&list, &devs, &Links::from([(NOVA, true)]), &[]);
        assert_eq!(picked(&back), Some("hp3"));
    }

    #[test]
    fn a_receiver_moved_to_another_port_still_matches_by_name() {
        let (list, _) = setup();
        let devs = vec![
            dev("hp3", "Headphones (3- Arctis Nova 7)", false, Some(NOVA)),
            dev("hp4", "Headphones (4- Arctis Nova 7)", true, Some(NOVA)),
            dev("spk", "Speakers (Realtek(R) Audio)", true, None),
        ];
        let c = choose(&list, &devs, &Links::new(), &[]);
        assert_eq!(picked(&c), Some("hp4"));
        assert_eq!(c.rows[0].name, "Headphones (4- Arctis Nova 7)");
    }

    #[test]
    fn unplugged_and_skipped_devices_are_passed_over() {
        let (list, mut devs) = setup();
        devs[0].active = false;
        let c = choose(&list, &devs, &Links::new(), &[]);
        assert_eq!(c.rows[0].status, Status::Unplugged);
        assert_eq!(picked(&c), Some("spk"));
        let (_, devs) = setup();
        let skip = ["Headphones (3- Arctis Nova 7)".to_string()];
        assert_eq!(
            picked(&choose(&list, &devs, &Links::new(), &skip)),
            Some("spk")
        );
        let gone = choose(&list[..1], &[], &Links::new(), &[]);
        assert!(gone.pick.is_none() && gone.rows[0].status == Status::Unplugged);
    }

    #[test]
    fn port_numbers_are_ignored_in_names() {
        assert_eq!(
            plain("Headphones (3- Arctis Nova 7)"),
            "Headphones (Arctis Nova 7)"
        );
        assert_eq!(
            plain("Speakers (Realtek(R) Audio)"),
            "Speakers (Realtek(R) Audio)"
        );
        assert_eq!(plain("4 - Q27G4ZR (AMD)"), "4 - Q27G4ZR (AMD)");
    }

    #[test]
    fn battery_keys_give_the_usb_ids() {
        let b = |key: &str, vid| Battery {
            key: key.into(),
            vid,
            ..Battery::default()
        };
        assert_eq!(
            battery_usb(&b("astro-046D:0AFE", 0x046D)),
            Some((0x046D, 0x0AFE))
        );
        assert_eq!(
            battery_usb(&b("atk-373e:0010", 0x373E)),
            Some((0x373E, 0x0010))
        );
        assert_eq!(battery_usb(&b("audeze-maxwell", 0x3329)), None);
        assert_eq!(battery_usb(&b("akg-046D:0AFE", 0x1234)), None);
    }

    #[test]
    fn headsets_that_miss_a_reading_keep_their_last_state() {
        set_headsets([("1038:22A1".to_string(), Some(false))]);
        set_headsets([("1038:22A1".to_string(), None)]);
        assert_eq!(lock(&HEADSETS).get(&NOVA), Some(&false));
        set_headsets([]);
        assert!(lock(&HEADSETS).is_empty());
    }

    #[test]
    fn tidy_drops_repeats() {
        let c = PriorityConfig {
            outputs: vec![entry("a", "A"), entry("a", "A"), entry("", "B")],
            ..PriorityConfig::default()
        }
        .tidy();
        assert_eq!(c.outputs, [entry("a", "A")]);
        assert!(c.mixer && !c.windows && !c.on);
    }
}
