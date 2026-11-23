use std::sync::{Arc, Mutex};

use super::*;
use crate::sim::SimBackend;

struct Bench {
    temps: Vec<Option<f32>>,
    sets: Vec<Vec<f32>>,
    releases: Vec<u32>,
    fail_set: bool,
    fail_release: bool,
}

type Shared = Arc<Mutex<Bench>>;

struct BenchSensor(usize, String, Shared);

impl Sensor for BenchSensor {
    fn id(&self) -> &str {
        &self.1
    }
    fn name(&self) -> &str {
        &self.1
    }
    fn read(&mut self) -> Option<f32> {
        self.2.lock().unwrap().temps[self.0]
    }
}

struct BenchFan(usize, String, Shared);

impl Fan for BenchFan {
    fn id(&self) -> &str {
        &self.1
    }
    fn name(&self) -> &str {
        &self.1
    }
    fn rpm(&mut self) -> Option<u32> {
        None
    }
    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        let mut bench = self.2.lock().unwrap();
        if bench.fail_set {
            return Err("write failed".into());
        }
        bench.sets[self.0].push(duty);
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        let mut bench = self.2.lock().unwrap();
        if bench.fail_release {
            return Err("release failed".into());
        }
        bench.releases[self.0] += 1;
        Ok(())
    }
}

struct BenchBackend(Shared);

impl FanBackend for BenchBackend {
    fn name(&self) -> &str {
        "bench"
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        let count = self.0.lock().unwrap().temps.len();
        (0..count)
            .map(|i| Box::new(BenchSensor(i, format!("s{i}"), self.0.clone())) as Box<dyn Sensor>)
            .collect()
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        let count = self.0.lock().unwrap().sets.len();
        (0..count)
            .map(|i| Box::new(BenchFan(i, format!("f{i}"), self.0.clone())) as Box<dyn Fan>)
            .collect()
    }
}

/// Sensors s0.. reading 30 degrees and fans f0...
fn rig(sensors: usize, fans: usize) -> Shared {
    Arc::new(Mutex::new(Bench {
        temps: vec![Some(30.0); sensors],
        sets: vec![Vec::new(); fans],
        releases: vec![0; fans],
        fail_set: false,
        fail_release: false,
    }))
}

/// A rig driven by `config`.
fn bench(sensors: usize, fans: usize, config: FanConfig) -> (Controller, Shared) {
    let shared = rig(sensors, fans);
    let controller = Controller::new(vec![Box::new(BenchBackend(shared.clone()))], config);
    (controller, shared)
}

fn set_temp(shared: &Shared, sensor: usize, temp: Option<f32>) {
    shared.lock().unwrap().temps[sensor] = temp;
}

fn curve(name: &str, sensors: &[&str], hysteresis: f32) -> Curve {
    Curve {
        name: name.into(),
        sensors: sensors.iter().map(|s| s.to_string()).collect(),
        points: vec![(30.0, 0.2), (50.0, 0.4), (80.0, 1.0)],
        hysteresis,
    }
}

fn rule(fan: &str, curve: &str, ramp: f32) -> FanRule {
    FanRule {
        fan: fan.into(),
        curve: curve.into(),
        min_duty: 0.0,
        max_duty: 1.0,
        ramp,
    }
}

/// One curve on s0 driving f0.
fn simple(hysteresis: f32, ramp: f32) -> FanConfig {
    FanConfig {
        curves: vec![curve("c", &["s0"], hysteresis)],
        rules: vec![rule("f0", "c", ramp)],
    }
}

