//! Parsing the `path:range` suffix that `read` accepts on every path.
//!
//! The grammar exists because `src/project.rs:578-700` is what a model writes when
//! it wants a region: it is how compilers, `grep -n`, editors, and GitHub
//! permalinks all name a place in a file. The old spelling put one `from_line` /
//! `to_line` pair beside a *list* of paths, so the two could not be stated
//! together, and the suffix a model already knew was an error.
//!
//! Everything here is purely lexical. A real filename may contain `:`, so the
//! caller only consults this after finding that the literal path does not exist —
//! an existing file always wins over a range reading of its name.

/// One end of a requested range, 1-indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endpoint {
    /// Counted forward: `1` is the first line.
    FromStart(usize),
    /// Counted backward: `1` is the last line. This is what makes `path:-40` the
    /// last 40 lines — an unbounded range that starts 40 from the end.
    FromEnd(usize),
}

impl Endpoint {
    /// Resolve against a known line count. `FromStart` is returned as written,
    /// including past the end of the file, because a start past the end is worth
    /// an error rather than a silent clamp — the caller is looking at the wrong
    /// file or the wrong revision and should hear the line count. `FromEnd`
    /// clamps to 1: "the last 500 lines" of a 30-line file is the whole file,
    /// which is plainly what was meant.
    fn resolve(self, total: usize) -> usize {
        match self {
            Self::FromStart(n) => n,
            Self::FromEnd(n) => total.saturating_sub(n).saturating_add(1).max(1),
        }
    }
}

/// A line range as requested, before it meets a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineRange {
    pub(crate) start: Endpoint,
    /// `None` means "to the end of the file".
    pub(crate) end: Option<Endpoint>,
    /// Whether `:` separated the two numbers. `file.rs:120:5` is overwhelmingly
    /// likely to be a compiler's line:column, not lines 120 through 5, so the
    /// descending-range error says so.
    pub(crate) colon_separated: bool,
}

impl LineRange {
    /// Resolve to an inclusive, 1-indexed `(start, end)` pair against a file of
    /// `total` lines, or an error phrased for whoever asked.
    pub(crate) fn resolve(self, total: usize, path: &std::path::Path) -> anyhow::Result<(usize, usize)> {
        let start = self.start.resolve(total);
        if start > total {
            anyhow::bail!(
                "{} has {total} lines, so line {start} is past its end",
                path.display()
            );
        }
        let end = self.end.map_or(total, |e| e.resolve(total)).min(total);
        if start > end {
            let hint = if self.colon_separated {
                format!(
                    ". If {end} was a column, `{}:{start}` reads from line {start} to the end \
                     of the file",
                    path.display()
                )
            } else {
                String::new()
            };
            anyhow::bail!("requested lines {start}-{end}, which is empty{hint}");
        }
        Ok((start, end))
    }
}

/// Split `path:range` into its two halves, or `None` when the string names no
/// range.
///
/// Candidate separators are tried left to right so that the *whole* tail is
/// offered to the range parser first: `a:b.rs:10-20` tries `b.rs:10-20` (not a
/// range) before `10-20` (one), and `main.rs:10:20` tries `10:20` and takes it
/// rather than splitting it into `main.rs:10` and `20`. `C:\src\x.rs:10` works
/// for the same reason.
pub(crate) fn split_spec(requested: &str) -> Option<(&str, LineRange)> {
    requested
        .char_indices()
        .filter(|&(_, c)| c == ':' || c == '#')
        .filter(|&(i, _)| i > 0)
        .find_map(|(i, _)| Some((&requested[..i], parse_range(&requested[i + 1..])?)))
}

/// Parse the part after the `:`. Accepts, in order of how often a model writes
/// them: `120-160`, `120:160`, `120..160`, `120..=160`, `L120-L160`, `120`,
/// `-40`, `..40`, `120-`, `120..-10`.
pub(crate) fn parse_range(spec: &str) -> Option<LineRange> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }

    // `..` first: it is the one separator that can never be confused with a
    // minus sign, which is why it is the spelling that composes with negatives.
    if let Some((start, end)) = spec.split_once("..") {
        let end = end.strip_prefix('=').unwrap_or(end);
        return build(start, end, false);
    }

    if let Some((start, end)) = spec.split_once(':') {
        return build(start, end, true);
    }

    // A `-` at index 0 is a sign, not a separator, so the search for a separator
    // skips it. `-20--10` splits at the second dash into `-20` and `-10`.
    match spec[1..].find('-').map(|i| i + 1) {
        Some(i) => build(&spec[..i], &spec[i + 1..], false),
        None => Some(LineRange {
            start: endpoint(spec)?,
            end: None,
            colon_separated: false,
        }),
    }
}

