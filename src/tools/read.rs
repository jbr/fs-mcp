use crate::tools::FsTools;
use anyhow::{Context, Result, anyhow};
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use serde::{Deserialize, Serialize};
use std::{io::Read as _, path::Path};

/// Read utf8 contents from a file. Non-utf8 characters will be replaced lossily
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "read")]
pub struct Read {
    /// Path or paths to read.
    /// Can be absolute, or relative to session context path.
    ///
    /// A path is a path only — a `file.rs:120-160` suffix is not a line range and
    /// names no file. To read part of a file, pass the plain path and set
    /// `from_line` / `to_line`.
    pub paths: Vec<String>,

    /// Max length in bytes to read. Will truncate response and indicate truncation.
    /// Final character may be a replacement character if truncated mid code point
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub max_length: Option<usize>,

    /// Start line (1-indexed). When this is past line 1, every returned line is
    /// prefixed with its file line number and a tab, so positions in the result
    /// need no offset arithmetic; the prefix is not part of the file's text.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub from_line: Option<usize>,

    /// End line (1-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub to_line: Option<usize>,
}

impl ToolMeta for Read {
    fn title() -> Option<&'static str> {
        Some("Read files")
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
                description: "Reading a file relative to a session context",
                item: Self {
                    paths: vec!["src/main.rs".into()],
                    max_length: None,
                    from_line: None,
                    to_line: None,
                },
            },
            Example {
                description: "Reading the head of a file by absolute path",
                item: Self {
                    paths: vec!["/some/absolute/path/src/main.rs".into()],
                    max_length: Some(100),
                    from_line: None,
                    to_line: None,
                },
            },
            Example {
                description: "Reading one region of a file — the line range goes in from_line/to_line, never in the path",
                item: Self {
                    paths: vec!["src/tools/read.rs".into()],
                    max_length: None,
                    from_line: Some(167),
                    to_line: Some(177),
                },
            },
            Example {
                description: "Reading several files at once",
                item: Self {
                    paths: vec![
                        "src/main.rs".into(),
                        "src/tools.rs".into(),
                        "src/tools/read.rs".into(),
                    ],
                    max_length: None,
                    from_line: None,
                    to_line: None,
                },
            },
        ]
    }
}

impl Tool<FsTools> for Read {
    type Output = String;

    fn execute(self, state: &mut FsTools, _context: &RequestContext) -> Result<String> {
        let separator = std::iter::repeat_with(fastrand::alphanumeric)
            .take(10)
            .collect::<String>();
        Ok(self
            .paths
            .iter()
            .map(|path| {
                self.read_file(state, path, &separator).unwrap_or_else(|e| {
                    format!(
                        "=={separator} BEGIN ERROR {path} {separator}==\n\
                        {e}\n=={separator} END ERROR {path} {separator}=="
                    )
                })
            })
            .collect())
    }
}

impl Read {
    #[allow(dead_code)]
    fn read_head(
        &self,
        path: &Path,
        max_length: usize,
        actual_length: usize,
        separator: &str,
    ) -> Result<String> {
        let mut bytes = vec![0u8; max_length];
        std::fs::File::open(path)
            .with_context(|| format!("Unable to open {}", path.display()))?
            .read_exact(&mut bytes)
            .with_context(|| format!("Unable to read from {}", path.display()))?;
        Ok(format!(
            "=={separator} BEGIN TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n\
            {content}\n\
            =={separator} END TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n",
            path = path.display(),
            content = String::from_utf8_lossy(&bytes)
        ))
    }

    fn read_head_from_string(
        &self,
        path: &Path,
        full_contents: &str,
        max_length: usize,
        actual_length: usize,
        separator: &str,
    ) -> Result<String> {
        // Walk back to a char boundary: slicing a `str` mid-code-point panics, and
        // `max_length` is a byte count the caller chose without knowing the encoding.
        let mut cut = max_length.min(full_contents.len());
        while cut > 0 && !full_contents.is_char_boundary(cut) {
            cut -= 1;
        }
        let truncated = &full_contents[..cut];
        Ok(format!(
            "=={separator} BEGIN TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n\
            {content}\n\
            =={separator} END TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n",
            path = path.display(),
            content = truncated
        ))
    }

