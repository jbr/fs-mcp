use crate::tools::FsTools;
use anyhow::{Context, Result, anyhow};
use globset::{GlobBuilder, GlobMatcher};
use grep::matcher::Matcher;
use grep::regex::RegexMatcherBuilder;
use ignore::WalkBuilder;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Bytes of context kept on each side of a match within a matched line, and the max
/// length of a (matchless) context line, before the rest is elided with `…`. A single
/// pathological line — a minified bundle, or a JSONL log entry that embeds a whole
/// prior tool result — must not be able to dominate (or blow up) the output.
const MATCH_CONTEXT: usize = 100;
const MAX_CONTEXT_LINE: usize = 200;

/// How far past `max_results` the scan keeps counting so the total it reports is a
/// real number rather than "at least the limit". Counting is nearly free once a file
/// has been read, but walking an entire monorepo after the caller already has all the
/// results they asked for is not — past this, the count is reported as `N+`.
const COUNT_CEILING: usize = 1000;

/// Ranges listed per file in `line_numbers_only` mode before the rest are summarized.
/// Capped so the broad scout — "where is `Result` used" across a repo — degrades into
/// a path-and-count list rather than hundreds of numbers.
const MAX_RANGES_PER_FILE: usize = 10;

/// Search for text patterns in files using ripgrep-like functionality
#[derive(Debug, Serialize, Deserialize, JsonSchema, clap::Args)]
#[serde(rename = "search")]
pub struct Search {
    /// Pattern to search for (regex). May also be a list of patterns, which are
    /// OR-ed together — half of real searches are alternations, and a list needs
    /// no `|` escaping and cannot group its alternatives wrong.
    pub pattern: Patterns,

    /// Paths to search. Each may be a file, a directory (searched recursively), or
    /// a glob (`src/**/*.rs`). Absolute, or relative to the session context path.
    /// Defaults to the session context path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,

    /// Case sensitive search.
    /// Default: smart case — case-sensitive if the pattern contains an uppercase
    /// letter, case-insensitive otherwise. Set explicitly to override.
    #[serde(skip_serializing_if = "Option::is_none")]
    // `ArgAction::SetTrue` on an `Option<bool>` yields `Some(false)` when the flag
    // is absent, not `None` — which reads as "the caller asked for insensitive" and
    // silently defeated smart case on every CLI search. Absent must stay `None`.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub case_sensitive: Option<bool>,

    /// Only search files matching these globs (e.g. `["*.rs", "**/test_*.py"]`).
    /// Matched against both the file name and the full path.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub include_globs: Option<Vec<String>>,

    /// Skip files matching these globs. Applied after `include_globs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub exclude_globs: Option<Vec<String>>,

    /// Maximum number of matches to return — or of *files*, when
    /// `line_numbers_only` is set, since that mode returns one line per file.
    /// The header always reports how many matches exist, so a truncated search is
    /// never mistaken for a complete one.
    /// Default: 20
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub max_results: Option<usize>,

    /// Lines of context before each match.
    /// Default: 2
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub context_before: Option<usize>,

    /// Lines of context after each match.
    /// Default: 6
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub context_after: Option<usize>,

    /// Return where the matches are instead of what they say: one line per file,
    /// giving the match count and the matching line numbers, with consecutive runs
    /// collapsed into ranges — `src/tools/read.rs (3 matches): 12, 88-90`. The
    /// ranges are written in `read`'s own `path:from-to` grammar, so the answer is
    /// the argument to the next call. Far cheaper than match output, and the right
    /// first move when the question is "which files are involved?".
    /// `context_before` / `context_after` do not apply.
    #[serde(default)]
    #[arg(long)]
    pub line_numbers_only: bool,
}

/// One pattern or several, OR-ed. Alternation is half of all real searches, and
/// writing it by hand is where the escaping goes wrong — so a caller may pass
/// `"needle"` or `["needle", "haystack"]` and mean the obvious thing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Patterns {
    One(String),
    Many(Vec<String>),
}

impl Patterns {
    /// Each alternative is wrapped in a non-capturing group before being joined:
    /// `["a b", "c"]` must not become `a b|c`, which alternates `a b` against `c`
    /// at a precedence the caller never wrote.
    fn to_regex(&self) -> String {
        match self {
            Self::One(pattern) => pattern.clone(),
            Self::Many(patterns) if patterns.len() == 1 => patterns[0].clone(),
            Self::Many(patterns) => patterns
                .iter()
                .map(|p| format!("(?:{p})"))
                .collect::<Vec<_>>()
                .join("|"),
        }
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        match self {
            Self::One(pattern) => std::slice::from_ref(pattern),
            Self::Many(patterns) => patterns.as_slice(),
        }
        .iter()
        .map(String::as_str)
    }
}

impl std::fmt::Display for Patterns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::One(pattern) => write!(f, "{pattern}"),
            Self::Many(patterns) => write!(f, "{}", patterns.join("\", \"")),
        }
    }
}