fn duty(status: &Status, fan: usize) -> f32 {
    status.fans[fan].duty.expect("fan has a duty")
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn curve_interpolates_and_clamps() {
    let c = curve("c", &[], 0.0);
    for (temp, want) in [
        (-10.0, 0.2),
        (30.0, 0.2),
        (40.0, 0.3),
        (50.0, 0.4),
        (65.0, 0.7),
        (80.0, 1.0),
        (120.0, 1.0),
    ] {
        assert!(
            close(c.duty_at(temp), want),
            "{temp} gave {}",
            c.duty_at(temp)
        );
    }
    let flat = Curve {
        points: vec![(40.0, 0.5)],
        ..c
    };
    assert_eq!(flat.duty_at(0.0), 0.5);
    assert_eq!(flat.duty_at(90.0), 0.5);
}

#[test]
fn the_hottest_sensor_drives_the_curve() {
    let config = FanConfig {
        curves: vec![curve("c", &["s0", "s1"], 0.0)],
        rules: vec![rule("f0", "c", 1000.0)],
    };
    let (mut controller, shared) = bench(2, 1, config);
    set_temp(&shared, 0, Some(40.0));
    set_temp(&shared, 1, Some(65.0));
    assert!(close(duty(&controller.tick(1.0), 0), 0.7));
    set_temp(&shared, 0, Some(70.0));
    assert!(close(duty(&controller.tick(1.0), 0), 0.8));
}

#[test]
fn losing_one_sensor_of_several_is_failsafe() {
    let config = FanConfig {
        curves: vec![curve("c", &["s0", "s1"], 0.0)],
        rules: vec![rule("f0", "c", 1000.0)],
    };
    let (mut controller, shared) = bench(2, 1, config);
    set_temp(&shared, 0, Some(40.0));
    set_temp(&shared, 1, Some(65.0));
    assert!(close(duty(&controller.tick(1.0), 0), 0.7));

    // The cooler sensor that still reads must not slow the fan down.
    set_temp(&shared, 1, None);
    let status = controller.tick(1.0);
    assert_eq!(status.sensors[1].value, None);
    assert!(status.fans[0].failsafe);
    assert_eq!(duty(&status, 0), 1.0);

    // Nor may the hotter one, when it is the cooler one that went.
    set_temp(&shared, 1, Some(65.0));
    set_temp(&shared, 0, None);
    let status = controller.tick(1.0);
    assert!(status.fans[0].failsafe);
    assert_eq!(duty(&status, 0), 1.0);

    set_temp(&shared, 0, Some(40.0));
    let status = controller.tick(1.0);
    assert!(!status.fans[0].failsafe);
    assert!(close(duty(&status, 0), 0.7));
}

#[test]
fn a_power_reading_never_drives_a_curve() {
    let (mut controller, _) = bench(1, 1, simple(0.0, 1000.0));
    let watts = |value| SensorStatus {
        id: "s0".into(),
        name: "s0".into(),
        value: Some(value),
        kind: Kind::Power,
    };
    let status = controller.apply(1.0, vec![watts(40.0)]);
    assert!(status.fans[0].failsafe);
    assert!(close(duty(&status, 0), 1.0));
    let temp = SensorStatus {
        kind: Kind::Temperature,
        ..watts(40.0)
    };
    assert!(!controller.apply(1.0, vec![temp]).fans[0].failsafe);
    let json = serde_json::to_string(&watts(186.0)).unwrap();
    assert!(json.contains("\"kind\":\"power\""));
    let old: SensorStatus = serde_json::from_str(r#"{"id":"a","name":"a","value":1.0}"#).unwrap();
    assert_eq!(old.kind, Kind::Temperature);
}

#[test]
fn readings_can_come_from_a_probe_read_elsewhere() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    let mut probe = controller.take_probe();
    set_temp(&shared, 0, Some(65.0));
    let readings = probe.read(1.0);
    assert_eq!(readings[0].value, Some(65.0));
    let status = controller.apply(1.0, readings);
    assert_eq!(status.sensors[0].value, Some(65.0));
    assert!(close(duty(&status, 0), 0.7));

    // Without its probe the controller has nothing to read.
    let status = controller.tick(1.0);
    assert!(status.sensors.is_empty());
    assert!(status.fans[0].failsafe);
}

#[test]
fn rule_clamps_to_min_and_max() {
    let mut config = simple(0.0, 1000.0);
    config.rules[0].min_duty = 0.3;
    config.rules[0].max_duty = 0.8;
    let (mut controller, shared) = bench(1, 1, config);
    assert!(close(duty(&controller.tick(1.0), 0), 0.3));
    set_temp(&shared, 0, Some(90.0));
    assert!(close(duty(&controller.tick(1.0), 0), 0.8));
}

#[test]
fn hysteresis_rises_at_once_and_falls_late() {
    let (mut controller, shared) = bench(1, 1, simple(5.0, 1000.0));
    let mut at = |temp: f32| {
        set_temp(&shared, 0, Some(temp));
        duty(&controller.tick(1.0), 0)
    };
    assert!(close(at(40.0), 0.3));
    assert!(close(at(50.0), 0.4));
    assert!(close(at(47.0), 0.4));
    assert!(close(at(45.5), 0.4));
    assert!(close(at(45.0), 0.35));
    assert!(close(at(46.0), 0.36));
    assert!(close(at(44.0), 0.36));
    assert!(close(at(41.0), 0.31));
}

#[test]
fn ramp_limits_duty_change() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 0.1));
    // The first write has nothing to ramp from.
    assert!(close(duty(&controller.tick(1.0), 0), 0.2));
    set_temp(&shared, 0, Some(80.0));
    assert!(close(duty(&controller.tick(1.0), 0), 0.3));
    assert!(close(duty(&controller.tick(1.0), 0), 0.4));
    assert!(close(duty(&controller.tick(0.5), 0), 0.45));
    assert!(close(duty(&controller.tick(0.0), 0), 0.45));
    assert!(close(duty(&controller.tick(f32::NAN), 0), 0.45));
    assert_eq!(duty(&controller.tick(60.0), 0), 1.0);
    set_temp(&shared, 0, Some(30.0));
    assert!(close(duty(&controller.tick(1.0), 0), 0.9));
    assert!(close(duty(&controller.tick(2.0), 0), 0.7));
    assert_eq!(duty(&controller.tick(60.0), 0), 0.2);
}

