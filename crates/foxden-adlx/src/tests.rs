use super::*;

#[derive(Debug, PartialEq)]
enum Call {
    Curve(Vec<Point>),
    Zero(bool),
}

struct Mock {
    supported: bool,
    curve: Vec<Point>,
    zero: Option<bool>,
    fail_curve: bool,
    calls: Vec<Call>,
}

fn driver_curve() -> Vec<Point> {
    vec![
        Point {
            speed: 20,
            temperature: 50,
        },
        Point {
            speed: 60,
            temperature: 80,
        },
    ]
}

fn mock(supported: bool) -> CardFan<Mock> {
    CardFan::new(Mock {
        supported,
        curve: driver_curve(),
        zero: Some(true),
        fail_curve: false,
        calls: Vec::new(),
    })
}

impl Tuning for Mock {
    fn speed_range(&self) -> Range {
        Range {
            min: 15,
            max: 100,
            step: 1,
        }
    }
    fn temperature_range(&self) -> Range {
        Range {
            min: 25,
            max: 100,
            step: 1,
        }
    }
    fn zero_rpm_supported(&self) -> bool {
        self.supported
    }
    fn curve(&mut self) -> Result<Vec<Point>, String> {
        Ok(self.curve.clone())
    }
    fn set_curve(&mut self, points: &[Point]) -> Result<(), String> {
        if self.fail_curve {
            return Err("refused".into());
        }
        self.curve = points.to_vec();
        self.calls.push(Call::Curve(points.to_vec()));
        Ok(())
    }
    fn zero_rpm(&mut self) -> Option<bool> {
        self.zero
    }
    fn set_zero_rpm(&mut self, on: bool) -> Result<(), String> {
        if !self.supported && on {
            return Err("unsupported".into());
        }
        self.zero = Some(on);
        self.calls.push(Call::Zero(on));
        Ok(())
    }
}

fn flat(speed: i32) -> Call {
    Call::Curve(
        [37, 50, 62, 75, 87]
            .into_iter()
            .map(|temperature| Point { speed, temperature })
            .collect(),
    )
}

#[test]
fn zero_duty_hands_the_driver_curve_back_with_zero_rpm_on() {
    let mut fan = mock(true);
    fan.set_duty(0.0).unwrap();
    assert_eq!(
        fan.card.calls,
        [Call::Curve(driver_curve()), Call::Zero(true)]
    );
}

#[test]
fn rising_duty_leaves_zero_rpm() {
    let mut fan = mock(true);
    fan.set_duty(0.0).unwrap();
    fan.card.calls.clear();
    fan.set_duty(0.5).unwrap();
    assert_eq!(fan.card.calls, [flat(50), Call::Zero(false)]);
    fan.card.calls.clear();
    fan.set_duty(0.0).unwrap();
    assert_eq!(
        fan.card.calls,
        [Call::Curve(driver_curve()), Call::Zero(true)]
    );
}

#[test]
fn one_percent_runs_at_the_minimum() {
    let mut fan = mock(true);
    fan.set_duty(0.0).unwrap();
    fan.card.calls.clear();
    fan.set_duty(0.01).unwrap();
    assert_eq!(fan.card.calls, [flat(15), Call::Zero(false)]);
}

#[test]
fn unchanged_duty_writes_nothing() {
    let mut fan = mock(true);
    fan.set_duty(0.0).unwrap();
    fan.set_duty(0.004).unwrap();
    fan.set_duty(0.4).unwrap();
    fan.card.calls.clear();
    fan.set_duty(0.4).unwrap();
    fan.set_duty(0.401).unwrap();
    assert!(fan.card.calls.is_empty());
}

#[test]
fn zero_duty_without_zero_rpm_runs_at_the_minimum() {
    let mut fan = mock(false);
    fan.set_duty(0.0).unwrap();
    assert_eq!(fan.card.calls, [flat(15), Call::Zero(false)]);
}

#[test]
fn release_restores_curve_and_zero_rpm() {
    let mut fan = mock(true);
    fan.card.zero = Some(false);
    fan.set_duty(0.0).unwrap();
    fan.set_duty(0.7).unwrap();
    fan.card.calls.clear();
    fan.release().unwrap();
    assert_eq!(
        fan.card.calls,
        [Call::Curve(driver_curve()), Call::Zero(false)]
    );
    fan.card.calls.clear();
    fan.release().unwrap();
    assert!(fan.card.calls.is_empty());
}

#[test]
fn refused_curve_still_restores_zero_rpm_and_retries() {
    let mut fan = mock(true);
    fan.set_duty(0.7).unwrap();
    fan.card.calls.clear();
    fan.card.fail_curve = true;
    assert!(fan.release().is_err());
    assert_eq!(fan.card.calls, [Call::Zero(true)]);
    fan.card.fail_curve = false;
    fan.card.calls.clear();
    fan.release().unwrap();
    assert_eq!(
        fan.card.calls,
        [Call::Curve(driver_curve()), Call::Zero(true)]
    );
}

#[test]
fn release_without_writes_does_nothing() {
    let mut fan = mock(true);
    fan.release().unwrap();
    assert!(fan.card.calls.is_empty());
}
