# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- `list` prefixes every entry with the request's path as written, so an entry can be
  passed straight back to another call instead of resolving against a header stated once
  (`assets/**/*` now answers `assets/themes/…`).
- `list --summary` annotates markdown files with their frontmatter `description`, first
  heading, or first line of prose.
- **Breaking:** `search` no longer takes `highlight_style`, and the five variants
  (`box`, `emphasis`, `ansi`, `markdown`, `none`) are gone. Wrapping a match in `┌─…─┐`
  edited the line so it no longer matched the bytes on disk — search output is what gets
  pasted into a `write` — and split the matched text into tokens that no longer looked
  like the pattern that was searched for, so the marker made the match *less* findable to
  its reader, not more. It also cost ~5% of the response and ran a second, independent
  regex engine over every result line, disagreeing with the matcher that found the match.
  Dropped the direct `regex` dependency with it. The one thing it genuinely answered —
  where in a 900-character line — is still answered by the `…` windowing.
- `search` marks a matched line with grep's `:` gutter and a context line with `-`
  (`  30: fn resolve(…)` against `  31-     match self {`). One punctuation character
  either way, outside the content rather than inside it.
- `search` returns lines verbatim apart from trailing whitespace; it no longer trims
  leading indentation. Indentation is ~1 token a line, it is semantic in Python and YAML,
  and keeping it means a result pastes into a `write` unchanged and agrees with what
  `read` returns. `MAX_CONTEXT_LINE` is now measured against the content, so a deeply
  nested line is not clipped early for being nested.
- **Breaking:** `search` takes `paths: string[]` instead of a single `path`. Each entry
  may be a file, a directory, or a glob (`src/**/*.rs`), mixed freely; a file reachable
  from two entries is searched once. Only 19 of 5,389 observed searches relied on an
  implicit working directory and a quarter named more than one operand, so the plural is
  the common case.
