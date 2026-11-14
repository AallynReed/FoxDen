fn main() {
    for line in foxrgb_hid::diagnose() {
        println!("{line}");
    }
}
