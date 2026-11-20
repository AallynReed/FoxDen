//! The phone linked in Microsoft Phone Link, from the Start menu card it keeps on disk. Read-only.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::{Battery, Ctx, Kind};

const CONNECTED: [(&str, Lang); 2] = [
    (" - Connected to your PC", Lang::En),
    (" - PC に接続しました", Lang::Ja),
];
const BATTERY_EN: &str = "Battery ";
const BATTERY_JA: &str = "バッテリー 残量 ";
/// A card not rewritten for this long is a last known value.
const STALE: Duration = Duration::from_secs(3 * 60 * 60);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Lang {
    En,
    Ja,
}

#[derive(Debug, PartialEq)]
struct Reading {
    name: String,
    percent: u8,
    charging: bool,
}

/// "Battery 57% remaining[, charging]" or the Japanese "バッテリー 残量 57%".
fn tooltip_level(tip: &str, lang: Lang) -> Option<(u8, bool)> {
    let rest = tip.strip_prefix(if lang == Lang::En {
        BATTERY_EN
    } else {
        BATTERY_JA
    })?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=3).contains(&digits) {
        return None;
    }
    let level: u8 = rest[..digits].parse().ok().filter(|&v| v <= 100)?;
    let tail = rest[digits..].strip_prefix('%')?;
    let charging = match (lang, tail) {
        (Lang::En, " remaining") | (Lang::Ja, "") => false,
        (Lang::En, " remaining, charging") => true,
        _ => return None,
    };
    Some((level, charging))
}

/// Walks only the card layout (body, items, columns). `Err` when two elements disagree.
fn find_battery(v: &Value, lang: Lang, found: &mut Option<(u8, bool)>) -> Result<(), ()> {
    match v {
        Value::Array(items) => items.iter().try_for_each(|c| find_battery(c, lang, found)),
        Value::Object(o) => {
            let ty = o.get("type").and_then(Value::as_str);
            if ty.is_some_and(|t| {
                !matches!(
                    t,
                    "AdaptiveCard" | "Container" | "ColumnSet" | "Column" | "TextBlock" | "Image"
                )
            }) {
                return Ok(());
            }
            if let Some(tip) = o.get("tooltip").and_then(Value::as_str)
                && let Some((n, chg)) = tooltip_level(tip, lang)
                && o.get("text")
                    .and_then(Value::as_str)
                    .is_none_or(|t| t == format!("{n}%"))
            {
                if found.is_some_and(|f| f != (n, chg)) {
                    return Err(());
                }
                *found = Some((n, chg));
            }
            ["body", "items", "columns"]
                .iter()
                .filter_map(|k| o.get(*k))
                .try_for_each(|c| find_battery(c, lang, found))
        }
        _ => Ok(()),
    }
}

fn parse(json: &str) -> Option<Reading> {
    let root: Value = serde_json::from_str(json).ok()?;
    let speak = root.as_object()?.get("speak")?.as_str()?;
    let (name, lang) = CONNECTED
        .iter()
        .find_map(|(suffix, lang)| speak.strip_suffix(suffix).map(|n| (n, *lang)))
        .filter(|(n, _)| !n.is_empty())?;
    let mut found = None;
    find_battery(&root, lang, &mut found).ok()?;
    let (percent, charging) = found?;
    Some(Reading {
        name: name.to_string(),
        percent,
        charging,
    })
}

fn card_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(
        r"Packages\Microsoft.YourPhone_8wekyb3d8bbwe\LocalState\StartMenu\StartMenuCompanion.json",
    ))
}

pub fn poll_phone_link(ctx: &mut Ctx) {
    let Some(path) = card_path() else { return };
    let Ok(meta) = std::fs::metadata(&path) else {
        return;
    };
    if meta.len() == 0 || meta.len() > 4 * 1024 * 1024 {
        return;
    }
    let Ok(json) = std::fs::read_to_string(&path) else {
        return;
    };
    let Some(r) = parse(json.trim_start_matches('\u{FEFF}')) else {
        return;
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .unwrap_or_default();
    let b = Battery {
        key: ctx.stable_key("microsoft-phonelink"),
        name: r.name,
        via: "Phone Link".into(),
        kind: Kind::Phone,
        percent: Some(r.percent),
        charging: r.charging,
        asleep: age > STALE,
        ..Default::default()
    };
    ctx.out.push(b);
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = r#"{
        "type": "AdaptiveCard",
        "speak": "Pixel 8 - Connected to your PC",
        "body": [{ "type": "ColumnSet", "columns": [{ "type": "Column", "items": [
            { "type": "Image", "tooltip": "Battery 57% remaining, charging" },
            { "type": "TextBlock", "text": "57%", "tooltip": "Battery 57% remaining, charging" }
        ]}]}],
        "actions": [{ "type": "Action.OpenUrl", "tooltip": "Battery 10% remaining" }]
    }"#;

    #[test]
    fn english_card() {
        assert_eq!(
            parse(CARD),
            Some(Reading {
                name: "Pixel 8".into(),
                percent: 57,
                charging: true
            })
        );
    }

    #[test]
    fn japanese_card_and_conflicts() {
        let ja = r#"{"speak":"Galaxy - PC に接続しました","body":[{"type":"TextBlock","tooltip":"バッテリー 残量 80%"}]}"#;
        assert_eq!(
            parse(ja),
            Some(Reading {
                name: "Galaxy".into(),
                percent: 80,
                charging: false
            })
        );
        let clash = CARD.replacen(
            "Battery 57% remaining, charging",
            "Battery 30% remaining",
            1,
        );
        assert_eq!(parse(&clash), None);
        assert_eq!(
            parse(&CARD.replace("Connected to your PC", "Disconnected")),
            None
        );
        assert_eq!(tooltip_level("Battery 101% remaining", Lang::En), None);
    }
}
