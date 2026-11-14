//! Runs the controller against the simulated backend through a load step.

use foxrgb_fan::{Controller, Curve, FanConfig, FanRule, SimBackend};

fn curve(name: &str, sensors: &[&str]) -> Curve {
    Curve {
        name: name.into(),
        sensors: sensors.iter().map(|s| s.to_string()).collect(),
        points: vec![(35.0, 0.2), (75.0, 1.0)],
        hysteresis: 3.0,
    }
}

fn rule(fan: &str, curve: &str) -> FanRule {
    FanRule {
        fan: fan.into(),
        curve: curve.into(),
        min_duty: 0.2,
        max_duty: 1.0,
        ramp: 0.2,
    }
}

fn main() {
    let sim = SimBackend::default();
    let config = FanConfig {
        curves: vec![
            curve("cpu", &["sim/cpu"]),
            curve("gpu", &["sim/gpu"]),
            curve("case", &["sim/cpu", "sim/gpu"]),
        ],
        rules: vec![
            rule("sim/cpu_fan", "cpu"),
            rule("sim/gpu_fan", "gpu"),
            rule("sim/case_fan", "case"),
        ],
    };
    let mut controller = Controller::new(vec![Box::new(sim.clone())], config);

    let mut load = 0.1;
    for second in 0..150 {
        match second {
            20 => load = 1.0,
            100 => load = 0.1,
            _ => {}
        }
        sim.set_load(load);
        let status = controller.tick(1.0);

        let mut line = format!("{second:>4}s  load {:>3.0}%", load * 100.0);
        for sensor in &status.sensors {
            match sensor.value {
                Some(temp) => line += &format!("  {} {temp:>5.1}C", sensor.name),
                None => line += &format!("  {} --", sensor.name),
            }
        }
        for fan in &status.fans {
            let duty = fan.duty.map_or(0.0, |duty| duty * 100.0);
            let rpm = fan.rpm.unwrap_or(0);
            line += &format!("  {} {duty:>3.0}% {rpm:>4} rpm", fan.name);
        }
        println!("{line}");
    }
}
