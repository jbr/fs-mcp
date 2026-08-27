# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- `search` groups matches under one path header per file instead of repeating the absolute path on every printed line (matches and context alike). 27–47% fewer bytes on real searches. The header is absolute so an entry stays valid to paste into a following call; a non-contiguous block within one file keeps its `--` marker.

### Fixed

- `write` is atomic when replacing an existing file: contents go to a sibling temp file which is then renamed over the target, so a concurrent reader observes the old bytes or the new bytes and never the transiently empty or partial middle that `truncate(true)` exposed. The original's permissions are carried onto the replacement, since rename swaps the inode. `append` and exclusive-create keep their existing paths — routing append through a copy would make it O(file) and invent a lost-update race, and the exclusive-create open *is* the existence check the already-exists refusal relies on.
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
