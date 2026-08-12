//! Library surface for **in-process embedding** (e.g. the efference harness).
//!
//! The binary (`main.rs`) remains the standalone MCP stdio server. This lib
//! exposes the same tool logic — the generated [`tools::Tools`] dispatcher and the
//! [`state::FsTools`] state — so a host can run a tool directly with
//! `serde_json::from_value::<Tools>(json!({"name","arguments"}))` +
//! `Tool::execute(&mut state)`, bypassing `mcplease::run`'s stdio/clap/logger
//! ownership.
//!
//! Additive only: `main.rs` and `tests.rs` keep declaring their own `mod state;`
//! / `mod tools;`, so the binary and its tests are unchanged. Deleting this file
//! fully reverts the crate to bin-only.

#![allow(clippy::collapsible_if)]

pub mod state;
pub mod tools;

pub use state::FsTools;
pub use tools::Tools;

// `FsTools::embedded(cwd)` (in `state.rs`) is the in-process entry point: an
// in-memory session store with the working directory pre-seeded, so embedding
// never touches the shared `~/.ai-tools` store.
