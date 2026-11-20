//! Prints the HID collections of the supported devices. Opens nothing and sends nothing.

fn main() {
    for line in foxden_hid::list() {
        println!("{line}");
    }
}