#[test]
fn failsafe_on_sensor_loss_and_recovery() {
    let config = FanConfig {
        curves: vec![curve("c", &["s0", "s1"], 0.0)],
        rules: vec![rule("f0", "c", 0.1)],
    };
    let (mut controller, shared) = bench(2, 1, config);
    let status = controller.tick(1.0);
    assert!(close(duty(&status, 0), 0.2));
    assert!(!status.fans[0].failsafe);

    // Full speed at once, not at the ramp rate.
    set_temp(&shared, 0, None);
    set_temp(&shared, 1, Some(f32::NAN));
    for _ in 0..3 {
        let status = controller.tick(1.0);
        assert_eq!(duty(&status, 0), 1.0);
        assert!(status.fans[0].failsafe);
    }
    assert_eq!(shared.lock().unwrap().sets[0].last(), Some(&1.0));

    // One sensor back is not enough.
    set_temp(&shared, 1, Some(30.0));
    let status = controller.tick(1.0);
    assert!(status.fans[0].failsafe);
    assert_eq!(duty(&status, 0), 1.0);

    set_temp(&shared, 0, Some(30.0));
    let status = controller.tick(1.0);
    assert!(!status.fans[0].failsafe);
    assert!(close(duty(&status, 0), 0.9));
}

#[test]
fn curve_with_unknown_sensor_is_failsafe() {
    for sensors in [&["missing"][..], &["s0", "missing"]] {
        let config = FanConfig {
            curves: vec![curve("c", sensors, 0.0)],
            rules: vec![rule("f0", "c", 0.1)],
        };
        let (mut controller, _shared) = bench(1, 1, config);
        let status = controller.tick(1.0);
        assert!(status.fans[0].failsafe, "{sensors:?}");
        assert_eq!(duty(&status, 0), 1.0);
    }
}

#[test]
fn very_wide_curve_still_interpolates() {
    let mut config = simple(0.0, 1000.0);
    config.curves[0].points = vec![(-3e38, 0.2), (3e38, 1.0)];
    assert_eq!(config.validate(), Ok(()));
    let c = &config.curves[0];
    assert!(close(c.duty_at(-3e38), 0.2));
    assert!(close(c.duty_at(0.0), 0.6));
    assert!(close(c.duty_at(3e38), 1.0));

    let (mut controller, shared) = bench(1, 1, config);
    set_temp(&shared, 0, Some(100.0));
    let status = controller.tick(1.0);
    assert!(!status.fans[0].failsafe);
    assert!(close(duty(&status, 0), 0.6));
}

#[test]
fn implausible_readings_count_as_unreadable() {
    let config = FanConfig {
        curves: vec![curve("c", &["s0"], 0.0), curve("both", &["s0", "s1"], 0.0)],
        rules: vec![rule("f0", "c", 1000.0), rule("f1", "both", 1000.0)],
    };
    let (mut controller, shared) = bench(2, 2, config);
    set_temp(&shared, 1, Some(40.0));
    let cases = [
        (-20.0, true),
        (-20.5, false),
        (150.0, true),
        (150.5, false),
        (f32::INFINITY, false),
    ];
    for (temp, readable) in cases {
        set_temp(&shared, 0, Some(temp));
        let status = controller.tick(1.0);
        assert_eq!(status.sensors[0].value, readable.then_some(temp), "{temp}");
        assert_eq!(status.fans[0].failsafe, !readable, "{temp}");
        // A curve that also has a working sensor fares no better.
        assert_eq!(status.fans[1].failsafe, !readable, "{temp}");
    }
    set_temp(&shared, 0, Some(30.0));
    assert!(close(duty(&controller.tick(1.0), 1), 0.3));
}

#[test]
fn hysteresis_never_holds_duty_below_the_curve() {
    let mut config = simple(5.0, 1000.0);
    config.curves[0].points = vec![(30.0, 0.8), (50.0, 0.2), (80.0, 1.0)];
    let (mut controller, shared) = bench(1, 1, config);
    let mut at = |temp: f32| {
        set_temp(&shared, 0, Some(temp));
        duty(&controller.tick(1.0), 0)
    };
    assert!(close(at(50.0), 0.2));
    // Cooling here climbs the falling part of the curve, so nothing may be held.
    assert!(close(at(46.0), 0.32));
    assert!(close(at(40.0), 0.5));
    // On the rising part the usual hold still applies.
    assert!(close(at(65.0), 0.6));
    assert!(close(at(62.0), 0.6));
    assert!(close(at(60.0), 0.2 + 0.8 / 3.0));
}

