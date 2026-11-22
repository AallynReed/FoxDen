//! Share codes: a prefix and base64url of versioned JSON, read back only as checked data.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use serde::de::DeserializeOwned;

const MAX_NAME: usize = 40;

pub fn encode(prefix: &str, body: &impl Serialize) -> String {
    let json = serde_json::to_vec(body).unwrap_or_default();
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(json))
}

/// Reads a code made by `encode` with field `v` set to 1. `what` names the code in errors.
pub fn decode<T: DeserializeOwned>(
    code: &str,
    prefix: &str,
    max: usize,
    what: &str,
) -> Result<T, String> {
    let code = code.trim();
    if code.len() > max {
        return Err("That share code is too long".into());
    }
    let body = code
        .strip_prefix(prefix)
        .ok_or_else(|| format!("That is not a FoxDen {what} code"))?;
    let damaged = || "The share code is damaged".to_string();
    let json = URL_SAFE_NO_PAD
        .decode(body.trim_end_matches('='))
        .map_err(|_| damaged())?;
    let value: serde_json::Value = serde_json::from_slice(&json).map_err(|_| damaged())?;
    match value.get("v").and_then(|v| v.as_u64()) {
        Some(1) => {}
        Some(_) => return Err("This code comes from a newer FoxDen; update to use it".into()),
        None => return Err(damaged()),
    }
    serde_json::from_value(value).map_err(|_| damaged())
}

/// A name without control characters, trimmed and bounded, or `or` when nothing is left.
pub fn clean_name(name: &str, or: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    match s.trim() {
        "" => or.to_string(),
        s => s.to_string(),
    }
}
