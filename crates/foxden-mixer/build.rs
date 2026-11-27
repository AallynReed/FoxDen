//! Finds DeepFilterNet's low-latency model in the pinned git checkout, so it can be built in.

use std::path::PathBuf;

const REV: &str = "7b14b89";
const MODEL: &str = "DeepFilterNet3_ll_onnx.tar.gz";

fn main() {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".cargo")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .expect("no cargo home");
    let checkouts = home.join("git").join("checkouts");
    let found = std::fs::read_dir(&checkouts)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("deepfilternet-"))
        .map(|e| e.path().join(REV).join("models").join(MODEL))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{MODEL} not found under {}", checkouts.display()));
    println!("cargo:rerun-if-changed={}", found.display());
    println!("cargo:rustc-env=FOXDEN_DF_LL_MODEL={}", found.display());
}