impl FromStr for Patterns {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::One(s.to_string()))
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
                description: "Search a directory for a pattern",
                item: Self {
                    pattern: Patterns::One("fn execute".into()),
                    paths: vec!["src/".into()],
                    ..Default::default()
                },
            },
            Example {
                description: "Several patterns at once, OR-ed, over a glob",
                item: Self {
                    pattern: Patterns::Many(vec!["TODO".into(), "FIXME".into()]),
                    paths: vec!["src/**/*.rs".into()],
                    context_before: Some(0),
                    context_after: Some(0),
                    ..Default::default()
                },
            },
            Example {
                description: "Which files are involved, and where — the cheap first pass",
                item: Self {
                    pattern: Patterns::One("resolve_path".into()),
                    line_numbers_only: true,
                    ..Default::default()
                },
            },
            Example {
                description: "Several explicit files, skipping tests",
                item: Self {
                    pattern: Patterns::One("max_results".into()),
                    paths: vec!["src/tools/search.rs".into(), "src/tools/list.rs".into()],
                    exclude_globs: Some(vec!["**/test*.rs".into()]),
                    ..Default::default()
                },
            },
        ]
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            pattern: Patterns::One(String::new()),
            paths: Vec::new(),
            case_sensitive: None,
            include_globs: None,
            exclude_globs: None,
            max_results: None,
            context_before: None,
            context_after: None,
            line_numbers_only: false,
        }
    }
}

impl Tool<FsTools> for Search {
    type Output = String;

    fn execute(self, state: &mut FsTools, _context: &RequestContext) -> Result<String> {
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!self.case_sensitive())
            .build(&self.pattern.to_regex())
            .context("Invalid regex pattern")?;

        let targets = self.collect_targets(state)?;
        self.search_targets(&targets, matcher)
    }
}

impl Search {
    /// Smart case, ripgrep's rule: an explicit setting wins, and otherwise a
    /// pattern carrying an uppercase letter is taken as case-sensitive. A caller
    /// who writes `TODO` means `TODO`; a caller who writes `todo` is looking for
    /// the word however it was spelled. The previous unconditional
    /// case-insensitive default made `Search`, `Result`, and every other Rust
    /// type name match its own lowercase field names throughout a file.
    fn case_sensitive(&self) -> bool {
        self.case_sensitive
            .unwrap_or_else(|| self.pattern.iter().any(has_uppercase_literal))
    }

    /// Twenty rather than fifty. Three quarters of real searches were piped into
    /// `head -10` or `-20`; those are line budgets rather than match budgets, so
    /// they do not transfer directly — but they say plainly that the old ceiling
    /// returned more than anyone read. The truthful total makes a low cap safe:
    /// a caller who wanted more can see that there is more.
    fn max_results(&self) -> usize {
        self.max_results.unwrap_or(20)
    }

    /// Asymmetric, because real usage is: `-A` 931 invocations against `-B` 214,
    /// and `-C` — the symmetric form this tool used to offer as its only option —
    /// exactly zero. A match is a place you have arrived at, and what you want is
    /// mostly what follows it.
    ///
    /// The large `-A` values in that same data (12, 25, 40) are not chased here.
    /// They are a workaround for the question "show me the body of the thing I
    /// found", which is a code-semantic question rather than a textual one; it
    /// belongs to a tree-sitter or LSP-backed tool, not to a line-oriented matcher
    /// pretending to be one.
    fn context_before(&self) -> usize {
        self.context_before.unwrap_or(2)
    }

    fn context_after(&self) -> usize {
        self.context_after.unwrap_or(6)
    }

    /// Resolve the requested paths — files, directories, and globs, in any mix —
    /// into the concrete list of files to search.
    ///
    /// Only 19 of 5,389 observed searches relied on an implicit working directory
    /// and a quarter named more than one operand, so the plural is the common
    /// case, not the exotic one. Order follows the request; a file reachable from
    /// two requested paths is searched once.
    fn collect_targets(&self, state: &mut FsTools) -> Result<Targets> {
        let include = compile_globs(self.include_globs.as_deref())?;
        let exclude = compile_globs(self.exclude_globs.as_deref())?;

        let requested = if self.paths.is_empty() {
            vec![".".to_string()]
        } else {
            self.paths.clone()
        };

        let mut targets = Targets::default();
        let mut seen = HashSet::new();

        for request in &requested {
            let (base, pattern) = split_glob(request);
            let base_path = state.resolve_path(if base.is_empty() { "." } else { base }, None)?;
            let glob = pattern.map(compile_glob).transpose()?;

            if !base_path.exists() {
                // A typo'd path that quietly returns "no matches" is the same
                // silent-failure shape as a truncation that does not say it
                // truncated: the answer looks like an answer.
                targets.missing.push(base_path.display().to_string());
                continue;
            }
            targets.requested.push(base_path.display().to_string());

            if glob.is_none() && base_path.is_file() {
                if seen.insert(base_path.clone()) {
                    targets.files.push(base_path);
                }
                continue;
            }

            // The `ignore` crate's standard filters — the same `.gitignore` /
            // `.ignore` / hidden-file handling `list` uses. A hand-rolled
            // `read_dir` recursion here once knew only a few hardcoded directory
            // names, so it descended into gitignored trees (a project's own log
            // and build output) and could return runaway results.
            let walker = WalkBuilder::new(&base_path).standard_filters(true).build();
            for entry in walker.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if let Some(glob) = &glob {
                    let relative = path.strip_prefix(&base_path).unwrap_or(path);
                    if !glob.is_match(relative) {
                        continue;
                    }
                }
                if !self.should_search_file(path, include.as_deref(), exclude.as_deref()) {
                    continue;
                }
                if seen.insert(path.to_path_buf()) {
                    targets.files.push(path.to_path_buf());
                }
            }
        }

