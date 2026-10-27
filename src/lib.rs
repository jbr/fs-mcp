//! Library surface for **in-process embedding** (e.g. the efference harness).
//!
//! The binary (`main.rs`) remains the standalone MCP stdio server. This lib
//! exposes the same tool logic — the generated [`tools::Tools`] dispatcher and the
//! [`state::FsTools`] state — so a host can run a tool directly with
//! `serde_json::from_value::<Tools>(json!({"name","arguments"}))` +
//! `Dispatch::call(state, context)`, without the binary's stdio, clap, or
//! logger. Tool bodies run on tokio's blocking pool, so the host must be on a
//! tokio runtime.
//!
//! Additive only: `main.rs` and `tests.rs` keep declaring their own `mod state;`
//! / `mod tools;`, so the binary and its tests are unchanged. Deleting this file
//! fully reverts the crate to bin-only.

#![allow(clippy::collapsible_if)]

pub mod state;
pub mod tools;

pub use state::FsTools;
pub use tools::Tools;

// `FsTools::new(root)` (in `state.rs`) is the in-process entry point. A host
// running several agents gives each its own, rooted at that agent's directory.
