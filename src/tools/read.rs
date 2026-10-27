use crate::tools::FsTools;
use anyhow::{Context, Result, anyhow};
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod range;
use range::{LineRange, split_spec};

/// Read utf8 contents from a file. Non-utf8 characters will be replaced lossily
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "read")]
pub struct Read {
    /// Paths to read. Absolute, or relative to the server's root directory.
    ///
    /// Each path may carry its own line range as a `:` suffix. Lines are
    /// 1-indexed and ranges are inclusive; a negative number counts back from
    /// the end of the file, so -1 is the last line.
    ///
    ///   src/main.rs:120-160     lines 120 through 160
    ///   src/main.rs:120:160     the same
    ///   src/main.rs:L120-L160   the same (`src/main.rs#L120-L160` also works)
    ///   src/main.rs:120..160    the same
    ///   src/main.rs:120         line 120 to the end of the file
    ///   src/main.rs:-40         the last 40 lines
    ///   src/main.rs:..40        the first 40 lines
    ///   src/main.rs:120..-10    line 120 through the tenth-from-last
    ///
    /// A path that exists on disk is read as itself and never parsed for a
    /// range, so a filename containing `:` still works.
    pub paths: Vec<String>,

    /// Max length in bytes to read from each path, applied after its line range.
    /// Will truncate the response and indicate truncation. The final character
    /// may be a replacement character if truncated mid code point
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub max_length: Option<usize>,
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
                description: "Reading a file relative to the root directory",
                item: Self {
                    paths: vec!["src/main.rs".into()],
                    max_length: None,
                },
            },
            Example {
                description: "Reading one region of a file",
                item: Self {
                    paths: vec!["src/tools/read.rs:167-177".into()],
                    max_length: None,
                },
            },
            Example {
                description: "Reading the last 50 lines of a log, and the head of another file",
                item: Self {
                    paths: vec!["logs/server.log:-50".into(), "README.md:..30".into()],
                    max_length: None,
                },
            },
            Example {
                description: "Reading several files at once, each with its own range",
                item: Self {
                    paths: vec![
                        "src/main.rs".into(),
                        "src/tools.rs:1-40".into(),
                        "src/tools/read.rs:120".into(),
                    ],
                    max_length: None,
                },
            },
            Example {
                description: "Capping the bytes returned for each path",
                item: Self {
                    paths: vec!["/some/absolute/path/src/main.rs".into()],
                    max_length: Some(100),
                },
            },
        ]
    }
}

impl Tool<FsTools> for Read {
    type Output = String;

    async fn execute(self, state: FsTools, _context: RequestContext) -> Result<String> {
        state.unblock(move |state| self.run(state)).await
    }
}

