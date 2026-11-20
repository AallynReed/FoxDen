fn main() {
    let start = std::time::Instant::now();
    for b in foxden_battery::poll() {
        println!("{b:?}");
    }
    println!("took {:?}", start.elapsed());
}
