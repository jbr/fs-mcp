use crate::tools::FsTools;
use anyhow::{Result, anyhow};
use clap::ArgAction;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use serde::{Deserialize, Serialize};

/// Move a file from one location to another
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "move")]
pub struct Move {
    /// Path to move from
    /// Can be absolute, or relative to the server's root directory.
    pub source: String,
    /// Path to move to
    /// Can be absolute, or relative to the server's root directory.
    pub destination: String,

    /// Overwrite destination file if it exists
    /// Only use if you have recently read the file and intend to replace it.
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub overwrite: Option<bool>,

    /// Create any directories leading up to the destination path if they don't already exist
    /// Default: true
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub create_directories: Option<bool>,
}

impl ToolMeta for Move {
    fn title() -> Option<&'static str> {
        Some("Move file")
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(false),
            // Removes the source, and with `overwrite` replaces the destination.
            destructive_hint: Some(true),
            // A second move of the same source fails: it is no longer there.
            idempotent_hint: Some(false),
            open_world_hint: Some(false),
            ..Default::default()
        })
    }

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Rename a new file relative to the root directory, creating a directory if needed",
                item: Self {
                    source: "src/tool.rs".into(),
                    destination: "src/tool/mod.rs".into(),
                    overwrite: None,
                    create_directories: Some(true),
                },
            },
            Example {
                description: "Move a file, intentionally replacing the existing file",
                item: Self {
                    source: "/some/absolute/path/src/main.rs".into(),
                    destination: "/some/absolute/path/src/lib.rs".into(),
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
    type Output = String;

    async fn execute(self, state: FsTools, _context: RequestContext) -> Result<String> {
        state.unblock(move |state| self.run(state)).await
    }
}

impl Move {
    pub(crate) fn run(self, state: &FsTools) -> Result<String> {
        let source = state.resolve_path(&self.source);
        let destination = state.resolve_path(&self.destination);

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
