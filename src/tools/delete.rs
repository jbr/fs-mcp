use crate::tools::FsTools;
use anyhow::Result;
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};

/// Remove a file from disk
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename = "delete")]
pub struct Delete {
    /// Path to delete
    /// Can be absolute, or relative to session context path.
    pub path: String,

    /// Optional session identifier for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl WithExamples for Delete {
    fn examples() -> Vec<Example<Self>> {
        vec![Example {
            description: "Creating a new file relative to a session",
            item: Self {
                path: "src/mod/file.rs".into(),
                session_id: Some("some_rust_session_unique_id".into()),
            },
        }]
    }
}

impl Tool<FsTools> for Delete {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let path = state.resolve_path(&self.path, self.session_id.as_deref())?;
        std::fs::remove_file(&path)?;
        Ok(format!("Successfully deleted {}", path.display()))
    }
}
