use std::os::windows::process::CommandExt;
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Adds or removes the FoxRGB value under the current user's Run key.
pub fn set(enabled: bool) -> Result<(), String> {
    let mut reg = Command::new("reg");
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        reg.args(["add", RUN_KEY, "/v", "FoxRGB", "/t", "REG_SZ", "/f", "/d"]);
        reg.arg(format!("\"{}\"", exe.display()));
    } else {
        reg.args(["delete", RUN_KEY, "/v", "FoxRGB", "/f"]);
    }
    let output = reg
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("reg: {e}"))?;
    // Removing a value that is not there fails, which is fine.
    if enabled && !output.status.success() {
        return Err(format!(
            "reg: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}
