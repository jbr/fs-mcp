#![allow(clippy::collapsible_if)]

mod session;
mod tools;
mod traits;
mod types;

#[cfg(test)]
mod tests;

use std::fs::OpenOptions;

use crate::tools::Tools;
use anyhow::Result;
use env_logger::{Builder, Target};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tools::FsTools;
pub use types::{
    ContentResponse, InitializeResponse, McpMessage, McpResponse, RequestType, ToolsListResponse,
};

const INSTRUCTIONS: &str = "Filesystem operations with session support. Use session_id for persistent context between operations.";

impl RequestType {
    fn execute(self, id: Value, state: FsTools) -> McpResponse {
        match self {
            RequestType::Initialize(_) => McpResponse::success(
                id,
                InitializeResponse::default().with_instructions(Some(INSTRUCTIONS)),
            ),

            RequestType::ToolsList(_) => McpResponse::success(
                id,
                ToolsListResponse {
                    tools: Tools::schema(),
                },
            ),

            RequestType::ToolsCall(tool) => match tool.execute(state) {
                Ok(string) => McpResponse::success(id, ContentResponse::text(string)),
                Err(e) => McpResponse::error(id, -32601, e.to_string()),
            },
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let fs_tools = FsTools::new()?;

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = AsyncBufReader::new(stdin);
    let mut line = String::new();

    if let Ok(log_location) = std::env::var("LOG_LOCATION") {
        Builder::from_default_env()
            .target(Target::Pipe(Box::new(
                OpenOptions::new().append(true).open(log_location).unwrap(),
            )))
            .init();
    }

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                log::trace!("<- {line}");
                if let Ok(McpMessage::Request(request)) = serde_json::from_str(&line) {
                    let response = request.call.execute(request.id, fs_tools.clone());
                    let response_str = serde_json::to_string(&response)?;
                    log::trace!("-> {response_str}");
                    stdout.write_all(response_str.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
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
