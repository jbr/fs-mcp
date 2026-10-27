#![allow(clippy::collapsible_if)]

mod state;
mod tools;

#[cfg(test)]
mod tests;

use anyhow::Result;
use async_compat::Compat;
use clap::Parser;
use mcplease::{Cli, ServerConfig, server_info, traits::Dispatch, types::RequestContext};
use state::FsTools;
use std::path::PathBuf;
use tools::Tools;

const INSTRUCTIONS: &str = "Filesystem operations. Paths are absolute, or relative to the \
                            directory this server was started in.";

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// The directory relative paths resolve against [default: the current directory]
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Cli<Tools>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // stderr is the one place a stdio server may log.
    env_logger::init();
    let Args { root, command } = Args::parse();
    let root = match root {
        Some(root) => root,
        None => std::env::current_dir()?,
    };
    let state = FsTools::new(root);

    match command {
        Cli::Tool(tool) => {
            let output = tool.call_to_text(state, RequestContext::default()).await?;
            println!("{output}");
        }

        Cli::Serve => {
            let config = ServerConfig::new(server_info!()).with_instructions(INSTRUCTIONS);
            let stdin = Compat::new(tokio::io::stdin());
            let stdout = Compat::new(tokio::io::stdout());
            let mut calls = mcplease::calls::<Tools, _>(stdin, stdout, state, &config);
            while let Some(call) = calls.next().await {
                tokio::spawn(call?);
            }
        }
    }

    Ok(())
}
