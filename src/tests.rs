use crate::{McpMessage, tools::Tools};

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

#[test]
fn schemars_dont_panic() {
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&Tools::schema()).unwrap()
    );
}
