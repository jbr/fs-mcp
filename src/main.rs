#![allow(clippy::collapsible_if)]

mod state;
mod tools;

#[cfg(test)]
mod tests;

use anyhow::Result;
use mcplease::server_info;
use state::FsTools;

const INSTRUCTIONS: &str = "Filesystem operations with session support. Use session_id for persistent context between operations.";

fn main() -> Result<()> {
    let mut state = FsTools::new()?;

    mcplease::run::<tools::Tools, _>(&mut state, server_info!(), Some(INSTRUCTIONS))
}