    fn read_file(&self, state: &mut FsTools, requested: &str, separator: &str) -> Result<String> {
        let path = state.resolve_path(requested, None)?;

        if !path.exists() {
            // `src/project.rs:578-700` is what a model writes when it wants a
            // region and hasn't found `from_line`/`to_line`; observed six times in
            // one session, each a hard "does not exist" that taught nothing. The
            // path is still wrong — the suffix names no file — so this stays an
            // error, but it names the option that answers the request.
            if let Some((base, from, to)) = line_suffix(requested) {
                let base_path = state.resolve_path(base, None)?;
                if base_path.exists() {
                    let to = to.map(|t| format!(", to_line: {t}")).unwrap_or_default();
                    return Err(anyhow!(
                        "{} does not exist. A `:line-range` suffix is part of no filename — \
                         to read that region, call read with paths: [{base:?}], from_line: \
                         {from}{to}",
                        path.display(),
                    ));
                }
            }
            return Err(anyhow!("{} does not exist", path.display()));
        }

        let full_contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Unable to read {}", path.display()))?;

        // The line window is applied *before* `max_length`, because `max_length` is a
        // size cap on what was asked for, not a substitute for it. Truncating first
        // silently discarded `from_line`/`to_line` and returned the head of the file —
        // a caller asking for line 125 with a 20-byte cap got the first 20 bytes of
        // line 1, with nothing in the response to say so.
        let mut content_to_return = full_contents.clone();
        let mut window = None;

        if let Some(from_line) = self.from_line {
            let lines: Vec<&str> = full_contents.lines().collect();
            let start = from_line.saturating_sub(1);
            if start >= lines.len() {
                return Err(anyhow!(
                    "from_line {} is beyond end of file ({} lines)",
                    from_line,
                    lines.len()
                ));
            }

            let end = self
                .to_line
                .unwrap_or(lines.len())
                .saturating_sub(1)
                .min(lines.len() - 1);

            if start > end {
                return Err(anyhow!(
                    "from_line {} must be <= to_line {}",
                    from_line,
                    self.to_line.unwrap_or(lines.len())
                ));
            }

            // A window that starts past line 1 numbers each line with its file
            // line number. Without the prefix a reader has to add `from_line - 1` to
            // every position it reasons about, and a model re-targeting the region
            // was observed adding the offset once and then treating the relative
            // numbers as absolute (a hung turn and an extra search to reorient).
            // A window starting at line 1 needs no arithmetic, so it stays bare:
            // the numbers would only be bytes. Applied before `max_length` so a
            // truncated offset read is still numbered.
            content_to_return = if start > 0 {
                lines[start..=end]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}\t{line}", start + 1 + i))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                lines[start..=end].join("\n")
            };
            window = Some((start + 1, end + 1));
            // If the original file didn't end with a newline, joining might add one or lose context.
            // But for a tool result, this is generally acceptable.
        }

        let selected_length = content_to_return.len();
        if let Some(max_length) = self.max_length {
            if max_length < selected_length {
                return self.read_head_from_string(
                    &path,
                    &content_to_return,
                    max_length,
                    selected_length,
                    separator,
                );
            }
        }

        // The window rides *after* `LENGTH:` so the sentinel's path field is
        // unchanged: efference's transcript parsers take the path as everything
        // between ` BEGIN ` and `, LENGTH:` (docs/tui.md), and adding a field there
        // would silently change what they read a path as. Stating the range is what
        // makes a windowed read re-targetable — a line number in the result and a
        // line number in an `edit` mean the same thing.
        let lines = window
            .map(|(from, to)| format!(", LINES: {from}-{to}"))
            .unwrap_or_default();
        Ok(format!(
            "=={separator} BEGIN {path}, LENGTH: {len}{lines} {separator}==\n\
            {content_to_return}\n=={separator} END {path}, LENGTH: {len}{lines} {separator}==\n",
            path = path.display(),
            len = full_contents.len(),
        ))
    }
}