#[test]
fn fan_without_rule_is_never_set() {
    let (mut controller, shared) = bench(1, 2, simple(0.0, 1000.0));
    for _ in 0..5 {
        let status = controller.tick(1.0);
        assert_eq!(status.fans[1].duty, None);
        assert_eq!(status.fans[1].curve, None);
        assert!(!status.fans[1].failsafe);
    }
    drop(controller);
    let bench = shared.lock().unwrap();
    assert_eq!(bench.sets[0].len(), 5);
    assert!(bench.sets[1].is_empty());
    assert_eq!(bench.releases, [1, 0]);
}

#[test]
fn empty_config_touches_nothing() {
    let (mut controller, shared) = bench(1, 2, FanConfig::default());
    controller.tick(1.0);
    drop(controller);
    let bench = shared.lock().unwrap();
    assert!(bench.sets.iter().all(Vec::is_empty));
    assert_eq!(bench.releases, [0, 0]);
}

#[test]
fn releases_on_drop_and_only_once() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    controller.tick(1.0);
    assert_eq!(shared.lock().unwrap().releases, [0]);
    drop(controller);
    assert_eq!(shared.lock().unwrap().releases, [1]);

    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    controller.tick(1.0);
    assert_eq!(controller.release_all(), Ok(()));
    assert_eq!(shared.lock().unwrap().releases, [1]);
    drop(controller);
    assert_eq!(shared.lock().unwrap().releases, [1]);
}

#[test]
fn fan_is_released_when_its_rule_goes_away() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    controller.tick(1.0);
    controller.set_config(FanConfig::default()).unwrap();
    let status = controller.tick(1.0);
    assert_eq!(status.fans[0].duty, None);
    controller.tick(1.0);
    let bench = shared.lock().unwrap();
    assert_eq!(bench.sets[0].len(), 1);
    assert_eq!(bench.releases, [1]);
}

#[test]
fn set_duty_error_shows_in_status() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    shared.lock().unwrap().fail_set = true;
    let status = controller.tick(1.0);
    assert_eq!(status.fans[0].error.as_deref(), Some("write failed"));
    assert_eq!(status.fans[0].duty, None);

    shared.lock().unwrap().fail_set = false;
    let status = controller.tick(1.0);
    assert_eq!(status.fans[0].error, None);
    assert!(close(duty(&status, 0), 0.2));

    // The failed write may still have taken the fan over.
    drop(controller);
    assert_eq!(shared.lock().unwrap().releases, [1]);
}

#[test]
fn fan_that_keeps_refusing_writes_is_handed_back() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    controller.tick(1.0);
    shared.lock().unwrap().fail_set = true;
    for _ in 1..WRITE_TRIES {
        let status = controller.tick(1.0);
        assert_eq!(status.fans[0].error.as_deref(), Some("write failed"));
        assert!(close(duty(&status, 0), 0.2));
    }
    assert_eq!(shared.lock().unwrap().releases, [0]);
    let status = controller.tick(1.0);
    assert_eq!(shared.lock().unwrap().releases, [1]);
    assert_eq!(status.fans[0].duty, None);
    assert_eq!(
        status.fans[0].error.as_deref(),
        Some("write failed; fan handed back to the firmware")
    );

    // Left alone for a while, even though a write would work again.
    shared.lock().unwrap().fail_set = false;
    let status = controller.tick(RETRY_AFTER - 1.0);
    assert_eq!(status.fans[0].duty, None);
    assert!(status.fans[0].error.is_some());
    assert_eq!(shared.lock().unwrap().sets[0].len(), 1);

    let status = controller.tick(1.0);
    assert!(close(duty(&status, 0), 0.2));
    assert_eq!(status.fans[0].error, None);
    drop(controller);
    assert_eq!(shared.lock().unwrap().releases, [2]);
}

#[test]
fn fan_that_refuses_writes_and_release_stays_marked() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    {
        let mut bench = shared.lock().unwrap();
        (bench.fail_set, bench.fail_release) = (true, true);
    }
    for _ in 1..WRITE_TRIES {
        controller.tick(1.0);
    }
    let status = controller.tick(1.0);
    assert_eq!(
        status.fans[0].error.as_deref(),
        Some("write failed; release failed")
    );

    // Every tick tries again until the fan lets go.
    shared.lock().unwrap().fail_release = false;
    let status = controller.tick(1.0);
    assert_eq!(
        status.fans[0].error.as_deref(),
        Some("write failed; fan handed back to the firmware")
    );
    assert_eq!(shared.lock().unwrap().releases, [1]);
    drop(controller);
    assert_eq!(shared.lock().unwrap().releases, [1]);
}

