use crate::session::SessionStore;
use anyhow::Result;
use glob::Pattern;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
        storage_dir.push("fs");

        let session_store = SessionStore::new(storage_dir)?;

        Ok(Self { session_store })
    }

    /// Set context for a session
    pub fn set_context(&self, session_id: Option<&str>, path: PathBuf) -> Result<String> {
        let session_id = session_id.unwrap_or("default");

        let mut session_data = self.session_store.get_or_create(session_id)?;
        session_data.context_path = Some(path.clone());
        self.session_store.set(session_id, session_data)?;

        Ok(format!(
            "Set context to {} for session '{}'",
            path.display(),
            session_id
        ))
    }

    /// Get context for a session
    pub fn get_context(&self, session_id: Option<&str>) -> Result<Option<PathBuf>> {
        let session_id = session_id.unwrap_or("default");
        let session_data = self.session_store.get_or_create(session_id)?;
        Ok(session_data.context_path)
    }

    /// List directory contents with session context support and gitignore awareness
    pub fn list_directory(
        &self,
        path_str: &str,
        session_id: Option<&str>,
        include_gitignore: bool,
        pattern: Option<&str>,
    ) -> Result<Vec<String>> {
        let session_id = session_id.unwrap_or("default");

        // Resolve path relative to session context if needed
        let path = if path_str.starts_with('/') || path_str.starts_with("C:\\") {
            // Absolute path
            PathBuf::from(path_str)
        } else {
            // Relative path - use session context
            match self.get_context(Some(session_id))? {
                Some(context) => context.join(path_str),
                None => {
                    return Err(anyhow::anyhow!(
                        "No context set for session '{}'. Use set_context first or provide an absolute path.",
                        session_id
                    ));
                }
            }
        };

        if !path.is_dir() {
            return Err(anyhow::anyhow!(
                "Path is not a directory: {}",
                path.display()
            ));
        }

        let mut entries = Vec::new();

        // Compile glob pattern if provided
        let glob_pattern = if let Some(pattern_str) = pattern {
            Some(Pattern::new(pattern_str)?)
        } else {
            None
        };

        // Use ignore crate's WalkBuilder for proper gitignore support
        // Use ignore crate's WalkBuilder for proper gitignore support
        let mut walker = WalkBuilder::new(&path);
        walker.max_depth(Some(1)); // Only list direct children, not recursive
        walker.hidden(!include_gitignore); // Respect hidden file settings
        walker.git_ignore(!include_gitignore); // Respect .gitignore unless overridden
        walker.git_global(!include_gitignore); // Respect global gitignore
        walker.git_exclude(!include_gitignore); // Respect .git/info/exclude

        // Add glob pattern filtering if provided
        if let Some(ref pattern) = glob_pattern {
            let pattern_clone = pattern.clone();
            walker.filter_entry(move |entry| {
                // Always allow directories to be traversed for pattern matching
                if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    return true;
                }

                // For files, check if they match the glob pattern
                pattern_clone.matches_path(entry.path())
            });
        }

        let walker = walker.build();

        for result in walker {
            match result {
                Ok(entry) => {
                    // Skip the root directory itself
                    if entry.path() == path {
                        continue;
                    }

                    let file_name = entry.file_name().to_string_lossy().to_string();

                    let prefix = if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                        "[DIR] "
                    } else {
                        "[FILE] "
                    };

                    entries.push(format!("{prefix}{file_name}"));
                }
                Err(err) => {
                    // Log the error but continue processing
                    eprintln!("Warning: Error reading entry: {err}");
                }
            }
        }

        entries.sort();
        Ok(entries)
    }
}
