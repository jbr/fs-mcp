use crate::tools::FsTools;
use anyhow::{Context, Result};
use clap::ArgAction;
use grep::matcher::Matcher;
use grep::regex::RegexMatcherBuilder;
use ignore::WalkBuilder;
// Removed unused imports: SearcherBuilder and UTF8 sink
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Bytes of context kept on each side of a match within a matched line, and the max
/// length of a (matchless) context line, before the rest is elided with `…`. A single
/// pathological line — a minified bundle, or a JSONL log entry that embeds a whole
/// prior tool result — must not be able to dominate (or blow up) the output.
const MATCH_CONTEXT: usize = 100;
const MAX_CONTEXT_LINE: usize = 200;

/// Search for text patterns in files using ripgrep-like functionality
#[derive(Debug, Serialize, Deserialize, JsonSchema, clap::Args)]
#[serde(rename = "search")]
pub struct Search {
    /// Pattern to search for (supports regex)
    pub pattern: String,

    /// Path to search in (can be file or directory)
    /// Can be absolute, or relative to session context path.
    /// Defaults to current session context if not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Case sensitive search
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub case_sensitive: Option<bool>,

    /// File extensions to include (e.g., ["rs", "js", "py"])
    /// If not specified, searches all text files
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub include_extensions: Option<Vec<String>>,

    /// Maximum number of results to return
    /// Default: 50
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub max_results: Option<usize>,

    /// Highlight style for matches in output
    /// Options: "none", "box", "emphasis", "ansi", "markdown"
    /// Default: "box"
    #[arg(value_enum)]
    #[serde(default)]
    #[arg(long)]
    pub highlight_style: HighlightStyle,

    /// Number of context lines to show before and after each match
    /// Default: 1
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub context_lines: Option<usize>,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema, Clone, Copy, clap::ValueEnum)]
pub enum HighlightStyle {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "box")]
    #[default]
    Box, // ┌─match─┐
    #[serde(rename = "emphasis")]
    Emphasis, // ⦗match⦘
    #[serde(rename = "ansi")]
    Ansi, // ANSI color codes
    #[serde(rename = "markdown")]
    Markdown, // **match**
}

impl HighlightStyle {
    fn highlight(&self, text: &str, pattern: &str, case_sensitive: bool) -> String {
        match self {
            Self::None => text.to_string(),
            Self::Box => self.replace_matches(text, pattern, case_sensitive, "┌─", "─┐"),
            Self::Emphasis => self.replace_matches(text, pattern, case_sensitive, "⦗", "⦘"),
            Self::Ansi => {
                self.replace_matches(text, pattern, case_sensitive, "\x1b[93m", "\x1b[0m")
            }
            Self::Markdown => self.replace_matches(text, pattern, case_sensitive, "**", "**"),
        }
    }

    fn replace_matches(
        &self,
        text: &str,
        pattern: &str,
        case_sensitive: bool,
        prefix: &str,
        suffix: &str,
    ) -> String {
        // Try to build a regex from the pattern
        let regex_result = if case_sensitive {
            regex::Regex::new(pattern)
        } else {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
        };

        match regex_result {
            Ok(regex) => regex
                .replace_all(text, |caps: &regex::Captures| {
                    format!("{}{}{}", prefix, &caps[0], suffix)
                })
                .to_string(),
            Err(_) => {
                // Fallback to literal string replacement if regex fails
                if case_sensitive {
                    text.replace(pattern, &format!("{prefix}{pattern}{suffix}"))
                } else {
                    // Simple case-insensitive replacement
                    let lower_text = text.to_lowercase();
                    let lower_pattern = pattern.to_lowercase();

                    if let Some(pos) = lower_text.find(&lower_pattern) {
                        let mut result = text.to_string();
                        let actual_match = &text[pos..pos + pattern.len()];
                        result.replace_range(
                            pos..pos + pattern.len(),
                            &format!("{prefix}{actual_match}{suffix}"),
                        );
                        result
                    } else {
                        text.to_string()
                    }
                }
            }
        }
    }
}

