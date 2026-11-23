//! The changelog shipped with the app, and the slice of it between two versions.

use serde_json::Value;

const BUILT_IN: &str = include_str!("../../changelog.json");

pub const SECTIONS: [(&str, &str); 5] = [
    ("added", "New"),
    ("devices", "Device support"),
    ("changed", "Changes"),
    ("fixed", "Fixes"),
    ("removed", "Removed"),
];

pub fn built_in() -> Value {
    serde_json::from_str(BUILT_IN).unwrap_or(Value::Array(Vec::new()))
}

/// Entries newer than `from` up to and including `to`, newest first.
pub fn between(all: &Value, from: &str, to: &str) -> Vec<Value> {
    let mut out: Vec<Value> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            let v = e["version"].as_str().unwrap_or_default();
            crate::updater::newer(v, from) && !crate::updater::newer(v, to)
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        let (a, b) = (a["version"].as_str().unwrap_or_default(), b["version"].as_str().unwrap_or_default());
        if crate::updater::newer(a, b) {
            std::cmp::Ordering::Less
        } else if crate::updater::newer(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_changelog_reads_and_covers_this_version() {
        let all = built_in();
        let list = all.as_array().unwrap();
        assert!(list.iter().any(|e| e["version"] == env!("CARGO_PKG_VERSION")));
        for e in list {
            assert!(e["date"].is_string());
            assert!(e.as_object().unwrap().keys().all(|k| k == "version" || k == "date" || SECTIONS.iter().any(|s| s.0 == k)));
        }
    }

    #[test]
    fn between_takes_the_versions_after_the_installed_one() {
        let all = built_in();
        let got: Vec<_> = between(&all, "0.15.0", "0.16.0").iter().map(|e| e["version"].as_str().unwrap().to_string()).collect();
        assert_eq!(got, ["0.16.0", "0.15.4", "0.15.3", "0.15.2", "0.15.1"]);
    }
}
