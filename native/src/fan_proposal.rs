//! Fan curves an AI assistant proposes, held until the person at the PC answers on screen.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use foxden_client::{FanState, Service};
use foxden_fan::{Curve, FanConfig, FanRule, Kind, PUMP_FLOOR, pump_like};
use serde_json::{Value, json};

use crate::state::lock;

pub const WAIT: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(180) };
pub const BUSY: &str = "Another change is waiting for approval";
pub const LATE: &str = "Not approved in time";
pub const REJECTED: &str = "The user rejected the change";
pub const APPLIED: &str = "Applied";
pub const TAB_BUSY: &str = "FoxDen's Fans tab is busy applying another change. Try again in a moment.";
pub const NO_WINDOW: &str = "FoxDen's window could not open to ask for approval.";
const T_MIN: f64 = 20.0;
const T_MAX: f64 = 100.0;
const REASON: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    Apply,
    Reject,
}

/// What the approval sheet shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub id: u64,
    pub client: String,
    pub reason: String,
    pub diff: Diff,
}

/// One fan as a rule leaves it: its curve's name and points, and its duty limits.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub curve: String,
    pub points: Vec<(f32, f32)>,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub name: String,
    /// `None` while the firmware drives the fan.
    pub before: Option<Drive>,
    pub after: Option<Drive>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub changed: Vec<Change>,
    pub unchanged: usize,
    /// Curves that no fan follows, before or after, also change.
    pub loose: bool,
}

/// The one proposal that may wait for an answer, and how the last one ended.
#[derive(Default)]
pub struct Desk {
    slot: Mutex<Option<(u64, Sender<Decision>)>>,
    next: AtomicU64,
    last: Mutex<Option<Value>>,
}

pub static DESK: Desk = Desk::new();

impl Desk {
    pub const fn new() -> Desk {
        Desk { slot: Mutex::new(None), next: AtomicU64::new(1), last: Mutex::new(None) }
    }