impl ToolMeta for Search {
    fn title() -> Option<&'static str> {
        Some("Search file contents")
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
                description: "Search for function definitions in Rust files",
                item: Self {
                    pattern: "fn main".to_string(),
                    path: Some("src/".to_string()),
                    case_sensitive: Some(false),
                    include_extensions: Some(vec!["rs".to_string()]),
                    max_results: Some(10),
                    highlight_style: HighlightStyle::Box,
                    context_lines: None,
                },
            },
            Example {
                description: "Search for TODO comments with emphasis highlighting",
                item: Self {
                    pattern: "TODO|FIXME".to_string(),
                    path: None,
                    case_sensitive: Some(false),
                    include_extensions: None,
                    max_results: Some(20),
                    highlight_style: HighlightStyle::Emphasis,
                    context_lines: None,
                },
            },
            Example {
                description: "Search with ANSI color highlighting and 2 lines of context",
                item: Self {
                    pattern: "error".to_string(),
                    path: Some("src/".to_string()),
                    case_sensitive: Some(false),
                    include_extensions: None,
                    max_results: Some(15),
                    highlight_style: HighlightStyle::Ansi,
                    context_lines: Some(2),
                },
            },
        ]
    }
}

impl Tool<FsTools> for Search {
    type Output = String;

    fn execute(self, state: &mut FsTools, _context: &RequestContext) -> Result<String> {
        let search_path = state.resolve_path(self.path.as_deref().unwrap_or("."), None)?;

        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!self.case_sensitive())
            .build(&self.pattern)
            .context("Invalid regex pattern")?;

        self.search_with_matcher(&search_path, matcher)
    }
}

impl Search {
    fn case_sensitive(&self) -> bool {
        self.case_sensitive.unwrap_or(false)
    }

    fn max_results(&self) -> usize {
        self.max_results.unwrap_or(50)
    }

    fn highlight_style(&self) -> HighlightStyle {
        self.highlight_style
    }

    fn context_lines(&self) -> usize {
        self.context_lines.unwrap_or(1)
    }

    fn search_with_matcher(&self, search_path: &Path, matcher: impl Matcher) -> Result<String> {
        let mut results = Vec::new();
        let mut total_matches = 0;
        let max_results = self.max_results();

        self.search_path(
            search_path,
            &matcher,
            &mut results,
            &mut total_matches,
            max_results,
        )?;

        if results.is_empty() {
            Ok(format!(
                "No matches found for pattern \"{}\" in {}",
                self.pattern,
                search_path.display()
            ))
        } else {
            let mut output = format!(
                "Found {} matches for pattern \"{}\":\n\n",
                results.len(),
                self.pattern
            );

            let highlight_style = self.highlight_style();
            let case_sensitive = self.case_sensitive();

            for result in results {
                let highlighted_content =
                    highlight_style.highlight(&result.line_content, &self.pattern, case_sensitive);

                // Add context before if available
                for (i, context_line) in result.context_before.iter().enumerate() {
                    let context_line_num =
                        result.line_number - (result.context_before.len() - i) as u64;
                    output.push_str(&format!(
                        "{}:{}: {}\n",
                        result.file_path,
                        context_line_num,
                        context_line.trim()
                    ));
                }

                // Add the matched line
                output.push_str(&format!(
                    "{}:{}: {}\n",
                    result.file_path,
                    result.line_number,
                    highlighted_content.trim()
                ));

                // Add context after if available
                for (i, context_line) in result.context_after.iter().enumerate() {
                    let context_line_num = result.line_number + (i + 1) as u64;
                    output.push_str(&format!(
                        "{}:{}: {}\n",
                        result.file_path,
                        context_line_num,
                        context_line.trim()
                    ));
                }

                // Add separator between results if context lines are shown
                if self.context_lines() > 0
                    && (!result.context_before.is_empty() || !result.context_after.is_empty())
                {
                    output.push_str("--\n");
                }
            }

            if total_matches > max_results {
                output.push_str(&format!(
                    "\n... and {} more matches (limit {})",
                    total_matches - max_results,
                    max_results
                ));
            }

            Ok(output)
        }
    }

    /// Walk `path` gitignore-aware (the `ignore` crate's standard filters — the same
    /// `.gitignore` / `.ignore` / hidden-file handling `list` uses), searching each
    /// text file. Previously this hand-rolled a `read_dir` recursion that only knew a
    /// few hardcoded directory names, so it descended into gitignored trees (e.g. a
    /// project's own log/build output) and could return runaway results. The explicitly
    /// named root is always searched even if ignored, so an intentional search of an
    /// ignored file still works.
    fn search_path(
        &self,
        path: &Path,
        matcher: &impl Matcher,
        results: &mut Vec<SearchResult>,
        total_matches: &mut usize,
        max_results: usize,
    ) -> Result<()> {
        if *total_matches >= max_results {
            return Ok(());
        }

        let walker = WalkBuilder::new(path).standard_filters(true).build();
        for entry in walker.flatten() {
            if *total_matches >= max_results {
                break;
            }
            let entry_path = entry.path();
            if !entry_path.is_file() {
                continue;
            }
            // Belt-and-suspenders for repos with no .gitignore: still skip the usual
            // generated/VCS trees the gitignore filters would otherwise have caught.
            if self.should_exclude_path(entry_path) {
                continue;
            }
            if self.should_search_file(entry_path) {
                self.search_file(entry_path, matcher, results, total_matches, max_results)?;
            }
        }

        Ok(())
    }

