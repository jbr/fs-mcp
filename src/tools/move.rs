use crate::tools::FsTools;
use anyhow::{Result, anyhow};
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};

/// Move a file from one location to another
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename = "move")]
pub struct Move {
    /// Path to move from
    /// Can be absolute, or relative to session context path.
    pub source: String,
    /// Path to move to
    /// Can be absolute, or relative to session context path.
    pub destination: String,

    /// Optional session identifier for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Overwrite destination file if it exists
    /// Only use if you have recently read the file and intend to replace it.
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,

    /// Create any directories leading up to the destination path if they don't already exist
    /// Default: true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_directories: Option<bool>,
}

impl WithExamples for Move {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Rename a new file relative to a session, creating a directory if needed",
                item: Self {
                    source: "src/tool.rs".into(),
                    destination: "src/tool/mod.rs".into(),
                    session_id: Some("some_rust_session_unique_id".into()),
                    overwrite: None,
                    create_directories: Some(true),
                },
            },
            Example {
                description: "Move a file, intentionally replacing the existing file",
                item: Self {
                    source: "/some/absolute/path/src/main.rs".into(),
                    destination: "/some/absolute/path/src/lib.rs".into(),
                    session_id: None,
                    overwrite: Some(true),
                    create_directories: None,
                },
            },
        ]
    }
}

impl Move {
    fn overwrite(&self) -> bool {
        self.overwrite.unwrap_or_default()
    }

    fn create_directories(&self) -> bool {
        self.create_directories.unwrap_or(true)
    }
}

impl Tool<FsTools> for Move {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let source = state.resolve_path(&self.source, self.session_id.as_deref())?;
        let destination = state.resolve_path(&self.destination, self.session_id.as_deref())?;

        if destination.exists() && !self.overwrite() {
            return Err(anyhow!(
                "{} already exists, use `overwrite` to intentionally replace it",
                destination.display()
            ));
        }

        if !source.exists() {
            return Err(anyhow!("{} not found", source.display()));
        }

        if self.create_directories() {
            if let Some(parent_dir) = destination.parent() {
                std::fs::create_dir_all(parent_dir)?;
            }
        }

        std::fs::rename(&source, &destination)?;

        Ok(format!(
            "Successfully moved {} to {}",
            source.display(),
            destination.display()
        ))
    }
}