impl Read {
    pub(crate) fn run(self, state: &FsTools) -> Result<String> {
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

    /// Turn one requested string into the file it names and the range it asks for.
    ///
    /// Existence is checked before the string is parsed, so a file whose name
    /// really does end in `:10-20` is read as itself; only a path that resolves
    /// to nothing is reconsidered as `path:range`.
    fn locate(&self, state: &FsTools, requested: &str) -> Result<(PathBuf, Option<LineRange>)> {
        let path = state.resolve_path(requested);
        if path.exists() {
            return Ok((path, None));
        }

        if let Some((base, range)) = split_spec(requested) {
            let base_path = state.resolve_path(base);
            if base_path.exists() {
                return Ok((base_path, Some(range)));
            }
            // Report the file they meant, not the file-plus-range string, which
            // exists nowhere and would send a reader looking for the wrong name.
            return Err(anyhow!("{} does not exist", base_path.display()));
        }

        Err(anyhow!("{} does not exist", path.display()))
    }

    fn read_file(&self, state: &FsTools, requested: &str, separator: &str) -> Result<String> {
        let (path, range) = self.locate(state, requested)?;

        let full_contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Unable to read {}", path.display()))?;

        // The line window is applied *before* `max_length`, because `max_length` is a
        // size cap on what was asked for, not a substitute for it. Truncating first
        // silently discarded the range and returned the head of the file — a caller
        // asking for line 125 with a 20-byte cap got the first 20 bytes of line 1,
        // with nothing in the response to say so.
        let mut content_to_return = full_contents.clone();
        let mut window = None;

        if let Some(range) = range {
            let lines: Vec<&str> = full_contents.lines().collect();
            // No `with_context` here: anyhow shows only the outermost message, and
            // the range error is the one worth reading.
            let (start, end) = range.resolve(lines.len(), &path)?;

            // A window that starts past line 1 numbers each line with its file
            // line number. Without the prefix a reader has to add `start - 1` to
            // every position it reasons about, and a model re-targeting the region
            // was observed adding the offset once and then treating the relative
            // numbers as absolute (a hung turn and an extra search to reorient).
            // A window starting at line 1 needs no arithmetic, so it stays bare:
            // the numbers would only be bytes. Applied before `max_length` so a
            // truncated offset read is still numbered.
            content_to_return = if start > 1 {
                lines[start - 1..end]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}\t{line}", start + i))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                lines[start - 1..end].join("\n")
            };
            window = Some((start, end));
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

        // The window rides *after* `BYTES:` so the sentinel's path field is
        // unchanged: efference's transcript parsers take the path as everything
        // between ` BEGIN ` and `, LENGTH:` or `, BYTES:` (docs/tui.md), and adding a field there
        // would silently change what they read a path as. Stating the range —
        // resolved to absolute line numbers, whichever end it was counted from —
        // is what makes a windowed read re-targetable: a line number in the result
        // and a line number in a later `write` mean the same thing.
        let lines = window
            .map(|(from, to)| format!(", LINES: {from}-{to}"))
            .unwrap_or_default();
        // `BYTES` names what the number counts (the file's size, not its lines). The
        // newline before `END` is added only when the content does not end in one,
        // so a file with a trailing newline does not read as having an empty last line.
        let close = if content_to_return.ends_with('\n') { "" } else { "\n" };
        Ok(format!(
            "=={separator} BEGIN {path}, BYTES: {len}{lines} {separator}==\n\
            {content_to_return}{close}=={separator} END {path}, BYTES: {len}{lines} {separator}==\n",
            path = path.display(),
            len = full_contents.len(),
        ))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn tools(root: &Path) -> FsTools {
        FsTools::new(root.to_path_buf())
    }

    fn call(state: &FsTools, path: &str, max_length: Option<usize>) -> String {
        Read {
            paths: vec![path.into()],
            max_length,
        }
        .run(state)
        .unwrap()
    }

    /// Ten lines, so a window is unambiguous.
    fn numbered(root: &Path) {
        let body: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        fs::write(root.join("f.txt"), body).unwrap();
    }

    /// The suffix a model already writes now *is* the interface, and every
    /// spelling of it reaches the same lines.
    #[test]
    fn every_spelling_of_a_range_reads_the_same_region() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let state = &tools(root);
        for spec in [
            "f.txt:5-6",
            "f.txt:5:6",
            "f.txt:L5-L6",
            "f.txt:5..6",
            "f.txt:5..=6",
            "f.txt#L5-L6",
        ] {
            let out = call(state, spec, None);
            assert!(out.contains(", LINES: 5-6 "), "{spec}: {out}");
            assert!(out.contains("line 5") && out.contains("line 6"), "{spec}: {out}");
            assert!(!out.contains("line 7"), "{spec}: {out}");
        }
    }

    /// The tail idiom, which is the whole reason endpoints count from the end.
    /// It resolves to absolute line numbers in the sentinel, so what came back
    /// can be re-targeted without knowing the file's length.
    #[test]
    fn a_negative_start_reads_the_end_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:-3", None);
        assert!(out.contains(", LINES: 8-10 "), "{out}");
        assert!(out.contains("8\tline 8"), "{out}");
        assert!(!out.contains("line 7"), "{out}");
    }

