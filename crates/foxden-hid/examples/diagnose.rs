fn main() {
    for line in foxden_hid::diagnose() {
        println!("{line}");
    }
}