    fn reserve(&self) -> Result<(u64, Receiver<Decision>), String> {
        let mut slot = lock(&self.slot);
        if slot.is_some() {
            return Err(BUSY.into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        *slot = Some((id, tx));
        Ok((id, rx))
    }

    /// Takes the proposal `id` off the desk, if it is still there.
    fn take(&self, id: u64) -> Option<Sender<Decision>> {
        let mut slot = lock(&self.slot);
        match slot.take() {
            Some((at, tx)) if at == id => Some(tx),
            other => {
                *slot = other;
                None
            }
        }
    }

    /// The person's answer to proposal `id`; false when it is no longer waiting.
    pub fn decide(&self, id: u64, decision: Decision) -> bool {
        self.take(id).is_some_and(|tx| tx.send(decision).is_ok())
    }

    /// Rejects whatever is waiting.
    pub fn cancel(&self) {
        let waiting = lock(&self.slot).as_ref().map(|s| s.0);
        if let Some(id) = waiting {
            self.decide(id, Decision::Reject);
        }
    }

    pub fn last(&self) -> Value {
        lock(&self.last).clone().unwrap_or(Value::Null)
    }

    fn record(&self, client: &str, outcome: &str) {
        *lock(&self.last) = Some(json!({"client": client, "outcome": outcome}));
    }
}

/// What the flow needs from the app, so tests can stand in for the window and the service.
pub struct Hooks<'a> {
    pub show: &'a dyn Fn(Ask) -> Result<(), String>,
    pub close: &'a dyn Fn(u64),
    pub apply: &'a dyn Fn(FanConfig) -> Result<(), String>,
}

/// Checks a proposal, asks the person on screen and applies it once they agree.
pub fn propose(desk: &Desk, state: &FanState, args: &Value, client: &str, hooks: &Hooks, wait: Duration) -> Result<String, String> {
    if state.service != Service::Running {
        return Err("FoxDen's fan service isn't running, so fan curves can't be changed now.".into());
    }
    let reason = match args["reason"].as_str().map(|r| clean_text(r, REASON)) {
        Some(r) if !r.is_empty() => r,
        _ => return Err("reason must be a short non-empty string.".into()),
    };
    let config = from_percent(args)?;
    check(&config, state)?;
    let now = rounded(&state.config);
    if tidy(&config) == tidy(&now) {
        return Ok("Nothing to change: that is the fan config in use.".into());
    }
    let (id, rx) = desk.reserve()?;
    let client = match clean_text(client, 40) {
        c if c.is_empty() => String::new(),
        c => c,
    };
    let ask = Ask { id, client: client.clone(), reason, diff: diff(&now, &config, state) };
    desk.record(&client, "waiting for approval");
    if let Err(e) = (hooks.show)(ask) {
        desk.take(id);
        desk.record(&client, "not shown");
        return Err(e);
    }
    let decision = match rx.recv_timeout(wait) {
        Ok(d) => Some(d),
        // The answer came at the last moment: it is on its way.
        Err(RecvTimeoutError::Timeout) if desk.take(id).is_none() => rx.recv().ok(),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => Some(Decision::Reject),
    };
    (hooks.close)(id);
    let result = match decision {
        None => Err(LATE.to_string()),
        Some(Decision::Reject) => Ok(REJECTED.to_string()),
        Some(Decision::Apply) => (hooks.apply)(config).map(|_| APPLIED.to_string()),
    };
    let outcome = match (&decision, &result) {
        (None, _) => "not approved in time".to_string(),
        (Some(Decision::Reject), _) => "rejected".to_string(),
        (_, Ok(_)) => "applied".to_string(),
        (_, Err(e)) => format!("approved, but the service refused it: {e}"),
    };
    desk.record(&client, &outcome);
    result
}

/// Printable text on one line, clipped to `max` characters.
pub fn clean_text(text: &str, max: usize) -> String {
    let printable: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !invisible(*c))
        .collect();
    let one_line = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = one_line.chars().take(max).collect();
    if one_line.chars().count() > max {
        out = out.trim_end().to_string();
        out.push('…');
    }
    out
}

pub(crate) fn invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

pub fn tidy(config: &FanConfig) -> FanConfig {
    let mut c = config.clone();
    c.rules.sort_by(|a, b| a.fan.cmp(&b.fan));
    c
}

/// Percent to a rule's duty, as the Fans tab rounds a typed one.
fn rule_duty(percent: f64) -> f32 {
    ((percent as f32) * 10.0).round() / 1000.0
}

/// Percent to a curve point's duty, in whole percent like the tab.
fn point_duty(percent: f64) -> f32 {
    (percent as f32).round() / 100.0
}

fn tenths(v: f64) -> f32 {
    ((v as f32) * 10.0).round() / 10.0
}

/// A fraction as percent, as precise as the Fans tab keeps it.
fn pct(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 10.0
}