#[test]
fn swapping_backends_releases_touched_fans_first() {
    let (mut controller, old) = bench(1, 2, simple(0.0, 1000.0));
    controller.tick(1.0);
    let new = rig(2, 1);
    assert_eq!(
        controller.set_backends(vec![Box::new(BenchBackend(new.clone()))]),
        Ok(())
    );
    assert_eq!(old.lock().unwrap().releases, [1, 0]);

    // The config carries over to the new hardware.
    let status = controller.tick(1.0);
    assert_eq!((status.sensors.len(), status.fans.len()), (2, 1));
    assert!(close(duty(&status, 0), 0.2));
    drop(controller);
    assert_eq!(old.lock().unwrap().releases, [1, 0]);
    assert_eq!(new.lock().unwrap().releases, [1]);
}

#[test]
fn backends_stay_when_a_fan_refuses_release() {
    let (mut controller, old) = bench(1, 1, simple(0.0, 1000.0));
    controller.tick(1.0);
    old.lock().unwrap().fail_release = true;
    let new = rig(1, 1);
    assert_eq!(
        controller.set_backends(vec![Box::new(BenchBackend(new.clone()))]),
        Err("f0: release failed".into())
    );
    controller.tick(1.0);
    assert_eq!(old.lock().unwrap().sets[0].len(), 2);
    assert!(new.lock().unwrap().sets[0].is_empty());
    old.lock().unwrap().fail_release = false;
}

#[test]
fn failed_release_is_reported_and_retried() {
    let (mut controller, shared) = bench(1, 1, simple(0.0, 1000.0));
    controller.tick(1.0);
    shared.lock().unwrap().fail_release = true;
    assert_eq!(controller.release_all(), Err("f0: release failed".into()));
    assert_eq!(shared.lock().unwrap().releases, [0]);

    // With its rule gone the next tick tries again and says why it failed.
    controller.set_config(FanConfig::default()).unwrap();
    let status = controller.tick(1.0);
    assert_eq!(status.fans[0].error.as_deref(), Some("release failed"));
    assert!(status.fans[0].duty.is_some());

    shared.lock().unwrap().fail_release = false;
    drop(controller);
    assert_eq!(shared.lock().unwrap().releases, [1]);
}

#[test]
fn validation_rejects_bad_configs() {
    let good = FanConfig {
        curves: vec![curve("a", &["s0"], 2.0), curve("b", &["s0"], 0.0)],
        rules: vec![rule("f0", "a", 0.2), rule("f1", "b", 0.2)],
    };
    assert_eq!(good.validate(), Ok(()));

    type Spoil = Box<dyn Fn(&mut FanConfig)>;
    let cases: Vec<(&str, Spoil)> = vec![
        (
            "duplicate curve name",
            Box::new(|c| c.curves[1].name = "a".into()),
        ),
        ("no sensors", Box::new(|c| c.curves[0].sensors.clear())),
        ("no points", Box::new(|c| c.curves[0].points.clear())),
        (
            "unsorted points",
            Box::new(|c| c.curves[0].points.swap(0, 2)),
        ),
        (
            "repeated temperature",
            Box::new(|c| c.curves[0].points[1].0 = 30.0),
        ),
        ("duty above 1", Box::new(|c| c.curves[0].points[2].1 = 1.5)),
        ("duty below 0", Box::new(|c| c.curves[0].points[0].1 = -0.1)),
        (
            "NaN temperature",
            Box::new(|c| c.curves[0].points[0].0 = f32::NAN),
        ),
        (
            "infinite temperature",
            Box::new(|c| c.curves[0].points[2].0 = f32::INFINITY),
        ),
        ("NaN duty", Box::new(|c| c.curves[0].points[0].1 = f32::NAN)),
        (
            "negative hysteresis",
            Box::new(|c| c.curves[0].hysteresis = -1.0),
        ),
        (
            "hysteresis above 30",
            Box::new(|c| c.curves[0].hysteresis = 30.5),
        ),
        (
            "NaN hysteresis",
            Box::new(|c| c.curves[0].hysteresis = f32::NAN),
        ),
        (
            "unknown curve",
            Box::new(|c| c.rules[0].curve = "nope".into()),
        ),
        (
            "min above max",
            Box::new(|c| (c.rules[0].min_duty, c.rules[0].max_duty) = (0.9, 0.5)),
        ),
        ("min below 0", Box::new(|c| c.rules[0].min_duty = -0.1)),
        ("max above 1", Box::new(|c| c.rules[0].max_duty = 1.1)),
        ("NaN min", Box::new(|c| c.rules[0].min_duty = f32::NAN)),
        ("NaN max", Box::new(|c| c.rules[0].max_duty = f32::NAN)),
        (
            "two rules for one fan",
            Box::new(|c| c.rules[1].fan = "f0".into()),
        ),
        ("zero ramp", Box::new(|c| c.rules[0].ramp = 0.0)),
        ("negative ramp", Box::new(|c| c.rules[0].ramp = -0.2)),
        ("ramp below 0.01", Box::new(|c| c.rules[0].ramp = 0.009)),
        ("NaN ramp", Box::new(|c| c.rules[0].ramp = f32::NAN)),
        (
            "infinite ramp",
            Box::new(|c| c.rules[0].ramp = f32::INFINITY),
        ),
    ];
    for (what, spoil) in cases {
        let mut bad = good.clone();
        spoil(&mut bad);
        assert!(bad.validate().is_err(), "{what} was accepted");

        let (mut controller, _shared) = bench(1, 2, good.clone());
        assert!(
            controller.set_config(bad.clone()).is_err(),
            "{what} was applied"
        );
        assert_eq!(controller.config(), &good, "{what} replaced the old config");

        let (controller, _shared) = bench(1, 2, bad);
        assert_eq!(
            controller.config(),
            &FanConfig::default(),
            "{what} survived new()"
        );
    }
}