        Ok(targets)
    }

    fn search_targets(&self, targets: &Targets, matcher: impl Matcher) -> Result<String> {
        let mut scan = Scan {
            results: Vec::new(),
            total: 0,
            collected: 0,
            files_with_matches: 0,
            max_results: self.max_results(),
        };

        for file in &targets.files {
            if scan.hit_ceiling() {
                break;
            }
            self.search_file(file, &matcher, &mut scan)?;
        }

        let mut output = if scan.results.is_empty() {
            // Name what was actually searched. A bare "no matches" cannot be told
            // apart from a glob that matched no files or a path that was a typo,
            // and those want opposite next moves.
            let files = targets.files.len();
            let plural = if files == 1 { "file" } else { "files" };
            format!(
                "No matches found for pattern \"{}\" in {files} {plural} under {}",
                self.pattern,
                targets.requested.join(", ")
            )
        } else if self.line_numbers_only {
            self.render_line_numbers(&scan)
        } else {
            self.render_matches(&scan)
        };

        if !targets.missing.is_empty() {
            let plural = if targets.missing.len() == 1 { "path" } else { "paths" };
            output.push_str(&format!(
                "\nNote: {} requested {plural} did not exist and {} not searched: {}\n",
                targets.missing.len(),
                if targets.missing.len() == 1 { "was" } else { "were" },
                targets.missing.join(", ")
            ));
        }

        Ok(output)
    }

    /// A capped search used to end at exactly `max_results` with nothing saying
    /// so — the "... and N more" line could never fire, because the counter it
    /// compared against was the one the scan stopped at. Fifty results read as
    /// "that is all of them", and a caller with 500 matches narrowed nothing.
    fn header(&self, scan: &Scan, truncated: bool, showing: usize, unit: &str) -> String {
        let count = if scan.hit_ceiling() {
            format!("{}+", scan.total)
        } else {
            scan.total.to_string()
        };
        let files = scan.files_with_matches;
        let plural = if files == 1 { "file" } else { "files" };
        if truncated {
            format!(
                "Found {count} matches for pattern \"{}\" in {files} {plural}, showing the first \
                 {showing} {unit} (raise max_results, or narrow the pattern):\n\n",
                self.pattern
            )
        } else {
            format!(
                "Found {count} matches for pattern \"{}\" in {files} {plural}:\n\n",
                self.pattern
            )
        }
    }

    /// One line per file: where the matches are, rather than what they say.
    fn render_line_numbers(&self, scan: &Scan) -> String {
        let truncated = scan.results.len() < scan.files_with_matches;
        let mut output = self.header(scan, truncated, scan.results.len(), "files");

        for file in &scan.results {
            let ranges = collapse_runs(&file.matches);
            let shown = ranges.len().min(MAX_RANGES_PER_FILE);
            let count = file.matches.len();
            let plural = if count == 1 { "match" } else { "matches" };
            output.push_str(&format!(
                "{} ({count} {plural}): {}",
                file.path,
                ranges[..shown].join(", ")
            ));
            if ranges.len() > shown {
                output.push_str(&format!(", … +{} more", ranges.len() - shown));
            }
            output.push('\n');
        }

        output
    }

    fn render_matches(&self, scan: &Scan) -> String {
        let truncated = scan.collected < scan.total;
        let mut output = self.header(scan, truncated, scan.collected, "matches");

        // Grouped by file: the path is a header over its block instead of a
        // prefix on every printed line. Measured over 165 real results, the
        // repeated prefix was 46% of the bytes that reached a transcript —
        // and the attentional cost is the same shape, a wall of one long
        // identical path with the varying part (line, content) at the tail
        // of each line. Context lines multiply it: one match at one line of
        // context paid for the path three times.
        //
        // The header stays **absolute**. A stated-once relative base is
        // exactly what a later paste-the-path step loses — `list` was
        // observed answering "All paths relative to X" and having the base
        // dropped, costing two failed reads and a reconcile round. An entry
        // here stays valid to paste into the next call unchanged; with the
        // header and the line numbers together it composes directly into
        // `read`'s `path:from-to`.
        for file in &scan.results {
            output.push_str(&file.path);
            output.push_str(":\n");

            let mut previous = None;
            for (&number, text) in &file.lines {
                // A gap marker is for an actual gap. It used to print between
                // every pair of matches in a file whether or not their blocks
                // were contiguous, so adjacent context read as separated.
                if previous.is_some_and(|p| number > p + 1) {
                    output.push_str("--\n");
                }

                // The gutter is grep's: `:` on a matched line, `-` on a context
                // line. It costs nothing (one punctuation character either way),
                // it sits outside the content instead of inside it, and it is the
                // convention every reader of `rg -C` output already knows. The
                // delimiters it replaced — `┌─match─┐` and friends — were the
                // opposite trade: they edited the line so it no longer matched
                // the bytes on disk, and they split the matched text into tokens
                // that no longer looked like the pattern that was searched for.
                let gutter = if file.matches.contains(&number) { ':' } else { '-' };
                output.push_str(&format!("  {number}{gutter} {text}\n"));
                previous = Some(number);
            }
            output.push('\n');
        }

        output.trim_end().to_string() + "\n"
    }

    fn search_file(&self, file_path: &Path, matcher: &impl Matcher, scan: &mut Scan) -> Result<()> {
        // In `line_numbers_only` the cap counts files, since that is the unit the
        // mode returns; in match mode it counts matches. Either way the scan keeps
        // *counting* past the cap so the header can be honest.
        let collecting = if self.line_numbers_only {
            scan.results.len() < scan.max_results
        } else {
            scan.collected < scan.max_results
        };

        // Unreadable or non-UTF-8 (binary) files are skipped, not fatal: the walk
        // reaches more files than it used to, and one stray binary must not abort
        // the whole search.
        let Ok(content) = std::fs::read_to_string(file_path) else {
            return Ok(());
        };

        let lines: Vec<&str> = content.lines().collect();
        let before = self.context_before();
        let after = self.context_after();
        let mut file_matches: Option<FileMatches> = None;
        let mut matched = false;

        for (line_idx, line) in lines.iter().enumerate() {
            if scan.hit_ceiling() {
                break;
            }

            let Some(m) = matcher
                .find(line.as_bytes())
                .map_err(|e| anyhow!("Matcher error: {}", e))?
            else {
                continue;
            };

            // Counted whether or not it is collected: the total is what tells a
            // caller their 20 results are 20 of 312.
            scan.total += 1;
            matched = true;
            if !collecting {
                continue;
            }
            if !self.line_numbers_only {
                if scan.collected >= scan.max_results {
                    continue;
                }
                scan.collected += 1;
            }

            let number = (line_idx + 1) as u64;
            let file = file_matches.get_or_insert_with(|| FileMatches::new(file_path));
            file.matches.insert(number);

            if self.line_numbers_only {
                continue;
            }

            // Lines land in a map keyed by line number, which is what makes
            // overlapping windows merge instead of repeat. Two matches three
            // lines apart used to print their blocks independently, so the lines
            // between them arrived twice — 27 printed lines where 21 were meant,
            // with the same line number appearing at two different places in the
            // output. `or_insert` also means a match line already placed is never
            // overwritten by its neighbour's view of it as mere context.
            let start = line_idx.saturating_sub(before);
            let end = (line_idx + 1 + after).min(lines.len());
            for (offset, context) in lines[start..end].iter().enumerate() {
                file.lines
                    .entry((start + offset + 1) as u64)
                    .or_insert_with(|| clamp_line(context));
            }

            // Window the matched line around the match so one giant line cannot
            // dominate. Short lines pass through whole, indentation included.
            file.lines
                .insert(number, window_match(line, m.start(), m.end()));
        }

        // Files past the cap still count toward the file total the header reports,
        // so "in 40 files, showing the first 20" is a true sentence.
        if matched {
            scan.files_with_matches += 1;
        }
        if let Some(file) = file_matches {
            scan.results.push(file);
        }

        Ok(())
    }

    fn should_search_file(
        &self,
        path: &Path,
        include: Option<&[GlobMatcher]>,
        exclude: Option<&[GlobMatcher]>,
    ) -> bool {
        // Default exclusions for common non-source directories, belt-and-suspenders
        // for repos with no .gitignore.
        let path_str = path.to_string_lossy();
        if ["/.git/", "/target/", "/node_modules/", "/.svn/", "/.hg/"]
            .iter()
            .any(|dir| path_str.contains(dir))
        {
            return false;
        }

        // `--include='*.rs'` is the dominant real idiom — 365 uses against 45 of
        // rg's `--type` names — and it is matched against the file name as often
        // as against the path, so both are tried.
        let name = path.file_name().map(Path::new).unwrap_or(path);
        if let Some(include) = include {
            if !include.iter().any(|g| g.is_match(path) || g.is_match(name)) {
                return false;
            }
        }
        if let Some(exclude) = exclude {
            if exclude.iter().any(|g| g.is_match(path) || g.is_match(name)) {
                return false;
            }
        }

        // Skip common binary extensions.
        match path.extension().and_then(|s| s.to_str()) {
            Some(ext) => !matches!(
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
            ),
            // Files without extensions are usually text
            None => true,
        }
    }
}

