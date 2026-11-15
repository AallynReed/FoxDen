fn main() {
    let card = match foxrgb_adlx::Card::open() {
        Ok(card) => card,
        Err(e) => return println!("{e}"),
    };
    println!("speed range {:?}", card.speed_range);
    println!("temperature range {:?}", card.temperature_range);
    println!("zero rpm: {:?}", card.zero_rpm());
    let curve = card.curve();
    println!("curve: {curve:?}");
    if std::env::args().nth(1).as_deref() != Some("test") {
        return;
    }
    let original = curve.expect("curve");
    let zero = card.zero_rpm();
    println!("zero rpm off: {:?}", card.set_zero_rpm(false));
    let flat: Vec<_> = original
        .iter()
        .map(|p| foxrgb_adlx::Point {
            speed: 100,
            temperature: p.temperature,
        })
        .collect();
    println!("flat 100: {:?}", card.set_curve(&flat));
    println!("now: {:?}", card.curve());
    std::thread::sleep(std::time::Duration::from_secs(10));
    println!("restore: {:?}", card.set_curve(&original));
    if let Some(on) = zero {
        println!("zero rpm back: {:?}", card.set_zero_rpm(on));
    }
    println!("now: {:?} zero rpm {:?}", card.curve(), card.zero_rpm());
}
