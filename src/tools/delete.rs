use crate::tools::FsTools;
use anyhow::Result;
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};

/// Remove a file from disk
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "delete")]
pub struct Delete {
    /// Path to delete
    /// Can be absolute, or relative to working directory.
    /// Be absolutely certain of the working directory when using a relative path.
    pub path: String,
}

impl WithExamples for Delete {
    fn examples() -> Vec<Example<Self>> {
        vec![Example {
            description: "Deleting a file relative to a session",
            item: Self {
                path: "src/mod/file.rs".into(),
            },
        }]
    }
}

impl Tool<FsTools> for Delete {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let path = state.resolve_path(&self.path, None)?;
        std::fs::remove_file(&path)?;
        Ok(format!("Successfully deleted {}", path.display()))
    }
}
