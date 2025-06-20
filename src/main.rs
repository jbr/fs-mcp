#![allow(clippy::collapsible_if)]

mod session;
mod tools;

use crate::tools::FsTools;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpMessage {
    Request(McpRequest),
    Notification(McpNotification),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let fs_tools = Arc::new(FsTools::new()?);
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = AsyncBufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                if let Ok(McpMessage::Request(request)) = serde_json::from_str(&line) {
                    let response = handle_request(request, Arc::clone(&fs_tools)).await?;
                    let response_str = serde_json::to_string(&response)?;
                    stdout.write_all(response_str.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }
            }
            Err(e) => {
                eprintln!("Error reading line: {e}");
                break;
            }
        }
    }

    Ok(())
}

fn set_context(id: &Value, arguments: &Value, fs_tools: &FsTools) -> Result<Value> {
    let path_str = arguments["path"].as_str().unwrap_or("");
    let session_id = arguments["session_id"].as_str();

    match fs_tools.set_context(session_id, std::path::PathBuf::from(path_str)) {
        Ok(message) => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [
                    {
                        "type": "text",
                        "text": message
                    }
                ]
            }
        })),
        Err(e) => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32000,
                "message": format!("Failed to set context: {}", e)
            }
        })),
    }
}

async fn handle_request(request: McpRequest, fs_tools: Arc<FsTools>) -> Result<Value> {
    let id = &request.id;

    match &*request.method {
        "initialize" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "fs-mcp",
                    "version": "0.1.0",
                },
                "instructions": "Filesystem operations with session support. Use --session-id for persistent context between operations."
            }
        })),
        "tools/list" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": serde_json::from_str::<Value>(include_str!("../schema.json")).unwrap()
            }
        })),
        "tools/call" => {
            let params = request.params.ok_or_else(|| anyhow!("missing params"))?;
            let tool_name = params
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or_default();
            let arguments = params
                .get("arguments")
                .ok_or_else(|| anyhow!("param arguments not provided"))?;

            match tool_name {
                "set_context" => set_context(id, arguments, &fs_tools),
                "list_directory" => list_directory(id, arguments, &fs_tools),
                _ => Ok(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": format!("Unknown tool: {}", tool_name)
                    }
                })),
            }
        }
        _ => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        })),
    }
}

fn list_directory(
    id: &Value,
    arguments: &Value,
    fs_tools: &FsTools,
) -> std::result::Result<Value, anyhow::Error> {
    let path_str = arguments["path"].as_str().unwrap_or(".");
    let session_id = arguments["session_id"].as_str();
    let include_gitignore = arguments["include_gitignore"].as_bool().unwrap_or(false);
    match fs_tools.list_directory(path_str, session_id, include_gitignore) {
        Ok(entries) => {
            let session_notice = if session_id.is_none() {
                "\n[Session Notice: This operation used global state. Consider providing --session-id for better isolation and context management. See set_context for details.]"
            } else {
                ""
            };

            let content = format!(
                "Directory listing for {}:\n{}\n{}",
                path_str,
                entries.join("\n"),
                session_notice
            );

            Ok(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": content
                        }
                    ]
                }
            }))
        }
        Err(e) => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32000,
                "message": format!("Failed to list directory: {}", e)
            }
        })),
    }
}
