use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use glob::Pattern;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::tools::FsTools;

#[derive(Debug, Serialize, Deserialize)]
pub struct ListDirectory {
    pub path: String,
    pub session_id: Option<String>,
    pub include_gitignore: Option<bool>,
    pub recursive: Option<bool>,
}
impl ListDirectory {
    pub(crate) fn execute(self, state: Arc<FsTools>) -> Result<String> {
        let ListDirectory {
            path,
            session_id,
            include_gitignore,
            recursive,
        } = &self;

        let include_gitignore = include_gitignore.unwrap_or_default();
        let recursive = recursive.unwrap_or_default();

        // Parse path to separate directory from glob pattern
        let (base_path, pattern) =
            self.parse_path_and_pattern(path, session_id.as_deref(), state)?;

        if !base_path.is_dir() {
            return Err(anyhow!("Path is not a directory: {}", base_path.display()));
        }

        let mut entries = Vec::new();

        // Compile glob pattern if provided
        let glob_pattern = if let Some(pattern_str) = pattern {
            Some(Pattern::new(&pattern_str)?)
        } else {
            None
        };

        // Use ignore crate's WalkBuilder for proper gitignore support
        let mut walker = WalkBuilder::new(&base_path);
        if glob_pattern.is_none() && !recursive {
            walker.max_depth(Some(1));
        }
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

                    let file_name = pathdiff::diff_paths(entry.path(), &base_path)
                        .unwrap_or(entry.path().to_owned());

                    let prefix = if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                        "[DIR] "
                    } else {
                        "[FILE] "
                    };

                    let show_entry = glob_pattern
                        .as_ref()
                        .is_none_or(|pattern| pattern.matches_path(entry.path()));

                    if show_entry {
                        entries.push(format!("{prefix}{}", file_name.display()));
                    }
                }
                Err(err) => {
                    // Log the error but continue processing
                    eprintln!("Warning: Error reading entry: {err}");
                }
            }
        }

        entries.sort();

        let session_notice = if session_id.is_none() {
            "\n[Session Notice: This operation used global state. Consider providing --session-id for better isolation and context management. See set_context for details.]"
        } else {
            ""
        };

        let content = format!(
            "Directory listing for {}:\n{}\n{}",
            path,
            entries.join("\n"),
            session_notice
        );

        Ok(content)
    }

    /// Parse a path string that might contain glob patterns
    /// Returns (base_directory_path, optional_pattern)
    fn parse_path_and_pattern(
        &self,
        path_str: &str,
        session_id: Option<&str>,
        state: Arc<FsTools>,
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
                ("", path_str)
            };

            let base_path = self.resolve_path(dir_part, session_id, state)?;
            Ok((base_path, Some(pattern_part.to_string())))
        } else {
            // No glob pattern, just a regular path
            let base_path = self.resolve_path(path_str, session_id, state)?;
            Ok((base_path, None))
        }
    }

    /// Resolve a path relative to session context if needed
    fn resolve_path(
        &self,
        path_str: &str,
        session_id: Option<&str>,
        state: Arc<FsTools>,
    ) -> Result<PathBuf> {
        let path = PathBuf::from(path_str);

        if path.is_absolute() {
            // Absolute path
            return Ok(PathBuf::from(path_str));
        }

        match session_id {
            Some(session_id) => match state.get_context(Some(session_id))? {
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
}
