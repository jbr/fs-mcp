#![allow(clippy::collapsible_if)]

mod session;
mod tools;

use crate::tools::{FsTools, Tool};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::{borrow::Cow, collections::HashMap, sync::Arc};
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
    #[serde(flatten)]
    pub call: RequestType,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum RequestType {
    #[serde(rename = "initialize")]
    Initialize(InitializeRequest),

    #[serde(rename = "tools/list")]
    ToolsList(Value),

    #[serde(rename = "tools/call")]
    ToolsCall(Tool),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    capabilities: Value,
    client_info: Info,
    protocol_version: String,
}

#[derive(Debug, Serialize, Deserialize, fieldwork::Fieldwork)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    protocol_version: &'static str,
    capabilities: Capabilities,
    server_info: Info,
    #[fieldwork(with)]
    instructions: Option<&'static str>,
}

impl Default for InitializeResponse {
    fn default() -> Self {
        Self {
            protocol_version: "2024-11-05",
            capabilities: Capabilities::default(),
            server_info: Info {
                name: env!("CARGO_PKG_NAME").into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Info {
    name: Cow<'static, str>,
    version: Cow<'static, str>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    tools: HashMap<(), ()>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ToolsListResponse {
    tools: Vec<ToolSchema>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSchema {
    name: String,
    description: String,
    input_schema: InputSchema,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputSchema {
    #[serde(rename = "object")]
    Object {
        properties: HashMap<String, Box<InputSchema>>,
        required: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_properties: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        examples: Option<Vec<Value>>,
    },
    #[serde(rename = "string")]
    String {
        description: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#enum: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        examples: Option<Vec<String>>,
    },

    #[serde(rename = "boolean")]
    Boolean { description: String },
}

const INSTRUCTIONS: &str = "Filesystem operations with session support. Use --session-id for persistent context between operations.";

impl RequestType {
    fn execute(self, id: Value, fs_tools: Arc<FsTools>) -> McpResponse {
        match self {
            RequestType::Initialize(_) => McpResponse::success(
                id,
                InitializeResponse::default().with_instructions(Some(INSTRUCTIONS)),
            ),

            RequestType::ToolsList(_) => McpResponse::success(
                id,
                ToolsListResponse {
                    tools: serde_json::from_str(include_str!("../schema.json")).unwrap(),
                },
            ),

            RequestType::ToolsCall(tool) => match tool.execute(fs_tools) {
                Ok(string) => McpResponse::success(
                    id,
                    &ContentResponse {
                        r#type: "text",
                        text: string,
                    },
                ),
                Err(e) => McpResponse::error(id, -32601, e.to_string()),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContentResponse {
    r#type: &'static str,
    text: String,
}

impl McpResponse {
    pub fn success(id: Value, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(serde_json::to_value(result).unwrap()),
            error: None,
        }
    }

    pub fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(McpError {
                code,
                message,
                data: None,
            }),
        }
    }
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
                    let response = request.call.execute(request.id, Arc::clone(&fs_tools));
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

#[cfg(test)]
mod tests {
    use crate::{McpMessage, McpRequest, RequestType};

    use super::tools::{ListDirectory, Tool};

    #[test]
    fn deserialize_initialize() {
        let initialize = r#"{"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"claude-ai","version":"0.1.0"}},"jsonrpc":"2.0","id":0}"#;
        dbg!(serde_json::from_str::<McpMessage>(initialize).unwrap());
    }

    #[test]
    fn deserialize_initialized() {
        let initialize = r#"{"method":"notifications/initialized","jsonrpc":"2.0"}"#;
        dbg!(serde_json::from_str::<McpMessage>(initialize).unwrap());
    }

    #[test]
    fn deserialize_tool_list() {
        let initialize = r#"{"method":"tools/list","params":{},"jsonrpc":"2.0","id":1}"#;
        dbg!(serde_json::from_str::<McpMessage>(initialize).unwrap());
    }

    #[test]
    fn deserialize_list_directory() {
        let list_directory = r#"{"method":"tools/call","params":{"name":"list_directory","arguments":{"path":"src/languages/*.rs"}},"jsonrpc":"2.0","id":39}"#;
        eprintln!("{list_directory}");
        dbg!(serde_json::from_str::<McpMessage>(list_directory).unwrap());
    }
}