#[test]
fn boundary_values_are_valid() {
    let mut config = simple(30.0, 0.01);
    config.curves[0].points = vec![(0.0, 0.0), (100.0, 1.0)];
    config.rules[0].min_duty = 0.5;
    config.rules[0].max_duty = 0.5;
    assert_eq!(config.validate(), Ok(()));
}

#[test]
fn config_json_round_trip() {
    let config = FanConfig {
        curves: vec![curve("cpu", &["sim/cpu", "sim/gpu"], 3.5)],
        rules: vec![FanRule {
            min_duty: 0.25,
            max_duty: 0.9,
            ..rule("sim/cpu_fan", "cpu", 0.05)
        }],
    };
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(serde_json::from_str::<FanConfig>(&json).unwrap(), config);
}

#[test]
fn config_json_fills_missing_fields() {
    let json = r#"{
        "curves": [{ "name": "cpu", "sensors": ["sim/cpu"], "points": [[30, 0.2], [80, 1.0]] }],
        "rules": [{ "fan": "sim/cpu_fan", "curve": "cpu" }]
    }"#;
    let config: FanConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.curves[0].hysteresis, 2.0);
    assert_eq!(config.curves[0].points, [(30.0, 0.2), (80.0, 1.0)]);
    assert_eq!(config.rules[0].min_duty, 0.0);
    assert_eq!(config.rules[0].max_duty, 1.0);
    assert_eq!(config.rules[0].ramp, 0.2);
    assert_eq!(config.validate(), Ok(()));

    assert_eq!(
        serde_json::from_str::<FanConfig>("{}").unwrap(),
        FanConfig::default()
    );
}

#[test]
fn status_json_round_trip() {
    let (mut controller, shared) = bench(2, 2, simple(0.0, 1000.0));
    set_temp(&shared, 1, None);
    let status = controller.tick(1.0);
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(serde_json::from_str::<Status>(&json).unwrap(), status);
}

fn sim_config() -> FanConfig {
    let curve = |name: &str, sensors: &[&str]| Curve {
        name: name.into(),
        sensors: sensors.iter().map(|s| s.to_string()).collect(),
        points: vec![(35.0, 0.2), (75.0, 1.0)],
        hysteresis: 3.0,
    };
    FanConfig {
        curves: vec![
            curve("cpu", &["sim/cpu"]),
            curve("gpu", &["sim/gpu"]),
            curve("case", &["sim/cpu", "sim/gpu"]),
        ],
        rules: vec![
            rule("sim/cpu_fan", "cpu", 0.2),
            rule("sim/gpu_fan", "gpu", 0.2),
            rule("sim/case_fan", "case", 0.2),
        ],
    }
}

#[test]
fn sim_closed_loop_settles() {
    let sim = SimBackend::default();
    let mut controller = Controller::new(vec![Box::new(sim.clone())], sim_config());
    let idle = controller.tick(1.0);

    sim.set_load(1.0);
    let mut history = Vec::new();
    for _ in 0..600 {
        history.push(controller.tick(1.0));
    }

    let last = history.last().unwrap();
    for (i, sensor) in last.sensors.iter().enumerate() {
        let (now, before) = (sensor.value.unwrap(), idle.sensors[i].value.unwrap());
        assert!(now > before + 10.0, "{} did not heat up", sensor.id);
        // Stopped fans would let it reach 85 or more.
        assert!(now < 70.0, "{} is not being cooled: {now}", sensor.id);
    }
    for (i, fan) in last.fans.iter().enumerate() {
        assert!(
            duty(last, i) > duty(&idle, i),
            "{} did not speed up",
            fan.id
        );
        assert!(fan.rpm.unwrap() > idle.fans[i].rpm.unwrap());
    }

    // Nothing moves any more over the last two minutes.
    for status in &history[480..] {
        for i in 0..last.fans.len() {
            assert_eq!(
                duty(status, i),
                duty(last, i),
                "{} is hunting",
                last.fans[i].id
            );
        }
        for i in 0..last.sensors.len() {
            let drift = status.sensors[i].value.unwrap() - last.sensors[i].value.unwrap();
            assert!(drift.abs() < 0.05, "{} is still moving", last.sensors[i].id);
        }
    }
}