fn build(start: &str, end: &str, colon_separated: bool) -> Option<LineRange> {
    let start = if start.is_empty() {
        Endpoint::FromStart(1)
    } else {
        endpoint(start)?
    };
    let end = if end.is_empty() {
        None
    } else {
        Some(endpoint(end)?)
    };
    Some(LineRange {
        start,
        end,
        colon_separated,
    })
}

/// `120`, `L120`, and `-120` — an optional GitHub-style `L`, an optional sign,
/// then a line number. Line 0 does not exist in any of the conventions this
/// grammar imitates, so it is rejected rather than silently meaning line 1.
fn endpoint(spec: &str) -> Option<Endpoint> {
    let (spec, from_end) = match spec.strip_prefix('-') {
        Some(rest) => (rest, true),
        None => (spec, false),
    };
    let digits = spec.strip_prefix(['L', 'l']).unwrap_or(spec);
    let n: usize = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    Some(if from_end {
        Endpoint::FromEnd(n)
    } else {
        Endpoint::FromStart(n)
    })
}

#[cfg(test)]
mod test {
    use super::Endpoint::*;
    use super::*;

    fn r(spec: &str) -> Option<(Endpoint, Option<Endpoint>)> {
        parse_range(spec).map(|r| (r.start, r.end))
    }

    #[test]
    fn the_spellings_a_model_reaches_for_all_mean_the_same_range() {
        for spec in ["120-160", "120:160", "120..160", "120..=160", "L120-L160", "L120-160"] {
            assert_eq!(r(spec), Some((FromStart(120), Some(FromStart(160)))), "{spec}");
        }
    }

    /// A lone number is the start of an unbounded range, so `path:120` reads from
    /// 120 to the end of the file and `path:-40` — 40 lines from the end, to the
    /// end — is the tail. One rule, two idioms.
    #[test]
    fn a_lone_endpoint_runs_to_the_end_of_the_file() {
        assert_eq!(r("120"), Some((FromStart(120), None)));
        assert_eq!(r("120-"), Some((FromStart(120), None)));
        assert_eq!(r("120.."), Some((FromStart(120), None)));
        assert_eq!(r("-40"), Some((FromEnd(40), None)));
    }

    #[test]
    fn an_open_start_is_the_head_of_the_file() {
        assert_eq!(r("..40"), Some((FromStart(1), Some(FromStart(40)))));
    }

    /// Negatives compose with either end, which is what lets a caller say "line
    /// 120 through the tenth-from-last" without knowing the file's length.
    #[test]
    fn negatives_compose_with_both_separators() {
        assert_eq!(r("120..-10"), Some((FromStart(120), Some(FromEnd(10)))));
        assert_eq!(r("120--10"), Some((FromStart(120), Some(FromEnd(10)))));
        assert_eq!(r("-20..-10"), Some((FromEnd(20), Some(FromEnd(10)))));
        assert_eq!(r("-20--10"), Some((FromEnd(20), Some(FromEnd(10)))));
    }

    #[test]
    fn a_non_numeric_tail_is_not_a_range() {
        assert!(r("").is_none());
        assert!(r("abc").is_none());
        assert!(r("10-abc").is_none());
        assert!(r("0").is_none(), "there is no line 0");
        assert!(r("10-0").is_none());
    }

    /// The whole tail is offered to the parser before any shorter one, so a
    /// `:` inside the filename and a `line:column` pair both land correctly.
    #[test]
    fn the_split_prefers_the_longest_parseable_tail() {
        assert_eq!(split_spec("a/b.rs:10-20").unwrap().0, "a/b.rs");
        assert_eq!(split_spec("a/b:c.rs:10-20").unwrap().0, "a/b:c.rs");
        assert_eq!(split_spec("main.rs:10:20").unwrap().0, "main.rs");
        assert_eq!(split_spec(r"C:\src\x.rs:10").unwrap().0, r"C:\src\x.rs");
        assert_eq!(split_spec("a/b.rs#L10-L20").unwrap().0, "a/b.rs");
        assert!(split_spec("a/b.rs").is_none());
        assert!(split_spec(":10").is_none(), "a range names no file on its own");
    }

    #[test]
    fn endpoints_resolve_against_the_files_length() {
        assert_eq!(FromStart(3).resolve(10), 3);
        assert_eq!(FromEnd(1).resolve(10), 10);
        assert_eq!(FromEnd(10).resolve(10), 1);
        assert_eq!(FromEnd(500).resolve(10), 1, "a long tail of a short file is the file");
    }
}
