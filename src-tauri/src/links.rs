//! The project's own links, opened in the default browser.

pub const DISCORD: &str = "https://discord.gg/VGTaqW3vwn";
const KOFI: &str = "https://ko-fi.com/aallyn";
const PAYPAL: &str = "https://paypal.me/waterin";

pub fn open(url: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Only these links can be opened from the UI.
#[tauri::command]
pub fn open_link(which: String) -> Result<(), String> {
    match which.as_str() {
        "discord" => open(DISCORD),
        "kofi" => open(KOFI),
        "paypal" => open(PAYPAL),
        "vbaudio" => open(foxden_vcable::DONATE),
        _ => Err("unknown link".into()),
    }
}
