use crate::tools::FsTools;
use anyhow::{Result, anyhow};
use clap::ArgAction;
use globset::{GlobBuilder, GlobMatcher};
use ignore::{Walk, WalkBuilder};
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use size::Size;
use std::path::Path;

#[cfg(test)]
mod test;

/// List directory contents — recursive listing, glob patterns, optional metadata.
///
/// Gitignore-aware: skips paths matched by `.gitignore` and hidden entries like
/// `.git/`, so the output is the project's tracked tree rather than the whole
/// filesystem (e.g. `target/`, `node_modules/` are omitted). Prefer this over
/// shelling out to `ls`/`find` for orienting in a repository.
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

impl ToolMeta for List {
    fn title() -> Option<&'static str> {
        Some("List directory")
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(true),
            open_world_hint: Some(false),
            ..Default::default()
        })
    }

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
    type Output = String;

    fn execute(self, state: &mut FsTools, _context: &RequestContext) -> Result<String> {
        // Parse path to separate directory from glob pattern
        let (base_path, pattern) = self.parse_path_and_pattern()?;

        let base_path = state.resolve_path(base_path, None)?;

        if !base_path.is_dir() {
            return Err(anyhow!("Path is not a directory: {}", base_path.display()));
        }

        let glob_pattern = pattern.map(compile_glob).transpose()?;

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
        if path_str.contains(is_glob_char) {
            // Extract the directory part (everything before the first glob character)
            let mut split_pos = None;
            for (i, char) in path_str.char_indices() {
                if is_glob_char(char) {
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

    /// The walk itself is unfiltered when a pattern is present: a directory does
    /// not match a pattern that names files (`src/foo` against `**/*.rs`), so
    /// pruning on the pattern cut off every nested match — `src/**/*.rs` returned
    /// only the files directly in `src`, silently, as a listing that looked
    /// complete. The pattern is applied per entry in `build_entries` instead;
    /// gitignore and hidden-file pruning still keep `target/` out of the walk.
    fn build_walk(&self, base_path: &Path, glob_pattern: Option<&GlobMatcher>) -> Walk {
        let mut walker = WalkBuilder::new(base_path);
        if glob_pattern.is_none() && !self.recursive() {
            walker.max_depth(Some(1));
        }

        walker.standard_filters(true);
        walker.build()
    }

    fn include_metadata(&self) -> bool {
        self.include_metadata.unwrap_or_default()
    }

    fn build_entries(
        &self,
        base_path: &Path,
        glob_pattern: Option<GlobMatcher>,
    ) -> Result<Vec<String>> {
        let walker = self.build_walk(base_path, glob_pattern.as_ref());
        let mut entries = Vec::new();
        let formatter = timeago::Formatter::new();
        for entry in walker.flatten() {
            if entry.path() == base_path {
                continue;
            }
            let mut file_name =
                pathdiff::diff_paths(entry.path(), base_path).unwrap_or(entry.path().to_owned());

            // Matched against the path *relative to the base*, which is what the
            // pattern was written against: the caller wrote `**/*.rs` beside a
            // base of `src`, not against an absolute path they never typed.
            if let Some(pattern) = &glob_pattern
                && !pattern.is_match(&file_name)
            {
                continue;
            }

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

/// The characters that make a path a pattern rather than a directory name.
/// `{` is here because brace alternation is a common first instinct —
/// `{assets,docs}/**/*.{png,jpg}` — and without it the whole group was read as a
/// literal directory and answered "Path is not a directory".
fn is_glob_char(c: char) -> bool {
    matches!(c, '*' | '?' | '[' | '{')
}

/// `globset` rather than `glob`: the latter has no brace alternation at all, so
/// it compiled `{a,b}` without complaint and then matched nothing.
/// `literal_separator` keeps `*` from crossing a `/`, so `*.rs` means the files
/// here and `**/*.rs` means the ones below too.
fn compile_glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| anyhow!("invalid glob pattern {pattern:?}: {e}"))?
        .compile_matcher())
}