/// The files a request resolved to, and the paths it named that were not there.
#[derive(Debug, Default)]
struct Targets {
    files: Vec<PathBuf>,
    missing: Vec<String>,
    /// The resolved paths as requested, for saying where a fruitless search looked.
    requested: Vec<String>,
}

/// Running state of one search: what has been collected, and how much exists.
#[derive(Debug)]
struct Scan {
    results: Vec<FileMatches>,
    /// Every match seen, including those past `max_results`.
    total: usize,
    /// Matches actually placed in `results`.
    collected: usize,
    /// Files with at least one match, including those past the cap.
    files_with_matches: usize,
    max_results: usize,
}

impl Scan {
    /// Counting past the results the caller asked for is nearly free per file but
    /// not free per repo, so it stops here and the total is reported as `N+`.
    fn hit_ceiling(&self) -> bool {
        self.total >= COUNT_CEILING.max(self.max_results)
    }
}

/// One file's contribution to the output: the union of every match's window,
/// keyed by line number so overlapping windows merge rather than repeat.
#[derive(Debug)]
struct FileMatches {
    path: String,
    lines: BTreeMap<u64, String>,
    matches: BTreeSet<u64>,
}

impl FileMatches {
    fn new(path: &Path) -> Self {
        Self {
            path: path.display().to_string(),
            lines: BTreeMap::new(),
            matches: BTreeSet::new(),
        }
    }
}

/// Collapse sorted line numbers into `read`-grammar ranges: `10,11,12,14` becomes
/// `["10-12", "14"]`. Lossless — only genuinely consecutive lines merge, so a range
/// never claims a match on a line that has none.
fn collapse_runs(numbers: &BTreeSet<u64>) -> Vec<String> {
    let mut ranges = Vec::new();
    let mut iter = numbers.iter().copied();
    let Some(mut start) = iter.next() else {
        return ranges;
    };
    let mut end = start;

    for number in iter {
        if number == end + 1 {
            end = number;
        } else {
            ranges.push(format_range(start, end));
            start = number;
            end = number;
        }
    }
    ranges.push(format_range(start, end));
    ranges
}

