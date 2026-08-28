use super::*;

use crate::tools::FsTools;
use mcplease::traits::Tool as _;
use mcplease::types::RequestContext;
use std::fs;

fn list(path: &str) -> List {
    List {
        path: Some(path.into()),
        recursive: None,
        include_metadata: None,
        summary: None,
    }
}

fn run(root: &std::path::Path, mut list: List) -> String {
    let mut state = FsTools::embedded(root.to_path_buf()).unwrap();
    list.recursive.get_or_insert(true);
    list.execute(&mut state, &RequestContext::default()).unwrap()
}

/// A leading brace group used to be read as a literal directory name, so
/// `{assets,docs}/**/*.png` answered "Path is not a directory: …/{assets,docs}".
#[test]
fn a_brace_group_is_pattern_not_directory() {
    let l = list("{assets,docs}/**/*.{png,jpg}");
    assert_eq!(
        l.parse_path_and_pattern().unwrap(),
        ("", Some("{assets,docs}/**/*.{png,jpg}"))
    );
}

#[test]
fn the_base_is_everything_before_the_first_pattern_character() {
    assert_eq!(
        list("src/**/*.rs").parse_path_and_pattern().unwrap(),
        ("src", Some("**/*.rs"))
    );
    assert_eq!(
        list("a/b/{c,d}/*.rs").parse_path_and_pattern().unwrap(),
        ("a/b", Some("{c,d}/*.rs"))
    );
    assert_eq!(
        list("docs").parse_path_and_pattern().unwrap(),
        ("docs", None)
    );
}

/// `glob::Pattern` compiled `{a,b}` without complaint and then matched nothing,
/// which is why the matcher had to change rather than just the split.
#[test]
fn brace_alternation_matches() {
    let g = compile_glob("{assets,docs}/**/*.{png,jpg}").unwrap();
    assert!(g.is_match("assets/x.png"));
    assert!(g.is_match("docs/deep/y.jpg"));
    assert!(!g.is_match("other/x.png"));
}

/// Patterns are matched against the path relative to the base — the form the
/// caller wrote — and `**/` matches no directories as readily as many.
#[test]
fn patterns_match_relative_paths_at_any_depth() {
    let g = compile_glob("**/*.rs").unwrap();
    assert!(g.is_match("lib.rs"));
    assert!(g.is_match("tui/projection/nodes.rs"));
}

/// The counterpart: a single `*` does not cross a separator, so `*.rs` stays
/// the files here. Losing this would make every pattern recursive.
#[test]
fn a_single_star_does_not_cross_a_separator() {
    let g = compile_glob("*.rs").unwrap();
    assert!(g.is_match("lib.rs"));
    assert!(!g.is_match("tui/mod.rs"));
}

#[test]
fn a_malformed_glob_names_itself_in_the_error() {
    let err = compile_glob("**/*.{rs").unwrap_err().to_string();
    assert!(err.contains("**/*.{rs"), "{err}");
}

/// Every entry carries the request's own path prefix, so it can be handed to the
/// next call unchanged. The prior format stated the base once in a header and
/// printed bare entries; a model dropped the base and joined an entry to the repo
/// root instead, costing two failed reads and a reconcile round.
#[test]
fn entries_are_prefixed_with_the_path_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("assets/themes")).unwrap();
    fs::write(root.join("assets/themes/banner.png"), "x").unwrap();

    let out = run(root, list("assets/**/*"));
    assert!(
        out.contains("assets/themes/banner.png"),
        "entry keeps the request's prefix: {out}"
    );
    assert!(
        !out.lines().any(|line| line.trim() == "themes/banner.png"),
        "no bare entry that only resolves against a header: {out}"
    );
}

/// A non-glob directory request prefixes the same way.
#[test]
fn a_plain_directory_request_prefixes_too() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/a.md"), "# A\n").unwrap();

    let out = run(root, list("docs"));
    assert!(out.contains("docs/a.md"), "{out}");
}

/// A request that named no path keeps bare relative entries: those already resolve
/// against the session working directory, which is what the next call resolves
/// against too.
#[test]
fn a_pathless_request_leaves_entries_bare() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.md"), "# A\n").unwrap();

    let out = run(
        root,
        List {
            path: None,
            recursive: None,
            include_metadata: None,
            summary: None,
        },
    );
    assert!(out.lines().any(|line| line.trim() == "a.md"), "{out}");
}

/// An absolute request answers absolute entries — same rule, and the entry is
/// still valid to paste.
#[test]
fn an_absolute_request_answers_absolute_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.md"), "# A\n").unwrap();

    let out = run(root, list(&root.display().to_string()));
    assert!(
        out.contains(&root.join("a.md").display().to_string()),
        "{out}"
    );
}

#[test]
fn the_prefix_is_the_directory_part_as_written() {
    assert_eq!(entry_prefix(""), "");
    assert_eq!(entry_prefix("."), "");
    assert_eq!(entry_prefix("assets"), "assets/");
    assert_eq!(entry_prefix("a/b/"), "a/b/");
}

/// Summary mode makes a directory of documents self-describing: the frontmatter
/// description outranks a derived one, then the first heading, then prose.
#[test]
fn summary_prefers_frontmatter_then_heading_then_prose() {
    assert_eq!(
        summarize_markdown("---\ntitle: x\ndescription: What this unit is\n---\n\n# Heading\n")
            .as_deref(),
        Some("What this unit is")
    );
    assert_eq!(
        summarize_markdown("\n# Reduce formatter-only edit output\n\nbody\n").as_deref(),
        Some("Reduce formatter-only edit output")
    );
    assert_eq!(
        summarize_markdown("just prose, no heading\n").as_deref(),
        Some("just prose, no heading")
    );
    assert_eq!(summarize_markdown("\n\n   \n").as_deref(), None);
}

/// A file opening with a fenced block summarizes as its prose, not as the fence.
#[test]
fn summary_skips_a_leading_code_fence() {
    assert_eq!(
        summarize_markdown("```rust\nfn main() {}\n```\n\nWhat it does\n").as_deref(),
        Some("What it does")
    );
}

#[test]
fn summary_is_bounded() {
    let long = "x".repeat(500);
    let summary = summarize_markdown(&format!("# {long}\n")).unwrap();
    assert!(summary.len() <= MAX_SUMMARY + 4, "{}", summary.len());
    assert!(summary.ends_with('…'));
}

/// End to end: only markdown files gain a summary, and directories never do.
#[test]
fn summary_mode_annotates_markdown_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("unit.md"), "# Reduce formatter output\n\nbody\n").unwrap();
    fs::write(root.join("code.rs"), "// # not a heading\n").unwrap();

    let mut listing = list(".");
    listing.summary = Some(true);
    let out = run(root, listing);
    assert!(out.contains("unit.md  — Reduce formatter output"), "{out}");
    assert!(
        out.lines().any(|line| line.trim() == "code.rs"),
        "non-markdown is untouched: {out}"
    );
}
