use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use mcplease::session::SessionStore;

/// Shared context data that can be used across multiple MCP servers
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SharedContextData {
    /// Current working context path
    pub context_path: Option<PathBuf>,
}

/// Filesystem tools with session support
#[derive(Debug, fieldwork::Fieldwork)]
pub struct FsTools {
    /// Shared context store for cross-server communication
    #[fieldwork(get, get_mut)]
    shared_context_store: SessionStore<SharedContextData>,
}

impl FsTools {
    /// Create a new FsTools instance
    pub fn new() -> Result<Self> {
        // Use shared context store for cross-server communication
        let mut shared_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        shared_path.push(".ai-tools");
        shared_path.push("sessions");
        shared_path.push("shared-context.json");

        let shared_context_store = SessionStore::new(Some(shared_path))?;

        Ok(Self {
            shared_context_store,
        })
    }

    fn default_session_id(&self) -> &'static str {
        "default"
    }

    /// Resolve a path relative to session context if needed
    pub(crate) fn resolve_path(
        &mut self,
        path_str: &str,
        session_id: Option<&str>,
    ) -> Result<PathBuf> {
        let path = PathBuf::from(&*shellexpand::tilde(path_str));

        if path.is_absolute() {
            return Ok(path);
        }

        let session_id = session_id.unwrap_or_else(|| self.default_session_id());
        match self.get_context(Some(session_id))? {
            Some(context) => Ok(context.join(path_str)),
            None => Err(anyhow!(
                "Use set_working_directory first or provide an absolute path.",
            )),
        }
    }

    /// Get context for a session
    pub fn get_context(&mut self, session_id: Option<&str>) -> Result<Option<PathBuf>> {
        let session_data = self
            .shared_context_store
            .get_or_create(session_id.unwrap_or_else(|| self.default_session_id()))?;
        Ok(session_data.context_path.clone())
    }

    pub(crate) fn set_working_directory(
        &mut self,
        path: PathBuf,
        session_id: Option<&str>,
    ) -> Result<()> {
        self.shared_context_store.update(
            session_id.unwrap_or_else(|| self.default_session_id()),
            |shared_data| {
                shared_data.context_path = Some(path);
            },
        )
    }
}