fn format_range(start: u64, end: u64) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

/// Split a requested path into the directory to walk and the glob to match within
/// it, or `None` when the request names no glob. The same split `list` performs:
/// the base is everything up to the last separator before the first glob character.
fn split_glob(request: &str) -> (&str, Option<&str>) {
    let Some(first) = request.find(is_glob_char) else {
        return (request, None);
    };
    match request[..first].rfind(['/', '\\']) {
        Some(separator) => (&request[..separator], Some(&request[separator + 1..])),
        None => ("", Some(request)),
    }
}

fn is_glob_char(c: char) -> bool {
    matches!(c, '*' | '?' | '[' | '{')
}

/// `globset` rather than `glob`: the latter has no brace alternation at all, so it
/// compiled `{a,b}` without complaint and then matched nothing.
fn compile_glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|e| anyhow!("invalid glob pattern {pattern:?}: {e}"))?
        .compile_matcher())
}

fn compile_globs(patterns: Option<&[String]>) -> Result<Option<Vec<GlobMatcher>>> {
    patterns
        .map(|patterns| patterns.iter().map(|p| compile_glob(p)).collect())
        .transpose()
}

/// Whether a pattern contains an uppercase *literal*, for smart case. A char
/// after a backslash is part of an escape — `\S`, `\W`, `\D`, `\B` are classes,
/// not a caller asking for case-sensitivity.
fn has_uppercase_literal(pattern: &str) -> bool {
    let mut escaped = false;
    for c in pattern.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c.is_uppercase() {
            return true;
        }
    }
    false
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
    let line = line.trim_end();
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