/// Split a `path:120-160` / `path:120` request into `(path, from, to)`, or `None`
/// when the string has no such suffix. Purely lexical: a real filename ending in
/// `:digits` would parse this way too, which is why the caller only consults it on
/// a path that does not exist and only to word the error.
fn line_suffix(requested: &str) -> Option<(&str, usize, Option<usize>)> {
    let (base, suffix) = requested.rsplit_once(':')?;
    if base.is_empty() {
        return None;
    }
    match suffix.split_once('-') {
        Some((from, to)) => Some((base, from.parse().ok()?, Some(to.parse().ok()?))),
        None => Some((base, suffix.parse().ok()?, None)),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mcplease::traits::Tool as _;
    use std::fs;
    use std::path::Path;

    fn tools(root: &Path) -> FsTools {
        FsTools::embedded(root.to_path_buf()).unwrap()
    }

    fn call(
        state: &mut FsTools,
        path: &str,
        max_length: Option<usize>,
        from_line: Option<usize>,
        to_line: Option<usize>,
    ) -> Result<String> {
        Read {
            paths: vec![path.into()],
            max_length,
            from_line,
            to_line,
        }
        .execute(state, &RequestContext::default())
    }

    /// Ten lines, so a window is unambiguous.
    fn numbered(root: &Path) {
        let body: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        fs::write(root.join("f.txt"), body).unwrap();
    }

    /// `max_length` is a size cap on the region that was asked for, not a
    /// substitute for it. The two options were evaluated in the wrong order, so
    /// setting both silently discarded the line window and returned the head of
    /// the file — a caller asking for line 5 got line 1 with nothing saying so.
    #[test]
    fn max_length_caps_the_line_window_rather_than_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", Some(8), Some(5), None).unwrap();
        assert!(out.contains("line 5"), "should start at the window: {out}");
        assert!(!out.contains("line 1\n"), "must not fall back to the head: {out}");
    }

    /// The window alone is unaffected by the reorder.
    #[test]
    fn line_window_without_a_cap_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", None, Some(5), Some(6)).unwrap();
        assert!(out.contains("line 5") && out.contains("line 6"), "{out}");
        assert!(!out.contains("line 7"), "{out}");
    }

    /// A cap alone still truncates from the start of the file.
    #[test]
    fn cap_without_a_window_truncates_the_head() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", Some(6), None, None).unwrap();
        assert!(out.contains("TRUNCATED"), "{out}");
        assert!(out.contains("line 1"), "{out}");
        assert!(!out.contains("line 3"), "{out}");
    }

    /// A windowed read states its range in the sentinel, so a line number in the
    /// result and a line number in a later `edit` mean the same thing. It rides
    /// after `LENGTH:` because the path field is what the transcript parsers read.
    #[test]
    fn a_windowed_read_states_its_range_in_the_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", None, Some(5), Some(6)).unwrap();
        assert!(out.contains(", LINES: 5-6 "), "{out}");
        let begin = out.lines().next().unwrap();
        let path = begin.split(" BEGIN ").nth(1).unwrap().split(", LENGTH:").next().unwrap();
        assert!(path.ends_with("f.txt"), "path field unchanged: {path:?}");
    }

    /// A window starting past line 1 carries each line's file number, so a
    /// position in the result is a position in the file with no offset to add.
    /// The cap applies after numbering, so a truncated offset read stays numbered.
    #[test]
    fn an_offset_window_numbers_its_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", None, Some(5), Some(6)).unwrap();
        assert!(out.contains("\n5\tline 5\n6\tline 6\n"), "{out}");
        let capped = call(&mut tools(root), "f.txt", Some(8), Some(5), Some(6)).unwrap();
        assert!(capped.contains("TRUNCATED"), "{capped}");
        assert!(capped.contains("\n5\tline 5"), "{capped}");
    }

    /// A window starting at line 1 needs no arithmetic, so its lines stay bare.
    #[test]
    fn a_window_from_line_one_stays_bare() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", None, Some(1), Some(2)).unwrap();
        assert!(out.contains("\nline 1\nline 2\n"), "{out}");
        assert!(!out.contains("\t"), "{out}");
    }

    /// A whole-file read says nothing about lines.
    #[test]
    fn an_unwindowed_read_has_no_range_field() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", None, None, None).unwrap();
        assert!(!out.contains("LINES:"), "{out}");
    }

    /// `src/project.rs:578-700` is what a model writes when it wants a region and
    /// hasn't found `from_line`/`to_line`. The path is still wrong, so it still
    /// errors — but the error names the option that answers the request.
    #[test]
    fn a_line_range_suffix_is_answered_with_the_option_that_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt:5-6", None, None, None).unwrap();
        assert!(out.contains("BEGIN ERROR"), "still an error: {out}");
        assert!(out.contains("from_line: 5"), "{out}");
        assert!(out.contains("to_line: 6"), "{out}");
    }

    /// A path that simply doesn't exist keeps the plain error — no invented advice.
    #[test]
    fn an_ordinary_missing_path_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let out = call(&mut tools(root), "nope.txt", None, None, None).unwrap();
        assert!(out.contains("does not exist"), "{out}");
        assert!(!out.contains("from_line"), "{out}");
    }

    #[test]
    fn line_suffix_only_parses_a_numeric_tail() {
        assert_eq!(line_suffix("a/b.rs:5-9"), Some(("a/b.rs", 5, Some(9))));
        assert_eq!(line_suffix("a/b.rs:5"), Some(("a/b.rs", 5, None)));
        assert_eq!(line_suffix("a/b.rs"), None);
        assert_eq!(line_suffix("a/b:c.rs"), None);
        assert_eq!(line_suffix(":5"), None);
    }

    /// `max_length` is a byte count chosen by a caller who cannot see the
    /// encoding, so it lands mid-code-point routinely. Slicing a `str` there
    /// panics — every offset must instead back off to a char boundary.
    #[test]
    fn a_cap_inside_a_multibyte_character_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("u.txt"), "héllo wörld — ünicode\n").unwrap();
        for cap in 1..=24 {
            let out = call(&mut tools(root), "u.txt", Some(cap), None, None)
                .unwrap_or_else(|e| panic!("cap {cap} failed: {e}"));
            assert!(out.contains("TRUNCATED"), "cap {cap}: {out}");
        }
    }
}
