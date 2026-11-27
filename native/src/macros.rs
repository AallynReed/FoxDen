//! Key and mouse macros on hotkeys, kept in `macros.json` beside the config so profiles leave them alone.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use foxden_macros::win::Host;
use foxden_macros::{Entry, Hotkey, Library, Step};
use tauri::{AppHandle, Emitter, Manager};

use crate::log;
use crate::state::lock;

pub struct Macros {
    lib: Arc<Mutex<Library>>,
    host: Host,
    path: PathBuf,
}

impl Macros {
    pub fn open(path: PathBuf) -> Self {
        let lib = Arc::new(Mutex::new(Library::load(&path)));
        Macros { host: Host::new(lib.clone()), lib, path }
    }

    pub fn library(&self) -> Library {
        lock(&self.lib).clone()
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    fn change<T>(&self, f: impl FnOnce(&mut Library) -> Result<T, String>) -> Result<T, String> {
        let mut lib = lock(&self.lib);
        let mut next = lib.clone();
        let out = f(&mut next)?;
        next.save(&self.path).map_err(|e| format!("Couldn't save macros: {e}"))?;
        *lib = next;
        drop(lib);
        self.host.refresh_bindings();
        Ok(out)
    }

    /// Turns hotkeys on from a saved library; called once at start.
    pub fn resume(&self) {
        if lock(&self.lib).enabled {
            match self.host.set_bindings(true) {
                Ok(()) => log::info("macros", "Macro hotkeys are on"),
                Err(e) => log::warn("macros", e),
            }
        }
    }

    pub fn put(&self, entry: Entry) -> Result<String, String> {
        self.change(|l| l.put(entry))
    }

    /// Changes one entry and saves it if it still checks out.
    pub fn edit(&self, id: &str, f: impl FnOnce(&mut Entry)) -> Result<(), String> {
        self.change(|l| {
            let mut e = l.get(id).cloned().ok_or("No such macro.")?;
            f(&mut e);
            l.put(e).map(|_| ())
        })
    }

    pub fn note_read(&self) -> Result<(), String> {
        self.change(|l| {
            l.noted = true;
            Ok(())
        })
    }

    pub fn cancel_catch(&self) {
        self.host.cancel_catch();
    }

    pub fn remove(&self, id: &str) -> Result<Entry, String> {
        self.host.stop(id);
        self.change(|l| l.remove(id).ok_or_else(|| "No such macro.".to_string()))
    }

    pub fn set_enabled(&self, on: bool) -> Result<(), String> {
        self.host.set_bindings(on)?;
        self.change(|l| {
            l.enabled = on;
            Ok(())
        })?;
        log::info("macros", if on { "Macro hotkeys are on" } else { "Macro hotkeys are off" });
        Ok(())
    }

    pub fn play(&self, id: &str, delay: Duration) -> Result<(), String> {
        self.host.play_now(id, delay)
    }

    pub fn stop_all(&self) {
        self.host.stop_all();
    }

    pub fn start_recording(&self) -> Result<(), String> {
        self.host.start_recording()
    }

    pub fn stop_recording(&self) -> Vec<Step> {
        self.host.stop_recording()
    }

    pub fn catch_hotkey(&self, done: impl FnOnce(Option<Hotkey>) + Send + 'static) -> Result<(), String> {
        self.host.catch_hotkey(done)
    }

    /// The entry with this id, or else the one with this name.
    pub fn find(&self, id_or_name: &str) -> Option<Entry> {
        let lib = lock(&self.lib);
        lib.get(id_or_name)
            .or_else(|| lib.entries.iter().find(|e| e.macro_.name.eq_ignore_ascii_case(id_or_name.trim())))
            .cloned()
    }
}

pub fn start(app: &AppHandle) {
    let m = Macros::open(crate::config::dir().join("macros.json"));
    m.resume();
    app.manage(m);
}

pub fn get(app: &AppHandle) -> Result<tauri::State<'static, Macros>, String> {
    app.try_state::<Macros>().ok_or_else(|| "FoxDen is still starting.".into())
}

/// Tells the window the macros changed.
pub fn changed(app: &AppHandle) {
    let _ = app.emit("macros", serde_json::Value::Null);
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use foxden_macros::{Input, Macro};

    pub fn temp() -> (Macros, PathBuf) {
        let dir = std::env::temp_dir().join(format!("foxden-macros-test-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Macros::open(dir.join("macros.json")), dir)
    }

    #[test]
    fn changes_are_saved_and_reloaded() {
        let (m, dir) = temp();
        let hotkey = Some(Hotkey { input: Input::Key(0x68), mods: 0 });
        let id = m.put(Entry { id: String::new(), macro_: Macro::chord("Copy", &[0xE0], 0x06), hotkey, block: true }).unwrap();
        assert!(m.put(Entry { id: String::new(), macro_: Macro::chord("Again", &[], 0x04), hotkey, block: false }).is_err());
        assert_eq!(Macros::open(dir.join("macros.json")).library().entries.len(), 1);
        assert_eq!(m.find("copy").unwrap().id, id);
        m.remove(&id).unwrap();
        assert!(m.remove(&id).is_err());
        assert!(Macros::open(dir.join("macros.json")).library().entries.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let (m, dir) = temp();
        std::fs::create_dir_all(dir.join("macros.json")).unwrap();
        let entry = Entry { id: String::new(), macro_: Macro::chord("Copy", &[], 0x06), hotkey: None, block: false };
        assert!(m.put(entry).unwrap_err().contains("Couldn't save"));
        assert!(m.library().entries.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