/// Truncate a (matchless) context line to `MAX_CONTEXT_LINE` bytes of *content*,
/// eliding the rest with `…`.
///
/// Leading whitespace is kept. It was stripped when this output was shaped for an
/// 8k context and every byte was worth arguing about; at 18.6% of the bytes of
/// this repo's source that looked like a large saving, but indentation is ~1 token
/// a line to any tokenizer with multi-space tokens — around 8% of a response, and
/// it buys back the two things trimming cost: a line that can be pasted into a
/// `write` unchanged, and agreement with `read`, which never trimmed. The indent
/// also does a share of the visual work the match delimiters were there for.
/// Trailing whitespace is invisible and has no such defence, so it still goes.
///
/// The byte budget is measured against the content rather than the whole line, so
/// a deeply nested line is not clipped early for being nested.
fn clamp_line(line: &str) -> String {
    let line = line.trim_end();
    let indent = line.len() - line.trim_start().len();
    if line.len() - indent <= MAX_CONTEXT_LINE {
        return line.to_string();
    }
    let hi = floor_boundary(line, indent + MAX_CONTEXT_LINE);
    format!("{}…", &line[..hi])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tools(root: &Path) -> FsTools {
        FsTools::embedded(root.to_path_buf()).unwrap()
    }

    fn search_for(pattern: &str) -> Search {
        Search {
            pattern: Patterns::One(pattern.into()),
            case_sensitive: Some(false),
            context_before: Some(0),
            context_after: Some(0),
            max_results: Some(50),
            ..Default::default()
        }
    }

    fn run(search: Search, root: &Path) -> String {
        search
            .execute(&mut tools(root), &RequestContext::default())
            .unwrap()
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

        let out = run(search_for("needle"), root);
        assert!(out.contains("keep.txt"), "tracked file searched: {out}");
        assert!(!out.contains("skip.txt"), "gitignored tree skipped: {out}");
    }

    /// The path is a header over its block, not a prefix on every printed line:
    /// across 165 real results the repeated prefix was 46% of the bytes that
    /// reached a transcript.
    #[test]
    fn matches_are_grouped_under_one_path_header() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "needle one\nneedle two\nneedle three\n").unwrap();

        let out = run(search_for("needle"), root);
        let path = root.join("a.txt").display().to_string();
        assert_eq!(
            out.matches(&path).count(),
            1,
            "the path is printed once, not per line: {out}"
        );
        for line in ["  1: needle one", "  2: needle two", "  3: needle three"] {
            assert!(out.contains(line), "bare line gutter {line:?}: {out}");
        }
    }

    /// Each file opens its own header, and the header is absolute, so an entry is
    /// valid to paste into the next call unchanged.
    #[test]
    fn each_file_gets_its_own_absolute_header() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "needle a\n").unwrap();
        fs::write(root.join("b.txt"), "needle b\n").unwrap();

        let out = run(search_for("needle"), root);
        for name in ["a.txt", "b.txt"] {
            let header = format!("{}:\n", root.join(name).display());
            assert!(out.contains(&header), "header for {name}: {out}");
        }
        assert!(!out.contains(".txt:1: "), "no per-line path prefix survives: {out}");
    }

    /// A matched line is marked `:` and a context line `-`, grep's own gutter.
    #[test]
    fn the_gutter_distinguishes_a_match_from_its_context() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "before\nthe needle\nafter\n").unwrap();

        let mut search = search_for("needle");
        search.context_before = Some(1);
        search.context_after = Some(1);
        let out = run(search, root);
        assert!(out.contains("  1- before"), "{out}");
        assert!(out.contains("  2: the needle"), "{out}");
        assert!(out.contains("  3- after"), "{out}");
    }

    /// The matched line arrives exactly as it sits on disk — no delimiters
    /// inserted into it, so it still parses and still pastes into a `write`.
    #[test]
    fn a_matched_line_is_returned_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.rs"), "fn needle(x: usize) -> usize { x }\n").unwrap();

        let out = run(search_for("needle"), root);
        assert!(out.contains("  1: fn needle(x: usize) -> usize { x }"), "{out}");
    }

    /// Indentation is content. It was trimmed when this output was shaped for an
    /// 8k window; it is ~1 token a line, and it is load-bearing in Python and
    /// YAML and paste-breaking everywhere.
    #[test]
    fn indentation_is_preserved_and_trailing_whitespace_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.py"), "def f():\n    if x:\n        needle()   \n").unwrap();

        let mut search = search_for("needle");
        search.context_before = Some(1);
        let out = run(search, root);
        assert!(out.contains("  2-     if x:"), "context indent kept: {out}");
        assert!(out.contains("  3:         needle()\n"), "match indent kept, trailing gone: {out}");
    }

    /// Two matches whose windows overlap print one merged block. They used to
    /// print independently, so the lines between them arrived twice, under the
    /// same line numbers, at two places in the output.
    #[test]
    fn overlapping_context_windows_merge_instead_of_repeating() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body: String = (1..=10)
            .map(|n| if n == 4 || n == 6 { format!("needle {n}\n") } else { format!("line {n}\n") })
            .collect();
        fs::write(root.join("a.txt"), body).unwrap();

        let mut search = search_for("needle");
        search.context_before = Some(2);
        search.context_after = Some(2);
        let out = run(search, root);
        assert_eq!(out.matches("  5- line 5").count(), 1, "shared line printed once: {out}");
        assert!(!out.contains("--"), "one contiguous block, no gap marker: {out}");
        for expected in ["  2- line 2", "  4: needle 4", "  6: needle 6", "  8- line 8"] {
            assert!(out.contains(expected), "{expected:?} in {out}");
        }
    }

    /// Blocks that merely abut, with no line between them, are one block too.
    #[test]
    fn adjacent_blocks_do_not_get_a_gap_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body: String = (1..=12)
            .map(|n| if n == 3 || n == 8 { format!("needle {n}\n") } else { format!("line {n}\n") })
            .collect();
        fs::write(root.join("a.txt"), body).unwrap();

        // Windows are 1-5 and 6-10: contiguous, so they are one block.
        let mut search = search_for("needle");
        search.context_before = Some(2);
        search.context_after = Some(2);
        let out = run(search, root);
        assert!(!out.contains("--"), "contiguous windows are one block: {out}");
    }

    /// Within one file, non-contiguous blocks keep a gap marker.
    #[test]
    fn a_gap_within_one_file_keeps_its_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut body = String::from("needle at the top\n");
        for _ in 0..20 {
            body.push_str("filler\n");
        }
        body.push_str("needle at the bottom\n");
        fs::write(root.join("a.txt"), body).unwrap();

        let mut search = search_for("needle");
        search.context_before = Some(1);
        search.context_after = Some(1);
        let out = run(search, root);
        assert!(out.contains("\n--\n"), "gap marker between blocks: {out}");
        assert_eq!(
            out.matches(&root.join("a.txt").display().to_string()).count(),
            1,
            "still one header for the file: {out}"
        );
    }

    /// A capped search says how many matches exist. It used to stop at exactly
    /// `max_results` and compare that same counter against the limit, so the
    /// "and N more" line could never fire: the cap read as completeness.
    #[test]
    fn a_truncated_search_reports_the_total_it_did_not_return() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body: String = (1..=40).map(|n| format!("needle {n}\n")).collect();
        fs::write(root.join("a.txt"), body).unwrap();

        let mut search = search_for("needle");
        search.max_results = Some(5);
        let out = run(search, root);
        assert!(out.contains("Found 40 matches"), "{out}");
        assert!(out.contains("showing the first 5 matches"), "{out}");
        assert!(out.contains("  5: needle 5"), "{out}");
        assert!(!out.contains("  6: needle 6"), "capped: {out}");
    }

    /// An uncapped search says nothing about limits.
    #[test]
    fn a_complete_search_reports_a_plain_count() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "needle\nneedle\n").unwrap();
        let out = run(search_for("needle"), root);
        assert!(out.contains("Found 2 matches"), "{out}");
        assert!(out.contains("in 1 file"), "{out}");
        assert!(!out.contains("showing the first"), "{out}");
    }

    /// Smart case: an uppercase letter in the pattern means the caller meant it.
    #[test]
    fn case_sensitivity_follows_the_pattern_unless_stated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "Needle upper\nneedle lower\n").unwrap();

        let mut sensitive = search_for("Needle");
        sensitive.case_sensitive = None;
        let out = run(sensitive, root);
        assert!(out.contains("Needle upper"), "{out}");
        assert!(!out.contains("needle lower"), "uppercase pattern is literal: {out}");

        let mut insensitive = search_for("needle");
        insensitive.case_sensitive = None;
        let out = run(insensitive, root);
        assert!(out.contains("Needle upper") && out.contains("needle lower"), "{out}");

        let mut forced = search_for("Needle");
        forced.case_sensitive = Some(false);
        let out = run(forced, root);
        assert!(out.contains("needle lower"), "explicit false overrides: {out}");
    }

    /// `\S` and friends are classes, not a caller reaching for case sensitivity.
    #[test]
    fn an_escape_is_not_an_uppercase_literal() {
        assert!(!has_uppercase_literal(r"\S+needle"));
        assert!(!has_uppercase_literal(r"fn \w+"));
        assert!(has_uppercase_literal("TODO"));
        assert!(has_uppercase_literal(r"\s+Needle"));
        assert!(has_uppercase_literal("[A-Z]"));
    }

    /// Half of real searches are alternations, so a list of patterns is OR-ed.
    #[test]
    fn several_patterns_are_or_ed_together() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "alpha\nbeta\ngamma\n").unwrap();

        let mut search = search_for("unused");
        search.pattern = Patterns::Many(vec!["alpha".into(), "gamma".into()]);
        let out = run(search, root);
        assert!(out.contains("alpha") && out.contains("gamma"), "{out}");
        assert!(!out.contains("  2: beta"), "{out}");
    }

    /// Each alternative is grouped before being joined. Written by hand,
    /// `a b|c` alternates `a␣b` against `c` — a precedence nobody intends, and
    /// the single most common way an alternation written by hand goes wrong.
    #[test]
    fn alternatives_are_grouped_so_precedence_is_what_was_written() {
        let joined = Patterns::Many(vec!["a b".into(), "c".into()]).to_regex();
        assert_eq!(joined, "(?:a b)|(?:c)");
        // A single-element list is not gratuitously wrapped.
        assert_eq!(Patterns::Many(vec!["solo".into()]).to_regex(), "solo");
        assert_eq!(Patterns::One("solo".into()).to_regex(), "solo");
    }

    /// Paths are plural and may mix files, directories, and globs. A file
    /// reachable from two requested paths is searched once.
    #[test]
    fn paths_accept_files_directories_and_globs_without_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "needle a\n").unwrap();
        fs::write(root.join("src/b.rs"), "needle b\n").unwrap();
        fs::write(root.join("src/c.txt"), "needle c\n").unwrap();

        let mut search = search_for("needle");
        // The same file named directly, via a glob, and via its directory.
        search.paths = vec!["src/a.rs".into(), "src/*.rs".into(), "src".into()];
        let out = run(search, root);
        assert_eq!(out.matches("needle a").count(), 1, "searched once: {out}");
        assert!(out.contains("needle b") && out.contains("needle c"), "{out}");
        assert!(out.contains("Found 3 matches"), "{out}");
    }

    /// A glob restricts the walk to what it names.
    #[test]
    fn a_glob_path_limits_the_files_searched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/deep")).unwrap();
        fs::write(root.join("src/a.rs"), "needle rs\n").unwrap();
        fs::write(root.join("src/deep/b.rs"), "needle deep\n").unwrap();
        fs::write(root.join("src/c.txt"), "needle txt\n").unwrap();

        let mut search = search_for("needle");
        search.paths = vec!["src/**/*.rs".into()];
        let out = run(search, root);
        assert!(out.contains("needle rs") && out.contains("needle deep"), "{out}");
        assert!(!out.contains("needle txt"), "{out}");
    }

    /// `--include='*.rs'` is the dominant real idiom, so globs match the file
    /// name as well as the path; excludes are applied after includes.
    #[test]
    fn include_and_exclude_globs_filter_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.rs"), "needle a\n").unwrap();
        fs::write(root.join("test_a.rs"), "needle test\n").unwrap();
        fs::write(root.join("a.md"), "needle md\n").unwrap();

        let mut search = search_for("needle");
        search.include_globs = Some(vec!["*.rs".into()]);
        search.exclude_globs = Some(vec!["test_*.rs".into()]);
        let out = run(search, root);
        assert!(out.contains("needle a"), "{out}");
        assert!(!out.contains("needle md"), "include filtered: {out}");
        assert!(!out.contains("needle test"), "exclude filtered: {out}");
    }

    /// `line_numbers_only` answers "which files, and where" in one line each:
    /// the file list, the count, and the positions — with consecutive runs
    /// collapsed into `read`'s own range grammar.
    #[test]
    fn line_numbers_only_reports_paths_counts_and_collapsed_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Matches on 1,2,3 and 7 — one run and one singleton.
        fs::write(
            root.join("a.txt"),
            "needle\nneedle\nneedle\nx\nx\nx\nneedle\n",
        )
        .unwrap();

        let mut search = search_for("needle");
        search.line_numbers_only = true;
        let out = run(search, root);
        assert!(out.contains("(4 matches): 1-3, 7"), "{out}");
        assert!(!out.contains("  1: needle"), "no contents in this mode: {out}");
        assert!(out.contains("Found 4 matches"), "{out}");
    }

    /// A single match reads as one, and a lone line number is not a range.
    #[test]
    fn line_numbers_only_singular_forms() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "x\nneedle\n").unwrap();

        let mut search = search_for("needle");
        search.line_numbers_only = true;
        let out = run(search, root);
        assert!(out.contains("(1 match): 2\n"), "{out}");
    }

    /// In this mode the cap counts files, because files are what the mode
    /// returns — and the header still names how many there were.
    #[test]
    fn line_numbers_only_caps_files_rather_than_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(root.join(name), "needle\nneedle\n").unwrap();
        }

        let mut search = search_for("needle");
        search.line_numbers_only = true;
        search.max_results = Some(2);
        let out = run(search, root);
        assert!(out.contains("Found 6 matches"), "all counted: {out}");
        assert!(out.contains("in 3 files"), "all files counted: {out}");
        assert!(out.contains("showing the first 2 files"), "{out}");
        assert_eq!(out.matches("(2 matches)").count(), 2, "two files listed: {out}");
    }

    /// A file with hundreds of scattered matches degrades into a path and a
    /// count rather than hundreds of numbers — which is the broad-scout case
    /// that a bare file list would otherwise have been needed for.
    #[test]
    fn line_numbers_only_caps_the_ranges_listed_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A match on every odd line, so no two runs merge.
        let body: String = (1..=60)
            .map(|n| if n % 2 == 1 { "needle\n" } else { "x\n" })
            .collect();
        fs::write(root.join("a.txt"), body).unwrap();

        let mut search = search_for("needle");
        search.line_numbers_only = true;
        let out = run(search, root);
        assert!(out.contains("(30 matches)"), "{out}");
        assert!(out.contains("+20 more"), "capped at {MAX_RANGES_PER_FILE}: {out}");
    }

    /// A path that isn't there is said out loud. Returning "no matches" for a
    /// typo is the same silent-failure shape as a cap that doesn't say it capped.
    #[test]
    fn a_missing_requested_path_is_reported_not_silently_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "needle\n").unwrap();

        let mut search = search_for("needle");
        search.paths = vec!["a.txt".into(), "nope.txt".into()];
        let out = run(search, root);
        assert!(out.contains("needle"), "the real path still searched: {out}");
        assert!(out.contains("did not exist"), "{out}");
        assert!(out.contains("nope.txt"), "{out}");
    }

    /// A fruitless search names what it searched. A bare "no matches" cannot be
    /// told apart from a glob that matched no files or a path that was a typo,
    /// and those want opposite next moves.
    #[test]
    fn a_fruitless_search_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "haystack
