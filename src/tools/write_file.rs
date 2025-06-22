use crate::{tools::FsTools, traits::WithExamples, types::Example};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::Write};

/// Write contents to a file, optionally creating any directories needed
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename = "write_file")]
pub struct WriteFile {
    /// Path to write to
    /// Can be absolute, or relative to session context path.
    pub path: String,

    /// Optional session identifier for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// The full contents of the file, expressed as a utf8 string
    pub contents: String,

    /// Overwrite file if it exists
    /// Only use if you have recently read the file and intend to replace it.
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,

    /// Create any directories leading up to this file if they don't already exist
    /// Default: true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_directories: Option<bool>,
}

impl WithExamples for WriteFile {
    fn examples() -> Option<Vec<Example<Self>>> {
        Some(vec![
            Example {
                description: "Creating a new file relative to a session",
                item: Self {
                    path: "src/main.rs".into(),
                    session_id: Some("some_rust_session_unique_id".into()),
                    contents: "#[main]\nfn main() {\n  todo!()\n }".into(),
                    overwrite: None,
                    create_directories: None,
                },
            },
            Example {
                description: "Intentionally overwriting a file",
                item: Self {
                    path: "/some/absolute/path/src/main.rs".into(),
                    session_id: None,
                    contents: "#[main]\nfn main() {\n  todo!()\n }".into(),
                    overwrite: Some(true),
                    create_directories: Some(false),
                },
            },
        ])
    }
}

impl WriteFile {
    fn overwrite(&self) -> bool {
        self.overwrite.unwrap_or_default()
    }

    fn create_directories(&self) -> bool {
        self.create_directories.unwrap_or(true)
    }

    pub fn execute(self, state: FsTools) -> Result<String> {
        let path = state.resolve_path(&self.path, self.session_id.as_deref())?;
        if self.create_directories() {
            if let Some(parent_dir) = path.parent() {
                std::fs::create_dir_all(parent_dir)?;
            }
        }
        let mut open_options = OpenOptions::new();
        if self.overwrite() {
            open_options.write(true).truncate(true).create(true)
        } else {
            open_options.write(true).create_new(true)
        };

        {
            let mut file = open_options.open(&path)?;
            file.write_all(self.contents.as_bytes())?;
        }

        Ok(format!(
            "Successfully wrote {} bytes to {}",
            self.contents.len(),
            path.display()
        ))
    }
}
