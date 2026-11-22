//! Anonymous error reports, only while the user has them switched on.

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};

const URL: &str = "https://reports.foxhearth.com/v1/foxden";
const MOST: usize = 20;
const LONGEST: usize = 2000;
const CRASH: &str = "crash.txt";

static ON: AtomicBool = AtomicBool::new(false);
static HELD_OFF: AtomicBool = AtomicBool::new(false);
static SENT: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static DEVICES: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Hardware ids such as `usb:046d:c355` and what drives them, sent with each report.
pub fn set_devices(devices: impl Iterator<Item = (String, String)>) {
    *guard(&DEVICES) = devices
        .map(|(id, driver)| json!({ "id": id, "driver": driver }))
        .collect();
}

pub fn error(source: &str, text: &str) {
    let failure = matches!(source, "hid" | "openrgb" | "smbus" | "gpu" | "devices");
    let kind = if failure { "device-failure" } else { "error" };
    send(kind, source, text);
}

fn send(kind: &str, source: &str, text: &str) {
    if !ON.load(Ordering::Relaxed) || HELD_OFF.load(Ordering::Relaxed) {
        return;
    }
    let text = scrub(text);
    {
        let mut sent = guard(&SENT);
        let sent = sent.get_or_insert_with(HashSet::new);
        if sent.len() >= MOST || !sent.insert(format!("{kind}|{source}|{text}")) {
            return;
        }
    }
    let body = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "kind": kind,
        "os": "windows",
        "source": source,
        "error": text,
        "devices": guard(&DEVICES).clone(),
    });
    tauri::async_runtime::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        else {
            return;
        };
        if let Ok(reply) = client.post(URL).json(&body).send().await
            && reply.status().as_u16() == 429
        {
            HELD_OFF.store(true, Ordering::Relaxed);
        }
    });
}

/// Sends one report the user asked to send, whether or not error reports are on.
pub async fn post(body: &Value) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let reply = client
        .post(URL)
        .json(body)
        .send()
        .await
        .map_err(|_| "Could not reach the report server.".to_string())?;
    match reply.status().as_u16() {
        200..=299 => Ok(()),
        413 => Err("Too large to send. Copy it into an issue instead.".into()),
        429 => Err("Too many reports just now. Try again in a minute.".into()),
        code => Err(format!("The report server refused it ({code}).")),
    }
}

const FEEDBACK_URL: &str = "https://reports.foxhearth.com/v1/foxden/feedback";
const FEEDBACK: [&str; 4] = ["bug", "idea", "praise", "other"];
const ATTACHMENT_MOST: usize = 16 * 1024 * 1024;
const ATTACHMENTS: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("mov", "video/quicktime"),
];

#[derive(serde::Deserialize)]
struct Feedback {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    contact: String,
    #[serde(default)]
    file: Option<String>,
}

fn feedback_body(kind: &str, message: &str, contact: &str) -> Result<Value, String> {
    let (message, contact) = (message.trim(), contact.trim());
    if !FEEDBACK.contains(&kind) {
        return Err("Pick what kind of feedback this is.".into());
    }
    if message.is_empty() {
        return Err("Write a message first.".into());
    }
    if message.chars().count() > LONGEST || contact.chars().count() > 100 {
        return Err("Too large to send.".into());
    }
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "type": kind,
        "message": message,
        "contact": contact,
    }))
}

/// The attachment's upload name and content type, from its extension. The user's file name is not sent.
fn attachment(name: &str, size: usize) -> Result<(String, &'static str), String> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    let ext = if ext.as_deref() == Some("jpeg") {
        Some("jpg".to_string())
    } else {
        ext
    };
    let Some(&(ext, mime)) = ATTACHMENTS.iter().find(|(e, _)| Some(*e) == ext.as_deref()) else {
        return Err("That file type isn't supported.".into());
    };
    if size > ATTACHMENT_MOST {
        return Err("The file is larger than 16 MB.".into());
    }
    Ok((format!("attachment.{ext}"), mime))
}

/// Feedback the user wrote and chose to send. The body is a 4-byte length, the JSON, then the file.
#[tauri::command]
pub async fn send_feedback(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(body) = request.body() else {
        return Err("The feedback could not be read.".into());
    };
    let bad = || "The feedback could not be read.".to_string();
    let len = u32::from_le_bytes(
        body.get(..4)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ) as usize;
    let meta = body.get(4..4 + len).ok_or_else(bad)?;
    let file = &body[4 + len..];
    let meta: Feedback = serde_json::from_slice(meta).map_err(|_| bad())?;
    let data = feedback_body(&meta.kind, &meta.message, &meta.contact)?;
    let mut form = reqwest::multipart::Form::new().text("data", data.to_string());
    if let Some(name) = meta.file.filter(|_| !file.is_empty()) {
        let (name, mime) = attachment(&name, file.len())?;
        let part = reqwest::multipart::Part::bytes(file.to_vec())
            .file_name(name)
            .mime_str(mime)
            .map_err(|e| e.to_string())?;
        form = form.part("file", part);
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let reply = client
        .post(FEEDBACK_URL)
        .multipart(form)
        .send()
        .await
        .map_err(|_| "Could not reach the feedback server.".to_string())?;
    match reply.status().as_u16() {
        200..=299 => Ok(()),
        413 => Err("Too large to send.".into()),
        415 => Err("That file type isn't supported.".into()),
        429 => Err("Too many sent just now. Try again later.".into()),
        code => Err(format!("The feedback server refused it ({code}).")),
    }
}

/// Writes a crash to disk; it is sent at the next start if reports are on.
pub fn catch_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let _ = std::fs::write(
            crate::config::dir().join(CRASH),
            format!("{message}{place}"),
        );
        default(info);
    }));
}