    fn search_file(
        &self,
        file_path: &Path,
        matcher: &impl Matcher,
        results: &mut Vec<SearchResult>,
        total_matches: &mut usize,
        max_results: usize,
    ) -> Result<()> {
        // Unreadable or non-UTF-8 (binary) files are skipped, not fatal: the walk now
        // reaches more files, and one stray binary must not abort the whole search.
        let Ok(content) = std::fs::read_to_string(file_path) else {
            return Ok(());
        };

        let lines: Vec<&str> = content.lines().collect();
        let context_lines = self.context_lines();

        for (line_idx, line) in lines.iter().enumerate() {
            if *total_matches >= max_results {
                break;
            }

            let line_number = (line_idx + 1) as u64;

            // Check if this line matches
            let m = matcher
                .find(line.as_bytes())
                .map_err(|e| anyhow::anyhow!("Matcher error: {}", e))?;
            if let Some(m) = m {
                // Collect context before
                let context_before = if context_lines > 0 {
                    let start = line_idx.saturating_sub(context_lines);
                    lines[start..line_idx].iter().map(|s| clamp_line(s)).collect()
                } else {
                    Vec::new()
                };

                // Collect context after
                let context_after = if context_lines > 0 {
                    let end = (line_idx + 1 + context_lines).min(lines.len());
                    lines[line_idx + 1..end].iter().map(|s| clamp_line(s)).collect()
                } else {
                    Vec::new()
                };

                results.push(SearchResult {
                    file_path: file_path.display().to_string(),
                    line_number,
                    // Window the matched line around the match so one giant line can't
                    // dominate; the match itself is always kept (highlighting re-finds it).
                    line_content: window_match(line, m.start(), m.end()),
                    context_before,
                    context_after,
                });

                *total_matches += 1;
            }
        }

        Ok(())
    }

    fn should_search_file(&self, path: &Path) -> bool {
        // Check file extension if specified
        if let Some(extensions) = &self.include_extensions {
            if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                return extensions.iter().any(|allowed| allowed == ext);
            }
            return false;
        }

        // Default: search text files (skip common binary extensions)
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            !matches!(
                ext,
                "exe"
                    | "dll"
                    | "so"
                    | "dylib"
                    | "a"
                    | "o"
                    | "obj"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "bmp"
                    | "ico"
                    | "mp3"
                    | "mp4"
                    | "avi"
                    | "mov"
                    | "zip"
                    | "tar"
                    | "gz"
            )
        } else {
            true // Files without extensions are usually text
        }
    }

    fn should_exclude_path(&self, path: &Path) -> bool {
        // Default exclusions for common non-source directories
        let path_str = path.to_string_lossy();
        path_str.contains("/.git/")
            || path_str.contains("/target/")
            || path_str.contains("/node_modules/")
            || path_str.contains("/.svn/")
            || path_str.contains("/.hg/")
    }
}

#[derive(Debug)]
struct SearchResult {
    file_path: String,
    line_number: u64,
    line_content: String,
    context_before: Vec<String>,
    context_after: Vec<String>,
}

/// The largest byte index `<= i` that is a UTF-8 char boundary of `s`.
fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The smallest byte index `>= i` that is a UTF-8 char boundary of `s`.
fn ceil_boundary(s: &str, mut i: usize) -> usize {
    let n = s.len();
    while i < n && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(n)
}

/// Trim a matched line to a `MATCH_CONTEXT`-byte window on each side of the match,
/// marking each elided end with `…`. Offsets are snapped to char boundaries so the
/// slice is always valid UTF-8. Short lines pass through unchanged.
fn window_match(line: &str, match_start: usize, match_end: usize) -> String {
    let lo = floor_boundary(line, match_start.saturating_sub(MATCH_CONTEXT));
    let hi = ceil_boundary(line, match_end.saturating_add(MATCH_CONTEXT).min(line.len()));
    if lo == 0 && hi == line.len() {
        return line.to_string();
    }
    let mut out = String::new();
    if lo > 0 {
        out.push('…');
    }
    out.push_str(&line[lo..hi]);
    if hi < line.len() {
        out.push('…');
    }
    out
}

