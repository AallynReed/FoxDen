fn main() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id("../src-tauri/icons/icon.ico", "1");
    res.compile().expect("resources");
}
