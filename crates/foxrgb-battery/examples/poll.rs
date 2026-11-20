fn main() {
    let start = std::time::Instant::now();
    for b in foxrgb_battery::poll() {
        println!("{b:?}");
    }
    println!("took {:?}", start.elapsed());
}
