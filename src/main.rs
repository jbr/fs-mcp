#![allow(clippy::collapsible_if)]

mod state;
mod tools;

#[cfg(test)]
mod tests;

use anyhow::Result;
use mcplease::server_info;
use state::FsTools;

const INSTRUCTIONS: &str = "Filesystem operations. Use set_working_directory to use relative paths";

fn main() -> Result<()> {
    let mut state = FsTools::new()?;

    mcplease::run::<tools::Tools, _>(&mut state, server_info!(), Some(INSTRUCTIONS))
}