fn tenth(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

/// The config as the Fans tab rounds it.
fn rounded(config: &FanConfig) -> FanConfig {
    let mut c = config.clone();
    for curve in &mut c.curves {
        for p in &mut curve.points {
            *p = (tenths(tenth(p.0)), point_duty(pct(p.1)));
        }
        curve.hysteresis = tenths(tenth(curve.hysteresis));
    }
    for rule in &mut c.rules {
        for v in [&mut rule.min_duty, &mut rule.max_duty, &mut rule.ramp] {
            *v = rule_duty(pct(*v));
        }
    }
    c
}

/// The config, fans and sensors in the shape `propose_fan_curves` takes.
pub fn view(state: &FanState, last: Value) -> Value {
    let config = &state.config;
    let fan_name = |id: &str| state.fans.iter().find(|f| f.id == id).map_or(id, |f| f.name.as_str()).to_string();
    let curves: Vec<Value> = config
        .curves
        .iter()
        .map(|c| {
            let points: Vec<Value> = c.points.iter().map(|p| json!([tenth(p.0), pct(p.1)])).collect();
            json!({"name": c.name, "sensors": c.sensors, "points": points, "hysteresis": tenth(c.hysteresis)})
        })
        .collect();
    let rules: Vec<Value> = tidy(config)
        .rules
        .iter()
        .map(|r| {
            json!({"fan": r.fan, "name": fan_name(&r.fan), "curve": r.curve,
                "min_duty": pct(r.min_duty), "max_duty": pct(r.max_duty), "ramp": pct(r.ramp)})
        })
        .collect();
    let fans: Vec<Value> = state
        .fans
        .iter()
        .map(|f| json!({"id": f.id, "name": f.name, "pump": pump_like(&f.id, &f.name), "rpm": f.rpm}))
        .collect();
    let sensors: Vec<Value> = state
        .sensors
        .iter()
        .filter(|s| s.kind == Kind::Temperature)
        .map(|s| json!({"id": s.id, "name": s.name, "celsius": s.value}))
        .collect();
    json!({
        "service": state.service, "curves": curves, "rules": rules, "fans": fans, "sensors": sensors,
        "last_proposal": last,
    })
}

fn num(v: &Value, what: &str) -> Result<f64, String> {
    v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| format!("{what} must be a number."))
}

fn within(v: f64, lo: f64, hi: f64, what: &str) -> Result<f64, String> {
    if (lo..=hi).contains(&v) { Ok(v) } else { Err(format!("{what} must be from {lo} to {hi}.")) }
}

fn opt(v: &Value, default: f64, what: &str) -> Result<f64, String> {
    if v.is_null() { Ok(default) } else { num(v, what) }
}

/// The proposal's curves and rules, from percent and °C to the service's fractions.
pub fn from_percent(args: &Value) -> Result<FanConfig, String> {
    let list = |key: &str| args[key].as_array().cloned().ok_or_else(|| format!("{key} must be a list."));
    let mut config = FanConfig::default();
    for c in list("curves")? {
        let name: String = c["name"].as_str().map(str::trim).unwrap_or_default().to_string();
        if name.is_empty() || name.chars().count() > 40 || name.chars().any(|c| c.is_control() || invisible(c)) {
            return Err("Each curve needs a name of 1 to 40 printable characters.".into());
        }
        let sensors = c["sensors"]
            .as_array()
            .and_then(|s| s.iter().map(|id| id.as_str().map(str::to_string)).collect::<Option<Vec<_>>>())
            .ok_or_else(|| format!("Curve \"{name}\" needs sensors as a list of sensor ids."))?;
        let raw = c["points"].as_array().ok_or_else(|| format!("Curve \"{name}\" needs points as [temperature, duty] pairs."))?;
        let mut points = Vec::new();
        for p in raw {
            let pair = p.as_array().filter(|p| p.len() == 2).ok_or_else(|| format!("Each point of curve \"{name}\" must be [temperature °C, duty %]."))?;
            let t = within(num(&pair[0], "A temperature")?, T_MIN, T_MAX, &format!("Temperatures of curve \"{name}\""))?;
            let d = within(num(&pair[1], "A duty")?, 0.0, 100.0, &format!("Duties of curve \"{name}\""))?;
            points.push((tenths(t), point_duty(d)));
        }
        if points.windows(2).any(|w| w[1].0 - w[0].0 < 0.95) {
            return Err(format!("Points of curve \"{name}\" must rise in temperature, at least 1 °C apart."));
        }
        let hysteresis = within(opt(&c["hysteresis"], 2.0, "hysteresis")?, 0.0, 30.0, &format!("The hysteresis of curve \"{name}\""))?;
        config.curves.push(Curve { name, sensors, points, hysteresis: tenths(hysteresis) });
    }
    for r in list("rules")? {
        let fan = r["fan"].as_str().map(str::trim).filter(|f| !f.is_empty()).ok_or("Each rule needs a fan id.")?.to_string();
        let curve = r["curve"].as_str().map(str::trim).unwrap_or_default().to_string();
        let duty = |key: &str, default: f64, lo: f64| -> Result<f32, String> {
            let v = within(opt(&r[key], default, key)?, lo, 100.0, &format!("{key} of fan {fan}"))?;
            Ok(rule_duty(v))
        };
        config.rules.push(FanRule {
            min_duty: duty("min_duty", 0.0, 0.0)?,
            max_duty: duty("max_duty", 100.0, 0.0)?,
            ramp: duty("ramp", 20.0, 1.0)?,
            fan,
            curve,
        });
    }
    Ok(config)
}

