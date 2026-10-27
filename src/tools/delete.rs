use crate::tools::FsTools;
use anyhow::Result;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use serde::{Deserialize, Serialize};

/// Remove a file from disk
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "delete")]
pub struct Delete {
    /// Path to delete
    /// Can be absolute, or relative to the server's root directory.
    /// Be absolutely certain of the root directory when using a relative path.
    pub path: String,
}

impl ToolMeta for Delete {
    fn title() -> Option<&'static str> {
        Some("Delete file")
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(false),
            destructive_hint: Some(true),
            // A second delete of the same path fails: the file is already gone.
            idempotent_hint: Some(false),
            open_world_hint: Some(false),
            ..Default::default()
        })
    }

    fn examples() -> Vec<Example<Self>> {
        vec![Example {
            description: "Deleting a file relative to the root directory",
            item: Self {
                path: "src/mod/file.rs".into(),
            },
        }]
    }
}

impl Tool<FsTools> for Delete {
    type Output = String;

    async fn execute(self, state: FsTools, _context: RequestContext) -> Result<String> {
        state.unblock(move |state| self.run(state)).await
    }
}

impl Delete {
    pub(crate) fn run(self, state: &FsTools) -> Result<String> {
        let path = state.resolve_path(&self.path);
        std::fs::remove_file(&path)?;
        Ok(format!("Successfully deleted {}", path.display()))
    }
}
