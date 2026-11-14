//! Sends each line of stdin to the service and prints the reply.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};

fn main() -> std::io::Result<()> {
    let pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(r"\\.\pipe\foxrgb-service")?;
    let mut reader = BufReader::new(&pipe);
    let mut writer = &pipe;
    for request in std::io::stdin().lines() {
        // PowerShell puts a byte order mark in front of the first piped line.
        let request = request?;
        let request = request.trim_start_matches('\u{feff}');
        writer.write_all(format!("{request}\n").as_bytes())?;
        let mut reply = String::new();
        reader.read_line(&mut reply)?;
        print!("{reply}");
    }
    Ok(())
}
