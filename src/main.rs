#![allow(clippy::collapsible_if)]

mod session;
mod state;
mod tools;
mod traits;
mod types;

#[cfg(test)]
mod tests;

use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};

use anyhow::Result;
use env_logger::{Builder, Target};
use state::FsTools;
pub use types::{
    ContentResponse, InitializeResponse, McpMessage, McpResponse, RequestType, ToolsListResponse,
};

const INSTRUCTIONS: &str = "Filesystem operations with session support. Use session_id for persistent context between operations.";

fn main() -> Result<()> {
    let mut state = FsTools::new()?;

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    if let Ok(log_location) = std::env::var("LOG_LOCATION") {
        let path = PathBuf::from(&*shellexpand::tilde(&log_location));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        Builder::from_default_env()
            .target(Target::Pipe(Box::new(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap(),
            )))
            .init();
    }

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                log::trace!("<- {line}");
                if let Ok(McpMessage::Request(request)) = serde_json::from_str(&line) {
                    let response = request
                        .call
                        .execute(request.id, &mut state, Some(INSTRUCTIONS));
                    let response_str = serde_json::to_string(&response)?;
                    log::trace!("-> {response_str}");
                    stdout.write_all(response_str.as_bytes())?;
                    stdout.write_all(b"\n")?;
                    stdout.flush()?;
                }
            }
            Err(e) => {
                log::error!("Error reading line: {e}");
                break;
            }
        }
    }

    Ok(())
}
