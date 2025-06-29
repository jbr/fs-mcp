use crate::tools::FsTools;
use anyhow::{Context, Result};
use grep::matcher::Matcher;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::{SearcherBuilder, sinks::UTF8};
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Search for text patterns in files using ripgrep-like functionality
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename = "search")]
pub struct Search {
    /// Pattern to search for (supports regex)
    pub pattern: String,

    /// Path to search in (can be file or directory)
    /// Can be absolute, or relative to session context path.
    /// Defaults to current session context if not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Optional session identifier for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Case sensitive search
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<bool>,

    /// File extensions to include (e.g., ["rs", "js", "py"])
    /// If not specified, searches all text files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_extensions: Option<Vec<String>>,

    /// Maximum number of results to return
    /// Default: 50
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,

    /// Highlight style for matches in output
    /// Options: "none", "box", "emphasis", "ansi", "markdown"
    /// Default: "box"
    #[serde(default)]
    pub highlight_style: HighlightStyle,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema, Clone, Copy)]
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

impl WithExamples for Search {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Search for function definitions in Rust files",
                item: Self {
                    pattern: "fn main".to_string(),
                    path: Some("src/".to_string()),
                    session_id: Some("rust_project".to_string()),
                    case_sensitive: Some(false),
                    include_extensions: Some(vec!["rs".to_string()]),
                    max_results: Some(10),
                    highlight_style: HighlightStyle::Box,
                },
            },
            Example {
                description: "Search for TODO comments with emphasis highlighting",
                item: Self {
                    pattern: "TODO|FIXME".to_string(),
                    path: None,
                    session_id: Some("project".to_string()),
                    case_sensitive: Some(false),
                    include_extensions: None,
                    max_results: Some(20),
                    highlight_style: HighlightStyle::Emphasis,
                },
            },
            Example {
                description: "Search with ANSI color highlighting",
                item: Self {
                    pattern: "error".to_string(),
                    path: Some("src/".to_string()),
                    session_id: None,
                    case_sensitive: Some(false),
                    include_extensions: None,
                    max_results: Some(15),
                    highlight_style: HighlightStyle::Ansi,
                },
            },
        ]
    }
}

impl Tool<FsTools> for Search {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let search_path = state.resolve_path(
            self.path.as_deref().unwrap_or("."),
            self.session_id.as_deref(),
        )?;

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

                output.push_str(&format!(
                    "{}:{}: {}\n",
                    result.file_path,
                    result.line_number,
                    highlighted_content.trim()
                ));
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

        if path.is_file() {
            if self.should_search_file(path) {
                self.search_file(path, matcher, results, total_matches, max_results)?;
            }
        } else if path.is_dir() {
            let entries = std::fs::read_dir(path)
                .with_context(|| format!("Failed to read directory: {}", path.display()))?;

            for entry in entries {
                let entry = entry?;
                let entry_path = entry.path();

                if self.should_exclude_path(&entry_path) {
                    continue;
                }

                self.search_path(&entry_path, matcher, results, total_matches, max_results)?;

                if *total_matches >= max_results {
                    break;
                }
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
        let mut searcher = SearcherBuilder::new().build();

        // Use a simpler approach - collect matches directly
        searcher.search_path(
            matcher,
            file_path,
            UTF8(|line_num, line_content| {
                if *total_matches >= max_results {
                    return Ok(false); // Stop searching
                }

                results.push(SearchResult {
                    file_path: file_path.display().to_string(),
                    line_number: line_num,
                    line_content: line_content.to_string(),
                });

                *total_matches += 1;
                Ok(true)
            }),
        )?;

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
}
