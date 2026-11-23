fn main() {
    slint_build::compile("ui/app.slint").expect("slint");
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id("icons/icon.ico", "1");
    res.compile().expect("resources");
}