/// The service's own checks, then what the Fans tab would not let a person enter.
pub fn check(config: &FanConfig, state: &FanState) -> Result<(), String> {
    // The service speaks in fractions; the assistant in percent.
    config.validate().map_err(|e| e.replace("duties from 0 to 1,", "duties from 0 to 100%,"))?;
    let known_sensors: BTreeSet<&str> = state
        .sensors
        .iter()
        .filter(|s| s.kind == Kind::Temperature)
        .map(|s| s.id.as_str())
        .chain(state.config.curves.iter().flat_map(|c| c.sensors.iter().map(String::as_str)))
        .collect();
    for curve in &config.curves {
        if let Some(id) = curve.sensors.iter().find(|id| !known_sensors.contains(id.as_str())) {
            return Err(format!("Curve \"{}\" uses \"{id}\", which is not a temperature sensor. get_fan_config lists them.", curve.name));
        }
    }
    for rule in &config.rules {
        let live = state.fans.iter().find(|f| f.id == rule.fan);
        if live.is_none() && !state.config.rules.iter().any(|r| r.fan == rule.fan) {
            return Err(format!("There is no fan \"{}\". get_fan_config lists them.", rule.fan));
        }
        let name = live.map_or(rule.fan.as_str(), |f| f.name.as_str());
        if pump_like(&rule.fan, name) && rule.max_duty < PUMP_FLOOR {
            return Err(format!("{name} is a pump: its maximum duty must be at least {}%.", PUMP_FLOOR * 100.0));
        }
    }
    Ok(())
}

fn drive(config: &FanConfig, fan: &str) -> Option<Drive> {
    let rule = config.rules.iter().find(|r| r.fan == fan)?;
    let points = config.curves.iter().find(|c| c.name == rule.curve).map(|c| c.points.clone()).unwrap_or_default();
    Some(Drive { curve: rule.curve.clone(), points, min: rule.min_duty, max: rule.max_duty })
}