/// Truncate a (matchless) context line to `MAX_CONTEXT_LINE` bytes, eliding the rest
/// with `…`. Trims leading/trailing whitespace first (the output loop expects trimmed
/// context lines).
fn clamp_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.len() <= MAX_CONTEXT_LINE {
        return trimmed.to_string();
    }
    let hi = floor_boundary(trimmed, MAX_CONTEXT_LINE);
    format!("{}…", &trimmed[..hi])
}

#[cfg(test)]
mod tests {
    use super::*;
    use grep::regex::RegexMatcherBuilder;
    use std::fs;

    fn search_for(pattern: &str) -> Search {
        Search {
            pattern: pattern.to_string(),
            path: None,
            case_sensitive: Some(false),
            include_extensions: None,
            max_results: Some(50),
            highlight_style: HighlightStyle::None,
            context_lines: Some(0),
        }
    }

    fn run(search: &Search, root: &Path) -> String {
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(true)
            .build(&search.pattern)
            .unwrap();
        search.search_with_matcher(root, matcher).unwrap()
    }

    /// The walk is gitignore-aware: a tree matched by `.gitignore` is not searched, even
    /// though it contains the pattern. (This is the bug that let `search` grep a project's
    /// own gitignored log/build output.)
    #[test]
    fn search_skips_gitignored_trees() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // `.gitignore` is only honored inside a git repo (the `ignore` crate's
        // require_git default, matching `list`); a bare `.git` dir marks the root.
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        fs::write(root.join("keep.txt"), "the needle is here\n").unwrap();
        fs::create_dir(root.join("ignored")).unwrap();
        fs::write(root.join("ignored/skip.txt"), "needle in ignored\n").unwrap();

        let out = run(&search_for("needle"), root);
        assert!(out.contains("keep.txt"), "tracked file searched: {out}");
        assert!(!out.contains("skip.txt"), "gitignored tree skipped: {out}");
    }

    /// An explicitly named file is searched even if it would be ignored as a child of a
    /// walk — intentional searches of generated files still work.
    #[test]
    fn explicitly_named_ignored_file_is_still_searched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("app.log"), "needle in a log\n").unwrap();

        let out = run(&search_for("needle"), &root.join("app.log"));
        assert!(out.contains("app.log"), "named ignored file searched: {out}");
    }

    /// A non-UTF-8 / unreadable file doesn't abort the whole search.
    #[test]
    fn binary_files_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("bin.dat"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        fs::write(root.join("keep.txt"), "needle\n").unwrap();
        let out = run(&search_for("needle"), root);
        assert!(out.contains("keep.txt"), "search completed past the binary: {out}");
    }

    #[test]
    fn window_match_trims_around_the_match() {
        let line = format!("{}NEEDLE{}", "a".repeat(500), "b".repeat(500));
        let start = 500;
        let end = 506;
        let windowed = window_match(&line, start, end);
        assert!(windowed.contains("NEEDLE"), "match kept: {windowed}");
        assert!(windowed.starts_with('…') && windowed.ends_with('…'), "both ends elided");
        assert!(windowed.len() < line.len(), "shorter than the raw line");
        // ~MATCH_CONTEXT on each side + the match + two ellipses.
        assert!(windowed.len() <= 6 + 2 * MATCH_CONTEXT + 8, "bounded: {}", windowed.len());
    }

    #[test]
    fn window_match_leaves_short_lines_untouched() {
        assert_eq!(window_match("short needle line", 6, 12), "short needle line");
    }

    #[test]
    fn window_and_clamp_never_split_a_codepoint() {
        // Multibyte chars packed around the boundaries must not panic / corrupt.
        let line = format!("{}needle{}", "é".repeat(400), "ü".repeat(400));
        let m_start = line.find("needle").unwrap();
        let w = window_match(&line, m_start, m_start + 6);
        assert!(w.contains("needle"));
        let long = "𝔘".repeat(300); // 4 bytes each
        let c = clamp_line(&long);
        assert!(c.ends_with('…'));
        assert!(c.chars().filter(|&ch| ch == '𝔘').count() > 0);
    }

    #[test]
    fn clamp_line_bounds_long_context_lines() {
        let long = "z".repeat(1000);
        let c = clamp_line(&long);
        assert!(c.len() <= MAX_CONTEXT_LINE + 4, "bounded: {}", c.len());
        assert!(c.ends_with('…'));
    }
}
