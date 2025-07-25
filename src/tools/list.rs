use crate::tools::FsTools;
use anyhow::{Result, anyhow};
use clap::ArgAction;
use glob::Pattern;
use ignore::{Walk, WalkBuilder};
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use size::Size;
use std::path::Path;

/// List file system contents with session context support and globbing
#[derive(Debug, Serialize, Deserialize, JsonSchema, clap::Args)]
#[serde(rename = "list")]
pub struct List {
    /// Directory path or glob pattern.
    /// Can be absolute, or relative to session context path. Can include wildcards like 'src/**/*'.
    /// Defaults to the current session context if not provided
    pub path: Option<String>,

    /// Recurse into directories (only relevant if path does not contain a glob pattern)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub recursive: Option<bool>,

    /// Include metadata like file size and last modified
    #[arg(long, action = ArgAction::SetTrue)]
    pub include_metadata: Option<bool>,
}

impl WithExamples for List {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Finding all rust files within a project, having already set context. Include metadata",
                item: Self {
                    path: Some("src/**/*.rs".into()),
                    recursive: None,
                    include_metadata: Some(true),
                },
            },
            Example {
                description: "recursively showing all files by absolute path",
                item: Self {
                    path: Some("/some/absolute/path".into()),
                    recursive: Some(true),
                    include_metadata: None,
                },
            },
        ]
    }
}

impl Tool<FsTools> for List {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        // Parse path to separate directory from glob pattern
        let (base_path, pattern) = self.parse_path_and_pattern()?;

        let base_path = state.resolve_path(base_path, None)?;

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
}

impl List {
    /// Parse a path string that might contain glob patterns
    /// Returns (base_directory_path, optional_pattern)
    fn parse_path_and_pattern(&self) -> Result<(&str, Option<&str>)> {
        let path_str = self.path.as_deref().unwrap_or(".");

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
        let mut walker = WalkBuilder::new(base_path);
        if glob_pattern.is_none() && !self.recursive() {
            walker.max_depth(Some(1));
        }

        walker.standard_filters(true);

        // Add glob pattern filtering if provided
        if let Some(pattern) = glob_pattern.cloned() {
            let base_path = base_path.to_owned();
            walker.filter_entry(move |entry| {
                entry.path() != base_path && pattern.matches_path(entry.path())
            });
        }

        walker.build()
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