pub fn send_last_crash() {
    let path = crate::config::dir().join(CRASH);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path);
    crate::log::warn("app", format!("FoxDen crashed last time: {text}"));
    send("crash", "app", &text);
}

/// Takes out user and machine names, paths, addresses and device instance ids.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    for var in ["USERPROFILE", "USERNAME", "COMPUTERNAME", "USERDOMAIN"] {
        if let Ok(value) = std::env::var(var)
            && value.len() > 2
        {
            out = replace_ci(&out, &value, &format!("<{}>", var.to_lowercase()));
        }
    }
    let words: Vec<String> = out
        .split(' ')
        .map(|word| {
            let bare = word.trim_matches(|c: char| "()[]{}<>,;'\"".contains(c));
            let host = bare.rsplit_once(':').map_or(bare, |(h, p)| {
                if p.chars().all(|c| c.is_ascii_digit()) {
                    h
                } else {
                    bare
                }
            });
            if bare.contains('@') {
                "<email>".into()
            } else if host.parse::<IpAddr>().is_ok() || bare.parse::<IpAddr>().is_ok() {
                "<ip>".into()
            } else if bare.contains('#')
                || bare.starts_with("\\\\")
                || (bare.len() > 2 && bare.as_bytes()[1] == b':' && bare.contains('\\'))
            {
                "<path>".into()
            } else {
                word.to_string()
            }
        })
        .collect();
    let mut out = words.join(" ");
    if out.len() > LONGEST {
        let mut end = LONGEST;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

fn replace_ci(text: &str, find: &str, with: &str) -> String {
    let (lower, needle) = (text.to_lowercase(), find.to_lowercase());
    if lower.len() != text.len() {
        return text.replace(find, with);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    for (i, _) in lower.match_indices(&needle) {
        out.push_str(&text[rest..i]);
        out.push_str(with);
        rest = i + needle.len();
    }
    out.push_str(&text[rest..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_takes_out_anything_that_identifies_the_machine() {
        let user = std::env::var("USERNAME").unwrap_or_default();
        let text = format!(
            "open \\\\?\\hid#vid_046d&pid_c355#7&1a2b failed for {user} at C:\\Users\\x\\a.json, 192.168.1.5:6742 me@x.com"
        );
        let out = scrub(&text);
        assert!(!out.contains("vid_046d&pid"), "{out}");
        assert!(!out.contains("192.168"), "{out}");
        assert!(!out.contains("me@x.com"), "{out}");
        assert!(!out.contains("C:\\Users"), "{out}");
        if user.len() > 2 {
            assert!(!out.to_lowercase().contains(&user.to_lowercase()), "{out}");
        }
        assert!(out.starts_with("open <path> failed"), "{out}");
    }

    #[test]
    fn scrub_keeps_ordinary_errors_and_caps_the_length() {
        assert_eq!(
            scrub("device busy (0x08) on retry 3"),
            "device busy (0x08) on retry 3"
        );
        assert!(scrub(&"é".repeat(5000)).len() <= LONGEST);
    }

    #[test]
    fn feedback_is_checked_before_it_is_sent() {
        let body = feedback_body("idea", "  More fans  ", "").unwrap();
        assert_eq!(body["type"], "idea");
        assert_eq!(body["message"], "More fans");
        assert!(feedback_body("rant", "x", "").is_err());
        assert!(feedback_body("bug", "   ", "").is_err());
        assert!(feedback_body("bug", &"a".repeat(2001), "").is_err());
        assert!(feedback_body("bug", "x", &"a".repeat(101)).is_err());
        assert_eq!(
            attachment("Clip.MOV", 10).unwrap(),
            ("attachment.mov".into(), "video/quicktime")
        );
        assert_eq!(attachment("a.jpeg", 10).unwrap().1, "image/jpeg");
        assert!(attachment("a.exe", 10).is_err());
        assert!(attachment("a.png", ATTACHMENT_MOST + 1).is_err());
    }

    #[test]
    fn nothing_is_sent_while_reports_are_off() {
        set_enabled(false);
        send("error", "test", "boom");
        assert!(guard(&SENT).as_ref().is_none_or(|s| s.is_empty()));
    }
}
