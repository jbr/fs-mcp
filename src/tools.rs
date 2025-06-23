use crate::{session::SessionStore, traits::AsToolSchema, types::ToolSchema};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

mod delete;
mod list;
mod r#move;
mod read;
mod set_context;
mod write;

pub use delete::Delete;
pub use list::List;
pub use r#move::Move;
pub use read::Read;
pub use set_context::SetContext;
pub use write::Write;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "name")]
pub enum Tools {
    #[serde(rename = "list")]
    List { arguments: List },
    #[serde(rename = "set_context")]
    SetContext { arguments: SetContext },
    #[serde(rename = "write")]
    Write { arguments: Write },
    #[serde(rename = "delete")]
    Delete { arguments: Delete },
    #[serde(rename = "move")]
    Move { arguments: Move },
    #[serde(rename = "read")]
    Read { arguments: Read },
}
impl Tools {
    pub fn execute(self, state: FsTools) -> Result<String> {
        match self {
            Tools::List { arguments } => arguments.execute(state),
            Tools::SetContext { arguments } => arguments.execute(state),
            Tools::Write { arguments } => arguments.execute(state),
            Tools::Delete { arguments } => arguments.execute(state),
            Tools::Move { arguments } => arguments.execute(state),
            Tools::Read { arguments } => arguments.execute(state),
        }
    }

    pub fn schema() -> Vec<ToolSchema> {
        vec![
            SetContext::as_tool_schema(),
            List::as_tool_schema(),
            Write::as_tool_schema(),
            Delete::as_tool_schema(),
            Move::as_tool_schema(),
            Read::as_tool_schema(),
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
        }.with_context(|| format!("Attempting to resolve {}", path.display()))
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
