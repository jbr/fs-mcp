use super::*;

fn list(path: &str) -> List {
    List {
        path: Some(path.into()),
        recursive: None,
        include_metadata: None,
    }
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