/// Each fan whose rule or curve changes, and how many stay as they are.
pub fn diff(before: &FanConfig, after: &FanConfig, state: &FanState) -> Diff {
    let mut ids: Vec<String> = state.fans.iter().map(|f| f.id.clone()).collect();
    for r in before.rules.iter().chain(&after.rules) {
        if !ids.contains(&r.fan) {
            ids.push(r.fan.clone());
        }
    }
    let curve = |c: &FanConfig, name: &str| c.curves.iter().find(|x| x.name == name).cloned();
    let rule = |c: &FanConfig, fan: &str| c.rules.iter().find(|r| r.fan == fan).cloned();
    let mut out = Diff::default();
    for id in &ids {
        let (was, now) = (rule(before, id), rule(after, id));
        let same_curve = match (&was, &now) {
            (Some(a), Some(b)) => curve(before, &a.curve) == curve(after, &b.curve),
            _ => true,
        };
        if was == now && same_curve {
            out.unchanged += 1;
            continue;
        }
        let name = state.fans.iter().find(|f| f.id == *id).map_or(id.clone(), |f| f.name.clone());
        out.changed.push(Change { name, before: drive(before, id), after: drive(after, id) });
    }
    let used: BTreeSet<&str> = before.rules.iter().chain(&after.rules).map(|r| r.curve.as_str()).collect();
    let loose = |c: &FanConfig| -> Vec<Curve> { c.curves.iter().filter(|x| !used.contains(x.name.as_str())).cloned().collect() };
    out.loose = loose(before) != loose(after);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use foxden_fan::{FanStatus, SensorStatus};
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::thread;

    fn fan(id: &str, name: &str) -> FanStatus {
        FanStatus { id: id.into(), name: name.into(), ..FanStatus::default() }
    }

    fn sensor(id: &str, kind: Kind) -> SensorStatus {
        SensorStatus { id: id.into(), name: id.into(), value: Some(50.0), kind }
    }

    fn state() -> FanState {
        let config = FanConfig {
            curves: vec![Curve { name: "Quiet".into(), sensors: vec!["cpu/package".into()], points: vec![(30.0, 0.25), (70.0, 0.8)], hysteresis: 2.0 }],
            rules: vec![
                FanRule { fan: "board/fan2".into(), curve: "Quiet".into(), min_duty: 0.0, max_duty: 1.0, ramp: 0.2 },
                FanRule { fan: "board/fan1".into(), curve: "Quiet".into(), min_duty: 0.123, max_duty: 0.9, ramp: 0.05 },
            ],
        };
        FanState {
            service: Service::Running,
            fans: vec![fan("board/fan1", "CPU fan"), fan("board/fan2", "Rear fan"), fan("board/fan3", "Front fan"), fan("aio/pump", "AIO pump")],
            sensors: vec![sensor("cpu/package", Kind::Temperature), sensor("psu/power", Kind::Power)],
            config,
            ..FanState::default()
        }
    }

    fn proposal(state: &FanState) -> Value {
        let mut v = view(state, Value::Null);
        v["reason"] = json!("Quieter at idle.");
        v
    }

    #[test]
    fn get_fan_config_reads_back_as_the_same_config() {
        let s = state();
        let v = view(&s, Value::Null);
        assert_eq!(v["rules"][0]["fan"], "board/fan1");
        assert_eq!(v["rules"][0]["name"], "CPU fan");
        assert_eq!(v["rules"][0]["min_duty"], 12.3);
        assert_eq!(v["rules"][0]["ramp"], 5.0);
        assert_eq!(v["curves"][0]["points"], json!([[30.0, 25.0], [70.0, 80.0]]));
        assert_eq!(v["fans"][3]["pump"], true);
        assert_eq!(v["sensors"].as_array().unwrap().len(), 1);
        assert_eq!(tidy(&from_percent(&v).unwrap()), tidy(&s.config));
    }

    #[test]
    fn percent_and_degrees_become_fractions() {
        let c = from_percent(&json!({
            "curves": [{"name": " Cool ", "sensors": ["cpu/package"], "points": [[25, 33.333], [60.04, 100]]}],
            "rules": [{"fan": "board/fan3", "curve": "Cool", "max_duty": 75.55}],
        }))
        .unwrap();
        assert_eq!(c.curves[0].name, "Cool");
        assert_eq!(c.curves[0].points, vec![(25.0, 0.33), (60.0, 1.0)]);
        assert_eq!(c.curves[0].hysteresis, 2.0);
        let r = &c.rules[0];
        assert_eq!((r.min_duty, r.max_duty, r.ramp), (0.0, 0.756, 0.2));
        for bad in [
            json!({"curves": [{"name": "A", "sensors": ["x"], "points": [[10, 50]]}], "rules": []}),
            json!({"curves": [{"name": "A", "sensors": ["x"], "points": [[30, 150]]}], "rules": []}),
            json!({"curves": [{"name": "A", "sensors": ["x"], "points": [[30, 50], [30.5, 60]]}], "rules": []}),
            json!({"curves": [{"name": "", "sensors": ["x"], "points": [[30, 50]]}], "rules": []}),
            json!({"curves": [{"name": "Quiet\u{202e}", "sensors": ["x"], "points": [[30, 50]]}], "rules": []}),
            json!({"curves": [], "rules": [{"fan": "f", "curve": "A", "ramp": 0}]}),
            json!({"curves": "none", "rules": []}),
        ] {
            assert!(from_percent(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn checks_hold_the_services_and_the_tabs_limits() {
        let s = state();
        let base = proposal(&s);
        let refused = |change: &dyn Fn(&mut Value)| {
            let mut v = base.clone();
            change(&mut v);
            from_percent(&v).and_then(|c| check(&c, &s)).unwrap_err()
        };
        assert!(refused(&|v| v["rules"][1]["max_duty"] = json!(10)).contains("at least 20%"));
        assert!(refused(&|v| v["rules"][0]["max_duty"] = json!(10)).contains("0 to 100%"));
        assert!(refused(&|v| v["curves"][0]["points"] = json!([[30, 5], [70, 10]])).contains("never run above"));
        assert!(refused(&|v| v["curves"][0]["sensors"] = json!(["psu/power"])).contains("not a temperature sensor"));
        assert!(refused(&|v| v["rules"][0]["fan"] = json!("nope")).contains("no fan"));
        assert!(refused(&|v| v["rules"][0]["curve"] = json!("Loud")).contains("unknown curve"));
        let pump = |v: &mut Value| v["rules"].as_array_mut().unwrap().push(json!({"fan": "aio/pump", "curve": "Quiet", "max_duty": 40}));
        assert!(refused(&pump).contains("pump"));
        let mut ok = base.clone();
        ok["rules"].as_array_mut().unwrap().push(json!({"fan": "aio/pump", "curve": "Quiet", "max_duty": 60}));
        assert!(from_percent(&ok).and_then(|c| check(&c, &s)).is_ok());
    }

    #[test]
    fn the_diff_lists_changed_fans_and_counts_the_rest() {
        let s = state();
        let mut after = s.config.clone();
        after.rules.retain(|r| r.fan != "board/fan2");
        after.rules.push(FanRule { fan: "board/fan3".into(), curve: "Quiet".into(), min_duty: 0.3, max_duty: 1.0, ramp: 0.2 });
        let d = diff(&s.config, &after, &s);
        let names: Vec<&str> = d.changed.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Rear fan", "Front fan"]);
        assert_eq!(d.changed[0].after, None);
        assert_eq!(d.changed[1].before, None);
        assert_eq!(d.changed[1].after.as_ref().unwrap().min, 0.3);
        assert_eq!(d.unchanged, 2);
        assert!(!d.loose);

        let mut after = s.config.clone();
        after.curves[0].points[1].1 = 0.9;
        let d = diff(&s.config, &after, &s);
        assert_eq!(d.changed.len(), 2);
        let c = &d.changed[0];
        assert_eq!((c.before.as_ref().unwrap().points[1].1, c.after.as_ref().unwrap().points[1].1), (0.8, 0.9));
        assert_eq!(d.unchanged, 2);

        let mut after = s.config.clone();
        after.curves.push(Curve { name: "Spare".into(), sensors: vec!["cpu/package".into()], points: vec![(40.0, 0.5)], hysteresis: 2.0 });
        let d = diff(&s.config, &after, &s);
        assert!(d.changed.is_empty() && d.loose);
    }

    #[test]
    fn text_from_the_assistant_is_cleaned_and_clipped() {
        assert_eq!(clean_text("Claude\u{202e}\n  Desktop\u{200b}", 40), "Claude Desktop");
        let long = clean_text(&"word ".repeat(100), 300);
        assert_eq!(long.chars().count(), 300);
        assert!(long.ends_with('…'));
        assert_eq!(clean_text("\u{7}", 40), "");
    }

    struct Fake {
        shown: RefCell<Vec<Ask>>,
        closed: RefCell<Vec<u64>>,
        applied: RefCell<Vec<FanConfig>>,
    }

    fn fake() -> Fake {
        Fake { shown: RefCell::default(), closed: RefCell::default(), applied: RefCell::default() }
    }

    fn run(desk: &Desk, f: &Fake, state: &FanState, args: &Value, answer: Option<Decision>) -> Result<String, String> {
        let show = |ask: Ask| {
            if let Some(d) = answer {
                assert!(desk.decide(ask.id, d));
            }
            f.shown.borrow_mut().push(ask);
            Ok(())
        };
        let close = |id| f.closed.borrow_mut().push(id);
        let apply = |c: FanConfig| {
            f.applied.borrow_mut().push(c);
            Ok(())
        };
        propose(desk, state, args, "Tester", &Hooks { show: &show, close: &close, apply: &apply }, WAIT)
    }

    fn changed(s: &FanState) -> Value {
        let mut v = proposal(s);
        v["rules"][0]["max_duty"] = json!(70);
        v
    }

    #[test]
    fn a_bad_proposal_never_reaches_the_screen() {
        let (desk, f, s) = (Desk::new(), fake(), state());
        let mut v = changed(&s);
        v["rules"][0]["max_duty"] = json!(5);
        assert!(run(&desk, &f, &s, &v, None).is_err());
        let mut v = changed(&s);
        v["reason"] = json!("  ");
        assert!(run(&desk, &f, &s, &v, None).is_err());
        let r = run(&desk, &f, &s, &proposal(&s), None).unwrap();
        assert!(r.starts_with("Nothing to change"));
        let stopped = FanState { service: Service::Stopped, ..state() };
        assert!(run(&desk, &f, &stopped, &changed(&s), None).is_err());
        assert!(f.shown.borrow().is_empty());
        assert_eq!(desk.last(), Value::Null);
    }

    #[test]
    fn approval_applies_and_rejection_does_not() {
        let (desk, f, s) = (Desk::new(), fake(), state());
        assert_eq!(run(&desk, &f, &s, &changed(&s), Some(Decision::Apply)).unwrap(), APPLIED);
        assert_eq!(f.applied.borrow()[0].rules[0].max_duty, 0.7);
        assert_eq!(f.shown.borrow()[0].client, "Tester");
        assert_eq!(f.shown.borrow()[0].diff.changed.len(), 1);
        assert_eq!(desk.last()["outcome"], "applied");
        assert_eq!(run(&desk, &f, &s, &changed(&s), Some(Decision::Reject)).unwrap(), REJECTED);
        assert_eq!(f.applied.borrow().len(), 1);
        assert_eq!(f.closed.borrow().len(), 2);
        assert_eq!(desk.last()["outcome"], "rejected");
    }

    #[test]
    fn an_unanswered_proposal_times_out_and_closes_its_sheet() {
        let (desk, f, s) = (Desk::new(), fake(), state());
        assert_eq!(run(&desk, &f, &s, &changed(&s), None).unwrap_err(), LATE);
        let id = f.shown.borrow()[0].id;
        assert_eq!(*f.closed.borrow(), [id]);
        assert!(!desk.decide(id, Decision::Apply), "a late click does nothing");
        assert!(f.applied.borrow().is_empty());
        assert_eq!(desk.last()["outcome"], "not approved in time");
    }

    #[test]
    fn only_one_proposal_waits_at_a_time() {
        let desk = Arc::new(Desk::new());
        let s = state();
        let first = {
            let (desk, s) = (desk.clone(), s.clone());
            thread::spawn(move || {
                let f = fake();
                let long = Duration::from_secs(5);
                let none = |_: Ask| Ok(());
                let close = |_| {};
                let apply = |_| Ok(());
                propose(&desk, &s, &changed(&s), "", &Hooks { show: &none, close: &close, apply: &apply }, long).map(|r| (r, f.applied.borrow().len()))
            })
        };
        while lock(&desk.slot).is_none() {
            thread::yield_now();
        }
        let f = fake();
        assert_eq!(run(&desk, &f, &s, &changed(&s), None).unwrap_err(), BUSY);
        desk.cancel();
        assert_eq!(first.join().unwrap().unwrap().0, REJECTED);
        assert_eq!(run(&desk, &f, &s, &changed(&s), Some(Decision::Reject)).unwrap(), REJECTED);
    }
}
