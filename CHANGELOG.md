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
- `search`'s default `context_lines` is 3 rather than 1.
- `read` states a windowed read's line range in its sentinel, and a `path:120-160` request
  is answered by naming `from_line`/`to_line` rather than a bare "does not exist".

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