- **Breaking:** `search`'s `include_extensions` is replaced by `include_globs` and
  `exclude_globs`, matched against both the file name and the full path. A bare extension
  list cannot say `**/test_*.py` or negate; `--include='*.rs'` is the dominant real idiom
  (365 uses against 45 of rg's `--type` names).
- **Breaking:** `search`'s `context_lines` is replaced by `context_before` (default 2) and
  `context_after` (default 6). Real usage is forward-leaning and asymmetric — `-A` 931
  invocations against `-B` 214, and the symmetric `-C` this tool used to offer as its only
  shape exactly zero. The large `-A` values in that data are deliberately not chased: they
  are a workaround for "show me the body of the thing I found", which is a code-semantic
  question for a tree-sitter or LSP-backed tool rather than a line matcher.
- `search`'s `pattern` accepts a list as well as a string, OR-ing the alternatives. Half of
  real searches are alternations, and each alternative is wrapped in a non-capturing group
  before joining, so `["a b", "c"]` cannot become `a b|c` — a precedence nobody intends.
- `search` gains `line_numbers_only`: one line per file giving the match count and the
  matching line numbers, with consecutive runs collapsed into ranges
  (`src/tools/read.rs (3 matches): 12, 88-90`). Subsumes grep's `-l` and `-c` in one mode —
  the paths, the counts, and the clustering neither of those can show — and the ranges are
  written in `read`'s own `path:from-to` grammar, so the answer is the argument to the next
  call. In this mode `max_results` caps *files*, and a file with many scattered matches
  lists its first 10 ranges and then `… +N more`, so a broad scout degrades into a path
  and a count rather than hundreds of numbers.
- `search`'s default `max_results` is 20 rather than 50. Three quarters of real searches
  were piped into `head -10`/`-20`; those are line budgets rather than match budgets so
  they don't transfer directly, but they say plainly that the old ceiling returned more
  than anyone read. The truthful total makes the lower cap safe.
- `search` reports how many files it searched and where when it finds nothing, and names
  any requested path that did not exist instead of folding a typo into "no matches".
- `search` defaults to smart case: a pattern containing an uppercase letter is
  case-sensitive, one without is not. Passing `case_sensitive` explicitly still wins.
  (`\S`, `\W` and other escapes don't count as uppercase.)

- **Breaking:** `read` takes its line range as a suffix on each path rather than as a
  separate `from_line`/`to_line` pair, which are gone. `paths` is a list but the pair was
  singular, so a range and a list of paths could not be stated together, and the suffix a
  model already writes — `src/project.rs:578-700` — was an error. Now each path carries its
  own range: `path:120-160`, `path:120:160`, `path:L120-L160`, `path#L120-L160`,
  `path:120..160` and `path:120..=160` all name lines 120 through 160. A lone endpoint runs
  to the end of the file (`path:120`), and endpoints may count from the end, so `path:-40`
  is the last 40 lines, `path:..40` the first 40, and `path:120..-10` line 120 through the
  tenth-from-last. A path that exists on disk is read as itself and never parsed for a
  range, so a filename containing `:` still works. A start past the end of the file errors
  with the file's length; an end past it clamps; `path:120:5` — a compiler's line and
  column — is an empty range, and the error names `path:120` as the read that answers it.
- `read` states a windowed read's line range in its sentinel, resolved to absolute line
  numbers whichever end it was counted from.

### Fixed

- `search` merges overlapping and adjacent context windows instead of printing each
  match's window independently. Two matches three lines apart repeated the lines between
  them, under the same line numbers, at two places in the output — 27 printed lines where
  21 were meant. Relatedly, the `--` gap marker now marks an actual gap; it used to print
  between every pair of matches in a file whether or not their windows were contiguous.
- `search` reports how many matches exist when it truncates. The scan stopped at exactly
  `max_results` and then compared that same counter against the limit, so the "... and N
  more matches" line could never fire: fifty results read as all of them. The header now
  says `Found 312 matches …, showing the first 50`, counting up to a ceiling of 1000
  beyond which it reports `1000+`.
- `search --case-sensitive` on the CLI: clap's `SetTrue` on an `Option<bool>` produced
  `Some(false)` when the flag was absent rather than `None`, which reads as an explicit
  request for insensitivity and would have defeated smart case on every CLI search. The
  MCP path, which goes through serde, was unaffected.
### Added

- A `write` this server refuses now preserves the body it was given and returns a retry token; sending that token back as `contents`, paired with the flag the refusal asked for, writes the same text. Covers both refusals that used to drop a body — the already-exists check and the `append`/`overwrite` conflict. The token is a digest of the contents, sentinel-prefixed (`write-retry-`) so ordinary file text can never be mistaken for one and an identical re-send names the same body; it is single-use, in-memory, and holds at most 8 bodies. Nothing advertises it except a refusal, so it costs nothing in the tool description until the case fires.

### Changed

- `search` groups matches under one path header per file instead of repeating the absolute path on every printed line (matches and context alike). 27–47% fewer bytes on real searches. The header is absolute so an entry stays valid to paste into a following call; a non-contiguous block within one file keeps its `--` marker.

### Fixed

- `write` is atomic when replacing an existing file: contents go to a sibling temp file which is then renamed over the target, so a concurrent reader observes the old bytes or the new bytes and never the transiently empty or partial middle that `truncate(true)` exposed. The original's permissions are carried onto the replacement, since rename swaps the inode. `append` and exclusive-create keep their existing paths — routing append through a copy would make it O(file) and invent a lost-update race, and the exclusive-create open *is* the existence check the already-exists refusal relies on.
- `write` with `overwrite: true` creates a file that does not exist yet instead of failing with ENOENT. The branch opened with `truncate` and no `create`, so the one refusal a caller could not act on was the one where the flag it would be told to drop is the flag it already had.
- `list` accepts brace alternation (`{assets,docs}/**/*.{png,jpg}`): `{` now splits base from pattern, and the matcher is `globset` rather than `glob`, which had no brace support and matched nothing.
- `list` no longer drops nested matches. Glob filtering pruned the walk, but a directory does not match a pattern naming files, so `src/**/*.rs` returned only the files directly under `src` — 28 of 246 in one repository — as a listing that looked complete. The pattern is applied per entry instead, against the path relative to the base.

## [0.1.2](https://github.com/jbr/fs-mcp/compare/v0.1.1...v0.1.2) - 2025-07-25

### Fixed

- normalize paths instead of canonicalizing them
- hide hidden files

## [0.1.1](https://github.com/jbr/fs-mcp/compare/v0.1.0...v0.1.1) - 2025-07-25

### Added

- canonicalize resolved paths

### Fixed

- cargo.toml url copypasta
- gitignore was not working

### Other

- remove outdated schema.json and ignore .envrc
- use trusted publishers workflow
- Merge pull request #5 from jbr/dependabot/cargo/mcplease-0.2.3