#[test]
fn sim_failsafe_and_release() {
    let sim = SimBackend::default();
    let mut controller = Controller::new(vec![Box::new(sim.clone())], sim_config());
    assert_eq!(sim.fan_duty("sim/cpu_fan"), None);
    controller.tick(1.0);
    assert!(sim.fan_duty("sim/cpu_fan").is_some());

    sim.fail_sensor("sim/cpu", true);
    let status = controller.tick(1.0);
    let by_id = |id: &str| status.fans.iter().find(|fan| fan.id == id).unwrap();
    assert!(by_id("sim/cpu_fan").failsafe);
    assert_eq!(sim.fan_duty("sim/cpu_fan"), Some(1.0));
    // So is the curve that has the lost sensor among others, but not the one without it.
    assert!(by_id("sim/case_fan").failsafe);
    assert_eq!(sim.fan_duty("sim/case_fan"), Some(1.0));
    assert!(!by_id("sim/gpu_fan").failsafe);

    drop(controller);
    for id in ["sim/cpu_fan", "sim/gpu_fan", "sim/case_fan"] {
        assert_eq!(sim.fan_duty(id), None);
    }
}

#[test]
fn sim_only_moves_when_stepped() {
    let sim = SimBackend::default();
    let mut sensors = sim.clone().sensors();
    let before = sensors[0].read().unwrap();
    sim.set_load(1.0);
    assert_eq!(sensors[0].read().unwrap(), before);
    sim.step(5.0);
    assert!(sensors[0].read().unwrap() > before);
}

#[test]
fn a_swinging_sim_heats_up_and_cools_down_by_itself() {
    let sim = SimBackend::swinging();
    let mut sensors = sim.clone().sensors();
    let mut read = || sensors[0].read().unwrap();
    let idle = read();
    for _ in 0..45 {
        sim.step(1.0);
    }
    let busy = read();
    assert!(busy > idle + 20.0, "{idle} to {busy}");
    for _ in 0..45 {
        sim.step(1.0);
    }
    assert!(read() < busy - 20.0);

    // a load set by hand stays put
    sim.set_load(0.0);
    sim.step(1000.0);
    let cold = read();
    sim.step(45.0);
    assert_eq!(read(), cold);
}

fn reading(id: &str, name: &str, value: f32) -> SensorStatus {
    SensorStatus {
        id: id.into(),
        name: name.into(),
        value: Some(value),
        kind: Kind::Temperature,
    }
}

#[test]
fn a_critical_cpu_puts_every_driven_fan_at_full_until_it_cools_off() {
    let mut config = simple(0.0, 0.01);
    config.rules[0].max_duty = 0.5;
    let (mut controller, _) = bench(1, 2, config);
    let at = |controller: &mut Controller, cpu: f32| {
        controller.apply(
            1.0,
            vec![
                reading("s0", "s0", 30.0),
                reading("cpu/package", "CPU", cpu),
            ],
        )
    };
    let calm = at(&mut controller, 80.0);
    assert!(close(duty(&calm, 0), 0.2));
    assert!(!calm.fans[0].failsafe);

    let hot = at(&mut controller, CPU_CRITICAL);
    // Past the rule's maximum and its ramp, but only for the fan that has a rule.
    assert_eq!(duty(&hot, 0), 1.0);
    assert!(hot.fans[0].failsafe);
    assert!(
        hot.fans[0]
            .error
            .as_deref()
            .unwrap()
            .contains("critically hot")
    );
    assert_eq!(hot.fans[1].duty, None);

    let cooling = at(&mut controller, CPU_CRITICAL - CRITICAL_CLEAR + 0.5);
    assert_eq!(duty(&cooling, 0), 1.0);
    let cool = at(&mut controller, CPU_CRITICAL - CRITICAL_CLEAR - 0.5);
    assert!(!cool.fans[0].failsafe);
    assert_eq!(cool.fans[0].error, None);
    assert!(duty(&cool, 0) < 1.0);
}