").unwrap();

        let out = run(search_for("needle"), root);
        assert!(out.contains("No matches found"), "{out}");
        assert!(out.contains("in 1 file under"), "{out}");
        assert!(out.contains(&root.display().to_string()), "{out}");
    }

    /// An explicitly named file is searched even if a walk would have ignored
    /// it — intentional searches of generated files still work.
    #[test]
    fn explicitly_named_ignored_file_is_still_searched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("app.log"), "needle in a log\n").unwrap();

        let mut search = search_for("needle");
        search.paths = vec!["app.log".into()];
        let out = run(search, root);
        assert!(out.contains("app.log"), "named ignored file searched: {out}");
    }

    /// A non-UTF-8 / unreadable file doesn't abort the whole search.
    #[test]
    fn binary_files_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("bin.dat"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        fs::write(root.join("keep.txt"), "needle\n").unwrap();
        let out = run(search_for("needle"), root);
        assert!(out.contains("keep.txt"), "search completed past the binary: {out}");
    }

    /// Context is asymmetric and forward-leaning: `-A` outnumbered `-B` 931 to
    /// 214 in real usage, and the symmetric `-C` this tool used to offer as its
    /// only shape was used exactly zero times.
    #[test]
    fn the_default_context_leans_forward() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        fs::write(root.join("a.txt"), body.replace("line 10", "needle here")).unwrap();

        let mut search = search_for("needle");
        search.context_before = None;
        search.context_after = None;
        let out = run(search, root);
        assert!(out.contains("8- line 8"), "two lines before: {out}");
        assert!(!out.contains("7- line 7"), "and no more: {out}");
        assert!(out.contains("16- line 16"), "six lines after: {out}");
        assert!(!out.contains("17- line 17"), "and no more: {out}");
    }

    #[test]
    fn collapse_runs_merges_only_consecutive_lines() {
        let numbers: BTreeSet<u64> = [1, 2, 3, 7, 9, 10].into_iter().collect();
        assert_eq!(collapse_runs(&numbers), vec!["1-3", "7", "9-10"]);
        assert!(collapse_runs(&BTreeSet::new()).is_empty());
    }

    #[test]
    fn split_glob_finds_the_base_directory() {
        assert_eq!(split_glob("src/**/*.rs"), ("src", Some("**/*.rs")));
        assert_eq!(split_glob("src/a.rs"), ("src/a.rs", None));
        assert_eq!(split_glob("*.rs"), ("", Some("*.rs")));
        assert_eq!(split_glob("a/b/c"), ("a/b/c", None));
    }

    #[test]
    fn window_match_trims_around_the_match() {
        let line = format!("{}NEEDLE{}", "a".repeat(500), "b".repeat(500));
        let windowed = window_match(&line, 500, 506);
        assert!(windowed.contains("NEEDLE"), "match kept: {windowed}");
        assert!(windowed.starts_with('…') && windowed.ends_with('…'), "both ends elided");
        assert!(windowed.len() < line.len(), "shorter than the raw line");
        assert!(windowed.len() <= 6 + 2 * MATCH_CONTEXT + 8, "bounded: {}", windowed.len());
    }

    #[test]
    fn window_match_leaves_short_lines_untouched() {
        assert_eq!(window_match("short needle line", 6, 12), "short needle line");
    }

    #[test]
    fn window_and_clamp_never_split_a_codepoint() {
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

    /// The clamp budget is content, not the whole line, so nesting does not eat
    /// into how much of a line survives.
    #[test]
    fn the_clamp_budget_is_measured_past_the_indent() {
        let line = format!("{}{}", " ".repeat(24), "z".repeat(1000));
        let c = clamp_line(&line);
        assert!(c.starts_with("                        z"), "indent kept: {c:?}");
        assert_eq!(c.matches('z').count(), MAX_CONTEXT_LINE, "full content budget: {}", c.len());
    }
}
