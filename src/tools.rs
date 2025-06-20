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
    ) -> Result<Vec<String>> {
        let session_id = session_id.unwrap_or("default");

        // Parse path to separate directory from glob pattern
        let (base_path, pattern) = self.parse_path_and_pattern(path_str, session_id)?;

        if !base_path.is_dir() {
            return Err(anyhow::anyhow!(
                "Path is not a directory: {}",
                base_path.display()
            ));
        }

        let mut entries = Vec::new();

        // Compile glob pattern if provided
        let glob_pattern = if let Some(pattern_str) = pattern {
            Some(Pattern::new(&pattern_str)?)
        } else {
            None
        };

        // Use ignore crate's WalkBuilder for proper gitignore support
        // Use ignore crate's WalkBuilder for proper gitignore support
        let mut walker = WalkBuilder::new(&base_path);
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
                    if entry.path() == base_path {
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

    /// Parse a path string that might contain glob patterns
    /// Returns (base_directory_path, optional_pattern)
    fn parse_path_and_pattern(
        &self,
        path_str: &str,
        session_id: &str,
    ) -> Result<(PathBuf, Option<String>)> {
        // Check if path contains glob patterns
        if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
            // Extract the directory part (everything before the first glob character)
            let mut split_pos = None;
            for (i, char) in path_str.char_indices() {
                if char == '*' || char == '?' || char == '[' {
                    // Find the last path separator before this position
                    if let Some(sep_pos) = path_str[..i].rfind('/') {
                        split_pos = Some(sep_pos);
                    } else if let Some(sep_pos) = path_str[..i].rfind('\\') {
                        split_pos = Some(sep_pos);
                    }
                    break;
                }
            }

            let (dir_part, pattern_part) = if let Some(pos) = split_pos {
                (&path_str[..pos], &path_str[pos + 1..])
            } else {
                // Pattern starts from the beginning, use current directory
                (".", path_str)
            };

            let base_path = self.resolve_path(dir_part, session_id)?;
            Ok((base_path, Some(pattern_part.to_string())))
        } else {
            // No glob pattern, just a regular path
            let base_path = self.resolve_path(path_str, session_id)?;
            Ok((base_path, None))
        }
    }

    /// Resolve a path relative to session context if needed
    fn resolve_path(&self, path_str: &str, session_id: &str) -> Result<PathBuf> {
        if path_str.starts_with('/') || path_str.starts_with("C:\\") {
            // Absolute path
            Ok(PathBuf::from(path_str))
        } else {
            // Relative path - use session context
            match self.get_context(Some(session_id))? {
                Some(context) => Ok(context.join(path_str)),
                None => {
                    Err(anyhow::anyhow!(
                        "No context set for session '{}'. Use set_context first or provide an absolute path.",
                        session_id
                    ))
                }
            }
        }
    }
}