    /// "The last 500 lines" of a ten-line file is the ten-line file; a tail is
    /// clamped rather than refused, because nothing else could have been meant.
    #[test]
    fn a_tail_longer_than_the_file_is_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:-500", None);
        assert!(out.contains(", LINES: 1-10 "), "{out}");
        assert!(out.contains("line 1") && out.contains("line 10"), "{out}");
    }

    #[test]
    fn an_open_ended_range_runs_to_the_end_and_an_open_start_from_the_beginning() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let state = &tools(root);
        let tail = call(state, "f.txt:8", None);
        assert!(tail.contains(", LINES: 8-10 "), "{tail}");
        let head = call(state, "f.txt:..2", None);
        assert!(head.contains(", LINES: 1-2 "), "{head}");
        assert!(!head.contains("line 3"), "{head}");
    }

    /// Each path carries its own range — the pairing the old single
    /// `from_line`/`to_line` beside a list of paths could not express.
    #[test]
    fn each_path_in_one_call_gets_its_own_range() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        fs::write(root.join("g.txt"), "a\nb\nc\n").unwrap();
        let out = Read {
            paths: vec!["f.txt:5-6".into(), "g.txt:-1".into(), "f.txt".into()],
            max_length: None,
        }
        .run(&tools(root))
        .unwrap();
        assert!(out.contains(", LINES: 5-6 "), "{out}");
        assert!(out.contains(", LINES: 3-3 "), "{out}");
        assert!(out.contains("3\tc"), "{out}");
        // The unranged third path is a whole-file read in the same response.
        assert!(out.contains("\nline 1\n"), "{out}");
    }

    /// A file whose name really ends in `:5-6` is read as itself: existence is
    /// checked before the string is parsed, so the grammar can never shadow a
    /// real path.
    #[test]
    fn an_existing_path_beats_a_range_reading_of_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        fs::write(root.join("f.txt:5-6"), "the literal file\n").unwrap();
        let out = call(&tools(root), "f.txt:5-6", None);
        assert!(out.contains("the literal file"), "{out}");
        assert!(!out.contains("LINES:"), "{out}");
    }

    /// `src/main.rs:120:5` from a compiler is a line and a *column*. It parses as
    /// a descending range, which is empty — so the error names the reading that
    /// answers what was actually wanted.
    #[test]
    fn a_line_column_reference_is_answered_with_the_range_that_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:8:3", None);
        assert!(out.contains("BEGIN ERROR"), "{out}");
        assert!(out.contains("If 3 was a column"), "{out}");
        assert!(out.contains("f.txt:8"), "{out}");
    }

    /// A start past the end of the file is an error naming the length, not a
    /// silent clamp: the caller is looking at a stale revision and needs to know.
    #[test]
    fn a_start_past_the_end_of_the_file_names_the_length() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:50-60", None);
        assert!(out.contains("BEGIN ERROR"), "{out}");
        assert!(out.contains("has 10 lines"), "{out}");
    }

    /// An end past the file clamps — asking for more than is there is not wrong.
    #[test]
    fn an_end_past_the_file_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:9-60", None);
        assert!(out.contains(", LINES: 9-10 "), "{out}");
    }

    /// A missing path reports the file that was meant, not the path-plus-range
    /// string, which names nothing on disk.
    #[test]
    fn a_missing_path_reports_the_file_without_its_range() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let out = call(&tools(root), "nope.txt:5-6", None);
        assert!(out.contains("does not exist"), "{out}");
        assert!(!out.contains(":5-6 does not exist"), "{out}");
    }

    /// `max_length` is a size cap on the region that was asked for, not a
    /// substitute for it. The two were once evaluated in the wrong order, so
    /// setting both silently discarded the line window and returned the head of
    /// the file — a caller asking for line 5 got line 1 with nothing saying so.
    #[test]
    fn max_length_caps_the_line_window_rather_than_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:5", Some(8));
        assert!(out.contains("line 5"), "should start at the window: {out}");
        assert!(!out.contains("line 1\n"), "must not fall back to the head: {out}");
        assert!(out.contains("TRUNCATED"), "{out}");
    }

    /// A cap alone still truncates from the start of the file.
    #[test]
    fn cap_without_a_window_truncates_the_head() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt", Some(6));
        assert!(out.contains("TRUNCATED"), "{out}");
        assert!(out.contains("line 1"), "{out}");
        assert!(!out.contains("line 3"), "{out}");
    }

    /// A windowed read states its range in the sentinel. It rides after
    /// `BYTES:` because the path field is what the transcript parsers read.
    #[test]
    fn a_windowed_read_states_its_range_in_the_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:5-6", None);
        let begin = out.lines().next().unwrap();
        let path = begin.split(" BEGIN ").nth(1).unwrap().split(", BYTES:").next().unwrap();
        assert!(path.ends_with("f.txt"), "path field unchanged: {path:?}");
    }

    /// The frame closes on a fresh line without adding a blank one: a file that
    /// ends in a newline is not followed by an extra empty line, one that does
    /// not still gets its `END` on its own line.
    #[test]
    fn the_frame_adds_a_closing_newline_only_when_the_content_lacks_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("with.txt"), "red\ngreen\n").unwrap();
        fs::write(root.join("without.txt"), "red\ngreen").unwrap();
        let with = call(&tools(root), "with.txt", None);
        let body = with.split_once("==\n").unwrap().1;
        assert!(body.starts_with("red\ngreen\n=="), "{with}");
        assert!(with.contains(", BYTES: 10 "), "{with}");
        let without = call(&tools(root), "without.txt", None);
        let body = without.split_once("==\n").unwrap().1;
        assert!(body.starts_with("red\ngreen\n=="), "{without}");
        assert!(without.contains(", BYTES: 9 "), "{without}");
    }

    /// A window starting past line 1 carries each line's file number, so a
    /// position in the result is a position in the file with no offset to add.
    /// The cap applies after numbering, so a truncated offset read stays numbered.
    #[test]
    fn an_offset_window_numbers_its_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:5-6", None);
        assert!(out.contains("\n5\tline 5\n6\tline 6\n"), "{out}");
        let capped = call(&tools(root), "f.txt:5-6", Some(8));
        assert!(capped.contains("TRUNCATED"), "{capped}");
        assert!(capped.contains("\n5\tline 5"), "{capped}");
    }

    /// A window starting at line 1 needs no arithmetic, so its lines stay bare.
    #[test]
    fn a_window_from_line_one_stays_bare() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt:1-2", None);
        assert!(out.contains("\nline 1\nline 2\n"), "{out}");
        assert!(!out.contains("\t"), "{out}");
    }

    /// A whole-file read says nothing about lines.
    #[test]
    fn an_unwindowed_read_has_no_range_field() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&tools(root), "f.txt", None);
        assert!(!out.contains("LINES:"), "{out}");
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
            let out = call(&tools(root), "u.txt", Some(cap));
            assert!(out.contains("TRUNCATED"), "cap {cap}: {out}");
        }
    }
}
