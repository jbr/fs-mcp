use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::session::SessionStore;

/// Session data specific to filesystem operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FsSessionData {
    /// Current working context path
    pub context_path: Option<PathBuf>,
}

/// Filesystem tools with session support
#[derive(Debug, Clone, fieldwork::Fieldwork)]
pub struct FsTools {
    #[fieldwork(get)]
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

    /// Resolve a path relative to session context if needed
    pub(crate) fn resolve_path(&self, path_str: &str, session_id: Option<&str>) -> Result<PathBuf> {
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
