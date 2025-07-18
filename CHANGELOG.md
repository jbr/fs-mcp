# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/jbr/fs-mcp/releases/tag/v0.1.0) - 2025-07-18

### Added

- add cli interface
- sessions are reloaded when needed
- add context_lines to search
- simplify search implementation
- use mcplease
- search
- add seam display for append
- add append mode to write
- recursive listings always hide gitignored files
- add read tool
- [**breaking**] major overhaul
- Combine path and glob
- add glob pattern filtering to list_directory
- implement proper gitignore support with ignore crate
- initial filesystem MCP server with session support

### Fixed

- tests for mcplease switch
- overwrite mode
- log invocation

### Other

- add manifest keys
- add project scaffolding
- update readme
- *(deps)* upgrade mcplease
- remove tokio, use &mut State
- cleanup, use type system
