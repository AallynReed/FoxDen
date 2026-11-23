//! MCP server over stdio that hands each tool call to the running FoxDen app.

mod pipe;
mod protocol;
mod tools;

use std::io::{BufRead, Write};

fn main() {
    let mut server = protocol::Server::new(pipe::AppPipe::new());
    let stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("foxden-mcp: cannot read stdin: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let mut out = |msg: serde_json::Value| writeln!(stdout, "{msg}").and_then(|_| stdout.flush()).is_ok();
        let reply = server.handle(&line, &mut |msg| {
            out(msg);
        });
        if let Some(reply) = reply
            && !out(reply)
        {
            break;
        }
    }
}