#[test]
fn only_cpu_and_gpu_temperatures_have_a_critical_limit() {
    let limit = |id: &str, name: &str| critical_limit(&reading(id, name, 0.0));
    assert_eq!(limit("cpu/package", "CPU package"), Some(CPU_CRITICAL));
    assert_eq!(limit("board/it8688e-2e/t0", "CPU"), Some(CPU_CRITICAL));
    assert_eq!(
        limit("gpu0/temperature", "RX 7900 temperature"),
        Some(GPU_CRITICAL)
    );
    assert_eq!(
        limit("gpu0/hotspot", "RX 7900 hotspot"),
        Some(GPU_HOTSPOT_CRITICAL)
    );
    assert_eq!(limit("gpu0/memory", "RX 7900 memory temperature"), None);
    assert_eq!(limit("board/x/t1", "MOS CPU"), None);
    assert_eq!(limit("board/x/t2", "PCH CPU"), None);
    assert_eq!(limit("board/x/t3", "System"), None);
    let watts = SensorStatus {
        kind: Kind::Power,
        ..reading("cpu/power", "CPU", 200.0)
    };
    assert_eq!(critical_limit(&watts), None);
}

#[test]
fn a_hot_gpu_hotspot_needs_its_own_higher_limit() {
    let (mut controller, _) = bench(1, 1, simple(0.0, 1000.0));
    let at = |controller: &mut Controller, hotspot: f32| {
        let sensors = vec![
            reading("s0", "s0", 30.0),
            reading("gpu0/hotspot", "GPU hotspot", hotspot),
            reading("gpu0/memory", "GPU memory temperature", 104.0),
        ];
        controller.apply(1.0, sensors)
    };
    assert!(!at(&mut controller, GPU_CRITICAL + 10.0).fans[0].failsafe);
    assert!(at(&mut controller, GPU_HOTSPOT_CRITICAL).fans[0].failsafe);
}

#[test]
fn no_config_can_keep_a_fan_below_a_fifth() {
    let mut low_max = simple(0.0, 1.0);
    low_max.rules[0].max_duty = 0.19;
    assert!(low_max.validate().unwrap_err().contains("at least 20%"));

    let mut flat = simple(0.0, 1.0);
    flat.curves[0].points = vec![(30.0, 0.0), (90.0, 0.1)];
    assert!(flat.validate().unwrap_err().contains("never run above"));
    flat.rules[0].min_duty = 0.3;
    assert_eq!(flat.validate(), Ok(()));

    // Stopped while cool, still allowed.
    let mut zero_rpm = simple(0.0, 1.0);
    zero_rpm.curves[0].points = vec![(40.0, 0.0), (60.0, 0.0), (80.0, 1.0)];
    zero_rpm.rules[0].max_duty = MIN_MAX_DUTY;
    assert_eq!(zero_rpm.validate(), Ok(()));
}

#[test]
fn old_low_configs_are_lifted_on_load() {
    let mut c = simple(0.0, 1.0);
    c.rules[0].max_duty = 0.1;
    c.curves[0].points = vec![(30.0, 0.0), (90.0, 0.1)];
    c.lift_low_duties();
    assert_eq!(c.rules[0].max_duty, MIN_MAX_DUTY);
    assert_eq!(c.rules[0].min_duty, MIN_MAX_DUTY);
    assert_eq!(c.validate(), Ok(()));
}

struct Pump(Shared);

impl Fan for Pump {
    fn id(&self) -> &str {
        "board/aio"
    }
    fn name(&self) -> &str {
        "AIO pump"
    }
    fn rpm(&mut self) -> Option<u32> {
        None
    }
    fn set_duty(&mut self, duty: f32) -> Result<(), String> {
        self.0.lock().unwrap().sets[0].push(duty);
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        Ok(())
    }
}

struct PumpBackend(Shared);

impl FanBackend for PumpBackend {
    fn name(&self) -> &str {
        "pump"
    }
    fn sensors(&mut self) -> Vec<Box<dyn Sensor>> {
        Vec::new()
    }
    fn fans(&mut self) -> Vec<Box<dyn Fan>> {
        vec![Box::new(Pump(self.0.clone()))]
    }
}

#[test]
fn a_pump_keeps_its_floor() {
    let shared = rig(0, 1);
    let mut config = simple(0.0, 1000.0);
    config.curves[0].points = vec![(30.0, 0.0), (80.0, 1.0)];
    config.rules[0].fan = "board/aio".into();
    let mut controller = Controller::new(vec![Box::new(PumpBackend(shared.clone()))], config);
    let status = controller.apply(1.0, vec![reading("s0", "s0", 20.0)]);
    assert_eq!(duty(&status, 0), PUMP_FLOOR);
    let status = controller.apply(1.0, vec![reading("s0", "s0", 80.0)]);
    assert_eq!(duty(&status, 0), 1.0);
}
