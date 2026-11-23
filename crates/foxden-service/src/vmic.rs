//! VB-CABLE for the virtual microphone, run from the command line or for the app over the pipe.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use foxden_vcable::{self as vc, Installed};

fn files() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no program folder")?;
    Ok(vc::files_dir(dir))
}

/// Installs, removes or renames, and says what happened. Needs administrator rights.
pub fn act(action: &str) -> Result<String, String> {
    vc::com_init();
    match action {
        "install" => Ok(match vc::install(&files()?)? {
            Installed::Reused => "VB-CABLE was already installed; it is used as it is",
            Installed::Ready => "VB-CABLE installed",
            Installed::Reboot => "VB-CABLE installed; restart Windows to finish",
        }
        .into()),
        "remove" => Ok(if vc::remove(&files()?)? {
            "VB-CABLE removed; restart Windows to finish"
        } else {
            "VB-CABLE was not installed by FoxDen, so it stays"
        }
        .into()),
        "rename" => vc::rename().map(|n| format!("{n} endpoints renamed")),
        "status" => Ok(format!("{:?}", vc::status())),
        _ => Err(format!("unknown virtual mic action {action}")),
    }
}

/// As `act`, but VB-Audio's setup must sit where only administrators can change it.
pub fn act_as_service(action: &str) -> Result<String, String> {
    if matches!(action, "install" | "remove") {
        let setup = files()?.join(vc::SETUP);
        if setup.exists() {
            crate::guard::locked_down(&setup)?;
        }
    }
    act(action)
}

/// Names the endpoints again if an update of VB-CABLE reset them.
pub fn reapply(log: impl Fn(&str)) {
    vc::com_init();
    if !vc::driver_present() {
        return;
    }
    // At boot the endpoints can appear well after the service starts.
    let start = Instant::now();
    while !vc::status().ready() {
        if start.elapsed() > Duration::from_secs(90) {
            log("virtual mic: VB-CABLE's endpoints did not appear, so they keep their names");
            return;
        }
        thread::sleep(Duration::from_secs(2));
    }
    match vc::rename() {
        Ok(0) => {}
        Ok(n) => log(&format!("virtual mic: {n} endpoints renamed")),
        Err(e) => log(&format!("virtual mic: cannot rename the endpoints: {e}")),
    }
}
