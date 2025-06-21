use crate::session::SessionStore;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

mod list_directory;
mod set_context;

pub use list_directory::ListDirectory;
pub use set_context::SetContext;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "name")]
pub enum Tool {
    #[serde(rename = "list_directory")]
    ListDirectory { arguments: ListDirectory },
    #[serde(rename = "set_context")]
    SetContext { arguments: SetContext },
}
impl Tool {
    pub fn execute(self, state: Arc<FsTools>) -> std::result::Result<String, anyhow::Error> {
        match self {
            Tool::ListDirectory { arguments } => arguments.execute(state),
            Tool::SetContext { arguments } => arguments.execute(state),
        }
    }
}

/// Session data specific to filesystem operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FsSessionData {
    /// Current working context path
    pub context_path: Option<PathBuf>,
}

/// Filesystem tools with session support
pub struct FsTools {
    session_store: SessionStore<FsSessionData>,
}

impl FsTools {
    /// Create a new FsTools instance
    pub fn new() -> Result<Self> {
        // Use ~/.ai-tools/sessions/fs/ for session storage
        let mut storage_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        storage_dir.push(".ai-tools");
        storage_dir.push("sessions");
        storage_dir.push("fs.json");

        let session_store = SessionStore::new(storage_dir)?;

        Ok(Self { session_store })
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
