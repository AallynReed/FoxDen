//! Tray menu and notification text in the UI's language, from the same files the UI uses.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::AppHandle;

use crate::state::lock;

/// Language of the loaded table, and the table itself.
static TABLE: Mutex<Option<(String, HashMap<String, String>)>> = Mutex::new(None);
/// The Language setting, kept here so translating never waits on the app state.
static CHOICE: Mutex<String> = Mutex::new(String::new());

pub fn set_choice(choice: &str) {
    *lock(&CHOICE) = choice.to_string();
}

/// The language FoxDen shows: the setting, or the Windows display language for "auto".
pub fn language(choice: &str) -> String {
    if choice != "auto" && !choice.is_empty() {
        return choice.to_string();
    }
    pick(&windows_language()).unwrap_or("en").to_string()
}

/// Any Portuguese gets Portugal's; Chinese splits by script.
fn pick(tag: &str) -> Option<&'static str> {
    let t = tag.to_lowercase();
    if t.starts_with("zh") {
        let hant = ["hant", "tw", "hk", "mo"].iter().any(|k| t.contains(k));
        return Some(if hant { "zh-Hant" } else { "zh-Hans" });
    }
    if t.starts_with("pt") {
        return Some("pt-PT");
    }
    let base = t.split('-').next().unwrap_or("");
    [
        "en", "es", "de", "fr", "ru", "pl", "tr", "ja", "ko", "it", "uk", "nl", "cs",
    ]
    .into_iter()
    .find(|l| *l == base)
}

fn windows_language() -> String {
    use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    let mut buf = [0u16; 85];
    let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, buf.as_mut_ptr(), 85, 0) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

/// `text` in the chosen language, or as it is when there is no translation.
pub fn tr(app: &AppHandle, text: &str) -> String {
    let choice = lock(&CHOICE).clone();
    let lang = language(&choice);
    if lang == "en" {
        return text.to_string();
    }
    let mut table = lock(&TABLE);
    if table.as_ref().is_none_or(|(l, _)| *l != lang) {
        let loaded = app
            .asset_resolver()
            .get(format!("lang/{lang}.json"))
            .and_then(|a| serde_json::from_slice(&a.bytes).ok())
            .unwrap_or_default();
        *table = Some((lang, loaded));
    }
    let map = &table.as_ref().expect("loaded above").1;
    if let Some(out) = map.get(text) {
        return out.clone();
    }
    map.iter()
        .filter(|(k, _)| k.contains("{0}"))
        .find_map(|(k, v)| fill(k, v, text))
        .unwrap_or_else(|| text.to_string())
}

/// Matches `text` against an English template such as "Battery low ({0})." and fills the translation.
fn fill(template: &str, out: &str, text: &str) -> Option<String> {
    let mut slots = Vec::new();
    let mut literals = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}')? + open;
        literals.push(&rest[..open]);
        slots.push(&rest[open..=close]);
        rest = &rest[close + 1..];
    }
    literals.push(rest);
    let mut values = Vec::new();
    let mut at = text.strip_prefix(literals[0])?;
    for (i, lit) in literals.iter().enumerate().skip(1) {
        let end = if i == literals.len() - 1 {
            at.strip_suffix(lit).map(str::len)?
        } else if lit.is_empty() {
            return None;
        } else {
            at.find(lit)?
        };
        values.push(&at[..end]);
        at = &at[end + lit.len()..];
    }
    let mut filled = out.to_string();
    for (slot, value) in slots.iter().zip(values) {
        filled = filled.replace(slot, value);
    }
    Some(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_fill_in_order() {
        assert_eq!(
            fill(
                "Battery low ({0}).",
                "Bateria fraca ({0}).",
                "Battery low (20%)."
            )
            .as_deref(),
            Some("Bateria fraca (20%).")
        );
        assert_eq!(
            fill("{0} of {1}", "{1}: {0}", "3 of 5").as_deref(),
            Some("5: 3")
        );
        assert!(fill("Battery low ({0}).", "x", "Battery full").is_none());
    }

    #[test]
    fn languages() {
        assert_eq!(pick("pt-BR"), Some("pt-PT"));
        assert_eq!(pick("zh-TW"), Some("zh-Hant"));
        assert_eq!(pick("zh-CN"), Some("zh-Hans"));
        assert_eq!(pick("de-AT"), Some("de"));
        assert_eq!(pick("sv-SE"), None);
    }
}
