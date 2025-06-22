use crate::{session::SessionStore, traits::AsToolSchema, types::ToolSchema};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

mod delete_file;
mod list_directory;
mod move_file;
mod set_context;
mod write_file;

pub use delete_file::DeleteFile;
pub use list_directory::ListDirectory;
pub use move_file::MoveFile;
pub use set_context::SetContext;
pub use write_file::WriteFile;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "name")]
pub enum Tools {
    #[serde(rename = "list_directory")]
    ListDirectory { arguments: ListDirectory },
    #[serde(rename = "set_context")]
    SetContext { arguments: SetContext },
    #[serde(rename = "write_file")]
    WriteFile { arguments: WriteFile },
    #[serde(rename = "delete_file")]
    DeleteFile { arguments: DeleteFile },
    #[serde(rename = "move_file")]
    MoveFile { arguments: MoveFile },
}
impl Tools {
    pub fn execute(self, state: FsTools) -> Result<String> {
        match self {
            Tools::ListDirectory { arguments } => arguments.execute(state),
            Tools::SetContext { arguments } => arguments.execute(state),
            Tools::WriteFile { arguments } => arguments.execute(state),
            Tools::DeleteFile { arguments } => arguments.execute(state),
            Tools::MoveFile { arguments } => arguments.execute(state),
        }
    }

    pub fn schema() -> Vec<ToolSchema> {
        vec![
            SetContext::as_tool_schema(),
            ListDirectory::as_tool_schema(),
            WriteFile::as_tool_schema(),
            DeleteFile::as_tool_schema(),
            MoveFile::as_tool_schema(),
        ]
    }
}

/// Session data specific to filesystem operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FsSessionData {
    /// Current working context path
    pub context_path: Option<PathBuf>,
}

/// Filesystem tools with session support
#[derive(Debug, Clone)]
pub struct FsTools {
    session_store: Arc<SessionStore<FsSessionData>>,
}

impl FsTools {
    /// Create a new FsTools instance
    pub fn new() -> Result<Self> {
        // Use ~/.ai-tools/sessions/fs/ for session storage
        let mut storage_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        storage_dir.push(".ai-tools");
        storage_dir.push("sessions");
        storage_dir.push("fs.json");

        let session_store = Arc::new(SessionStore::new(storage_dir)?);

        Ok(Self { session_store })
    }

    /// Resolve a path relative to session context if needed
    fn resolve_path(&self, path_str: &str, session_id: Option<&str>) -> Result<PathBuf> {
        let path = PathBuf::from(&*shellexpand::tilde(path_str));

        if path.is_absolute() {
            return Ok(path);
        }

        match session_id {
            Some(session_id) => match self.get_context(Some(session_id))? {
                Some(context) => Ok(context.join(path_str)),
                None => Err(anyhow!(
                    "No context found for `{session_id}`. Use set_context first or provide an absolute path.",
                )),
            },

            None => Err(anyhow!(
                "No session found. Provide a session key and use set_context first to use relative paths."
            )),
        }
    }

    /// Get context for a session
    pub fn get_context(&self, session_id: Option<&str>) -> Result<Option<PathBuf>> {
        let Some(session_id) = session_id else {
            return Ok(None);
        };
        let session_data = self.session_store.get_or_create(session_id)?;
        Ok(session_data.context_path)
    }
}
