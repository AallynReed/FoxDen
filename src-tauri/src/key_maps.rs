//! Saved key colour maps, shareable as a short code or a .foxkeys file holding one.

use crate::share;
use crate::state::{Shared, update};
use foxden_core::{Color, clean_key_map, from_hex, to_hex};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

pub const PREFIX: &str = "foxden-keys:";
const VERSION: u32 = 1;
pub const MAX_CODE: usize = 4000;
pub const MAX_MAPS: usize = 32;
const MAX_KEYS: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyMap {
    pub name: String,
    #[serde(default)]
    pub map: Vec<(u32, Color)>,
}

impl KeyMap {
    pub fn clean(self) -> KeyMap {
        KeyMap {
            name: share::clean_name(&self.name, "Shared key map"),
            map: clean_key_map(self.map.into_iter().take(MAX_KEYS).collect()),
        }
    }
}

/// The share code's JSON: each colour as hex with its keys, so a board of few colours stays short.
#[derive(Serialize, Deserialize)]
struct Share {
    v: u32,
    n: String,
    c: Vec<(String, Vec<u32>)>,
}

pub fn encode(m: &KeyMap) -> String {
    let mut groups: Vec<(Color, Vec<u32>)> = Vec::new();
    for &(vk, c) in &m.map {
        match groups.iter_mut().find(|(seen, _)| *seen == c) {
            Some((_, keys)) => keys.push(vk),
            None => groups.push((c, vec![vk])),
        }
    }
    let body = Share {
        v: VERSION,
        n: m.name.clone(),
        c: groups.into_iter().map(|(c, k)| (to_hex(&[c]), k)).collect(),
    };
    share::encode(PREFIX, &body)
}

/// Reads a share code. It is only ever data: checked, clamped and bounded.
pub fn decode(code: &str) -> Result<KeyMap, String> {
    let s: Share = share::decode(code, PREFIX, MAX_CODE, 1, "key map")?;
    let mut map = Vec::new();
    for (hex, keys) in s.c.into_iter().take(MAX_KEYS) {
        let [c] = from_hex(&hex)
            .ok()
            .and_then(|c| <[Color; 1]>::try_from(c).ok())
            .ok_or("The share code has a bad colour")?;
        map.extend(keys.into_iter().take(MAX_KEYS).map(|vk| (vk, c)));
    }
    Ok(KeyMap { name: s.n, map }.clean())
}

/// Adds or replaces a saved map by name.
fn store(list: &mut Vec<KeyMap>, m: KeyMap) -> Result<(), String> {
    let full = list.len() >= MAX_MAPS;
    match list.iter_mut().find(|x| x.name == m.name) {
        Some(x) => *x = m,
        None if full => return Err("Too many key maps; delete one first".into()),
        None => list.push(m),
    }
    Ok(())
}

#[tauri::command]
pub fn save_key_map(app: AppHandle, shared: State<'_, Shared>, map: KeyMap) -> Result<(), String> {
    update(&app, &shared, |s| {
        store(&mut s.config.key_maps, map.clean())
    })
}

#[tauri::command]
pub fn delete_key_map(
    app: AppHandle,
    shared: State<'_, Shared>,
    name: String,
) -> Result<(), String> {
    update(&app, &shared, |s| {
        let before = s.config.key_maps.len();
        s.config.key_maps.retain(|m| m.name != name);
        if s.config.key_maps.len() == before {
            return Err(format!("no key map named {name}"));
        }
        Ok(())
    })
}

#[tauri::command]
pub fn key_map_code(map: KeyMap) -> String {
    encode(&map.clean())
}

#[tauri::command]
pub fn key_map_decode(code: String) -> Result<KeyMap, String> {
    decode(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use foxden_core::key_layout;

    #[test]
    fn a_fully_painted_board_round_trips_under_the_cap() {
        let map: Vec<(u32, Color)> = key_layout()
            .iter()
            .enumerate()
            .map(|(i, k)| (k.vk, Color(i as u8 * 2, 255 - i as u8, i as u8)))
            .collect();
        let m = KeyMap {
            name: "Every key its own colour".into(),
            map,
        }
        .clean();
        let code = encode(&m);
        assert!(
            code.starts_with(PREFIX) && code.len() < MAX_CODE,
            "{}",
            code.len()
        );
        assert_eq!(decode(&code).unwrap(), m);
    }

    #[test]
    fn codes_are_checked_and_cleaned() {
        let wrap = |json: &str| format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json));
        assert!(decode("hello").unwrap_err().contains("key map"));
        assert!(
            decode(&wrap(r#"{"v":2,"n":"x","c":[]}"#))
                .unwrap_err()
                .contains("newer")
        );
        assert!(decode(&wrap(r#"{"v":1,"n":"x","c":[["zz0000",[65]]]}"#)).is_err());
        assert!(decode(&wrap(r#"{"v":1,"n":"x","c":[["ff000000ff00",[65]]]}"#)).is_err());
        assert!(decode(&format!("{PREFIX}{}", "A".repeat(MAX_CODE))).is_err());
        let m = decode(&wrap(
            r#"{"v":1,"n":" \u0007 ","c":[["ff0000",[65,9999,16,65]],["0000ff",[87]]]}"#,
        ))
        .unwrap();
        assert_eq!(m.name, "Shared key map");
        assert_eq!(m.map, [(65, Color(255, 0, 0)), (87, Color(0, 0, 255))]);
    }

    #[test]
    fn saved_maps_are_replaced_by_name_and_capped() {
        let m = |name: &str| KeyMap {
            name: name.into(),
            map: Vec::new(),
        };
        let mut list = Vec::new();
        for i in 0..MAX_MAPS {
            store(&mut list, m(&i.to_string())).unwrap();
        }
        store(&mut list, m("0")).unwrap();
        assert!(store(&mut list, m("more")).is_err());
        assert_eq!(list.len(), MAX_MAPS);
    }
}
