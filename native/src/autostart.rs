//! Starting with Windows, through a sign-in task: Windows holds back Run-key apps after sign-in, tasks it does not.

use std::os::windows::process::CommandExt;
use std::process::Command;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const TASK: &str = "FoxDen";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Given when Windows starts the app at sign-in, which then stays in the tray.
pub const TRAY_ARG: &str = "--tray";

fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("{program}: {e}"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The task: at this user's sign-in, normal priority, no time limit, on battery too.
fn task_xml(user: &str, exe: &str) -> String {
    let (user, exe) = (escape(user), escape(exe));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>4</Priority></Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{TRAY_ARG}</Arguments></Exec></Actions>
</Task>
"#
    )
}

fn user() -> String {
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let name = std::env::var("USERNAME").unwrap_or_default();
    if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    }
}

/// Adds or removes the sign-in task. The Run value older versions used is always removed.
pub fn set(enabled: bool) -> Result<(), String> {
    let _ = run("reg", &["delete", RUN_KEY, "/v", "FoxDen", "/f"]);
    // Left by versions released as FoxRGB.
    let _ = run("reg", &["delete", RUN_KEY, "/v", "FoxRGB", "/f"]);
    let _ = run("schtasks", &["/Delete", "/TN", "FoxRGB", "/F"]);
    if !enabled {
        let _ = run("schtasks", &["/Delete", "/TN", TASK, "/F"]);
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    create(TASK, &exe.display().to_string())
}

fn create(name: &str, exe: &str) -> Result<(), String> {
    let xml = task_xml(&user(), exe);
    let file = std::env::temp_dir().join("foxden-task.xml");
    let wide: Vec<u8> = std::iter::once(0xFEFFu16)
        .chain(xml.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(&file, wide).map_err(|e| e.to_string())?;
    let path = file.display().to_string();
    let output = run("schtasks", &["/Create", "/TN", name, "/XML", &path, "/F"]);
    let _ = std::fs::remove_file(&file);
    let output = output?;
    if !output.status.success() {
        return Err(format!(
            "schtasks: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Creates a real task under a throwaway name: run with --ignored.
    #[test]
    #[ignore]
    fn a_real_task_is_created() {
        let name = "FoxDen-selftest";
        super::create(name, r"C:\Windows\System32\cmd.exe").unwrap();
        let shown = super::run("schtasks", &["/Query", "/TN", name, "/XML"]).unwrap();
        let _ = super::run("schtasks", &["/Delete", "/TN", name, "/F"]);
        let xml = String::from_utf8_lossy(&shown.stdout).to_string();
        assert!(
            xml.contains("LogonTrigger") && xml.contains("--tray"),
            "{xml}"
        );
    }

    #[test]
    fn the_task_starts_the_app_in_the_tray_and_escapes_the_path() {
        let xml = super::task_xml(r"PC\me", r"C:\A & B\foxden.exe");
        assert!(xml.contains(r"<Command>C:\A &amp; B\foxden.exe</Command>"));
        assert!(xml.contains("<Arguments>--tray</Arguments>"));
        assert!(xml.contains(r"<UserId>PC\me</UserId>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
    }
}
