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

    /// Append a one-line summary to each markdown file: its frontmatter
    /// `description`, else its first heading, else its first line of prose.
    /// Makes a directory of documents self-describing, so choosing what to read
    /// does not require reading.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub summary: Option<bool>,
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
                    summary: None,
                },
            },
            Example {
                description: "recursively showing all files by absolute path",
                item: Self {
                    path: Some("/some/absolute/path".into()),
                    recursive: Some(true),
                    include_metadata: None,
                    summary: None,
                },
            },
            Example {
                description: "Summarizing a directory of markdown documents, so choosing what to read doesn't require reading",
                item: Self {
                    path: Some("docs".into()),
                    recursive: None,
                    include_metadata: None,
                    summary: Some(true),
                },
            },
        ]
    }
}

impl Tool<FsTools> for List {
    type Output = String;

    fn execute(self, state: &mut FsTools, _context: &RequestContext) -> Result<String> {
        // Parse path to separate directory from glob pattern
        let (base_dir, pattern) = self.parse_path_and_pattern()?;

        let base_path = state.resolve_path(base_dir, None)?;

        if !base_path.is_dir() {
            return Err(anyhow!("Path is not a directory: {}", base_path.display()));
        }

        let glob_pattern = pattern.map(compile_glob).transpose()?;

        let entries = self.build_entries(&base_path, glob_pattern, &entry_prefix(base_dir))?;

        let content = format!(
            "{} entries under {}. Each path below is written the way this request \
             wrote its own path, so it can be passed straight back to another call:\n\n{}",
            entries.len(),
            base_path.display(),
            entries.join("\n"),
        );

        Ok(content)
    }
}

/// What every entry is prefixed with: the directory part of the request's path
/// **as the caller wrote it**.
///
/// The prior format stated the base once (`All paths relative to /abs/base:`) and
/// then printed bare entries. Observed live 2026-08-26: a model dropped the header's
/// base and joined an entry to the repo root instead — two failed `read_image` calls
/// on `themes/Coldark/assets/coldark-banner.png` from a `list · assets/**/*`, then a
/// bash round to reconcile "the listing and the filesystem disagree". One header line
/// carrying load over hundreds of entries is the failure shape, and a stated-once base
/// is exactly what a later paste-the-path step loses. Prefixing instead makes each
/// entry independently valid: `assets/**/*` answers `assets/themes/…`.
///
/// A request that named no path (or named `.`) keeps bare relative entries — those
/// already resolve against the session's working directory, which is what the next
/// call resolves against too.
fn entry_prefix(base_dir: &str) -> String {
    let trimmed = base_dir.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        String::new()
    } else {
        format!("{trimmed}/")
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

    fn summary(&self) -> bool {
        self.summary.unwrap_or_default()
    }

    fn build_entries(
        &self,
        base_path: &Path,
        glob_pattern: Option<GlobMatcher>,
        prefix: &str,
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

            let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
            if is_dir {
                file_name.push("");
            }

            let summary_string = if self.summary() && !is_dir {
                markdown_summary(entry.path())
                    .map(|summary| format!("  — {summary}"))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let metadata_string = if self.include_metadata() {
                let metadata = entry.metadata()?;
                let len = Size::from_bytes(metadata.len());
                let created = formatter.convert(metadata.created()?.elapsed()?);
                let modified = formatter.convert(metadata.modified()?.elapsed()?);
                format!(" | {len} | created {created} | modified {modified}")
            } else {
                String::new()
            };

            entries.push(format!(
                "{prefix}{}{metadata_string}{summary_string}",
                file_name.display()
            ));
        }
        entries.sort();
        Ok(entries)
    }
}

/// Longest summary line kept; past this the rest is elided with `…`. A directory
/// listing's value is that it fits in one screen, so one file with a long first
/// paragraph must not be able to dominate it.
const MAX_SUMMARY: usize = 160;

/// A one-line description of a markdown file, in preference order: the YAML
/// frontmatter's `description`, then the first ATX heading, then the first line of
/// prose. `None` for a non-markdown file, an unreadable one, or one with nothing to
/// say.
///
/// Only markdown is summarized: for source files the first heading has no analogue,
/// and guessing one (a doc comment? the first item?) is a language question this tool
/// has no parser for.
fn markdown_summary(path: &Path) -> Option<String> {
    if !matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("markdown")
    ) {
        return None;
    }
    // The whole file is not needed and a listing may touch hundreds of them, so read
    // a bounded head — a description that isn't in the first 4 KiB isn't a summary.
    let contents = read_head(path, 4096)?;
    summarize_markdown(&contents)
}

/// Up to `limit` bytes of `path`, truncated at a UTF-8 boundary.
fn read_head(path: &Path, limit: usize) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0u8; limit];
    let mut file = std::fs::File::open(path).ok()?;
    let mut filled = 0;
    while filled < limit {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    buf.truncate(filled);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The summary rule itself, over already-read text (see [`markdown_summary`]).
fn summarize_markdown(contents: &str) -> Option<String> {
    let mut lines = contents.lines().peekable();

    // Frontmatter, when the file opens with one: a `description:` there is the
    // author's own one-line summary and outranks anything derived.
    if lines.peek().is_some_and(|line| line.trim_end() == "---") {
        lines.next();
        for line in lines.by_ref() {
            let trimmed = line.trim_end();
            if trimmed == "---" || trimmed == "..." {
                break;
            }
            if let Some(rest) = trimmed
                .strip_prefix("description:")
                .or_else(|| trimmed.strip_prefix("summary:"))
            {
                let value = rest.trim().trim_matches(['"', '\'']).trim();
                if !value.is_empty() {
                    return Some(clamp(value));
                }
            }
        }
    }

    // Otherwise the first heading, else the first line of prose. Fenced code is
    // skipped so a file opening with a code block doesn't summarize as `” ```rust ”`.
    let mut in_fence = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || trimmed.is_empty() {
            continue;
        }
        let text = trimmed.trim_start_matches('#').trim();
        if !text.is_empty() {
            return Some(clamp(text));
        }
    }
    None
}

/// Truncate to [`MAX_SUMMARY`] bytes at a char boundary, marking the elision.
fn clamp(text: &str) -> String {
    if text.len() <= MAX_SUMMARY {
        return text.to_string();
    }
    let mut cut = MAX_SUMMARY;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", text[..cut].trim_end())
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
