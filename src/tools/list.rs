use crate::{tools::FsTools, traits::WithExamples, types::Example};
use anyhow::{Result, anyhow};
use glob::Pattern;
use ignore::{Walk, WalkBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use size::Size;
use std::path::Path;

/// List file system contents with session context support, globbing and gitignore
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename = "list")]
pub struct List {
    /// Directory path or glob pattern.
    /// Can be absolute, or relative to session context path. Can include wildcards like 'src/**/*'.
    pub path: String,

    /// Optional session identifier for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Hide gitignored files.
    /// Defaults to true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitignore: Option<bool>,

    /// Recurse into directories (only relevant if path does not contain a glob pattern)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,

    /// Include metadata like file size and last modified
    pub include_metadata: Option<bool>,
}

impl WithExamples for List {
    fn examples() -> Option<Vec<Example<Self>>> {
        Some(vec![
            Example {
                description: "Finding all rust files within a project, having already set context. Include metadata",
                item: Self {
                    path: "src/**/*.rs".into(),
                    session_id: Some("some_rust_session_unique_id".into()),
                    gitignore: None,
                    recursive: None,
                    include_metadata: Some(true),
                },
            },
            Example {
                description: "recursively showing all files by absolute path",
                item: Self {
                    path: "/some/absolute/path".into(),
                    session_id: None,
                    gitignore: None,
                    recursive: Some(true),
                    include_metadata: None,
                },
            },
        ])
    }
}

impl List {
    pub fn execute(self, state: FsTools) -> Result<String> {
        // Parse path to separate directory from glob pattern
        let (base_path, pattern) = self.parse_path_and_pattern()?;

        let base_path = state.resolve_path(base_path, self.session_id.as_deref())?;

        if !base_path.is_dir() {
            return Err(anyhow!("Path is not a directory: {}", base_path.display()));
        }

        // Compile glob pattern if provided
        let glob_pattern = pattern.map(Pattern::new).transpose()?;

        let entries = self.build_entries(&base_path, glob_pattern)?;

        let content = format!(
            "All paths relative to {}:\n\n{}",
            base_path.display(),
            entries.join("\n"),
        );

        Ok(content)
    }

    /// Parse a path string that might contain glob patterns
    /// Returns (base_directory_path, optional_pattern)
    fn parse_path_and_pattern(&self) -> Result<(&str, Option<&str>)> {
        let path_str = &*self.path;

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

            Ok((dir_part, Some(pattern_part)))
        } else {
            // No glob pattern, just a regular path
            Ok((path_str, None))
        }
    }

    fn recursive(&self) -> bool {
        self.recursive.unwrap_or_default()
    }

    fn build_walk(&self, base_path: &Path, glob_pattern: Option<&Pattern>) -> Walk {
        // Use ignore crate's WalkBuilder for proper gitignore support
        let mut walker = WalkBuilder::new(base_path);
        if glob_pattern.is_none() && !self.recursive() {
            walker.max_depth(Some(1));
        }

        walker.hidden(false);

        if !self.gitignore() {
            walker
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false);
        }

        // Add glob pattern filtering if provided
        if let Some(pattern) = glob_pattern.cloned() {
            let base_path = base_path.to_owned();
            walker.filter_entry(move |entry| {
                entry.path() != base_path && pattern.matches_path(entry.path())
            });
        }

        walker.build()
    }

    fn gitignore(&self) -> bool {
        self.gitignore.unwrap_or(true)
    }

    fn include_metadata(&self) -> bool {
        self.include_metadata.unwrap_or_default()
    }

    fn build_entries(
        &self,
        base_path: &Path,
        glob_pattern: Option<Pattern>,
    ) -> Result<Vec<String>> {
        let walker = self.build_walk(base_path, glob_pattern.as_ref());
        let mut entries = Vec::new();
        let formatter = timeago::Formatter::new();
        for entry in walker.flatten() {
            let mut file_name =
                pathdiff::diff_paths(entry.path(), base_path).unwrap_or(entry.path().to_owned());

            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                file_name.push("");
            }

            let metadata_string = if self.include_metadata() {
                let metadata = entry.metadata()?;
                let len = Size::from_bytes(metadata.len());
                let created = formatter.convert(metadata.created()?.elapsed()?);
                let modified = formatter.convert(metadata.modified()?.elapsed()?);
                format!(" | {len} | created {created} | modified {modified}")
            } else {
                String::new()
            };

            entries.push(format!("{}{}", file_name.display(), metadata_string));
        }
        entries.sort();
        Ok(entries)
    }
}
