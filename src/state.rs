use std::{
    collections::{HashMap, VecDeque},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use anyhow::Result;

/// How many rejected-write bodies to hold at once. One is usually enough (the
/// retry normally follows the refusal directly), but a burst of rejections
/// before any retry should not silently drop earlier bodies the model may
/// still confirm.
const REJECTED_WRITE_LIMIT: usize = 8;

/// A rejected write's preserved body. The retry names it by token and supplies
/// its own path and flags; only the text comes from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RejectedWrite {
    pub(crate) path: PathBuf,
    pub(crate) contents: String,
}

/// The filesystem tools' shared state, cloned into every call.
///
/// There is no working directory to set. MCP has no sessions, so nothing a
/// call does may change how the next one resolves: a path is absolute, or it
/// is relative to the root this server was started with — `--root`, else the
/// process's working directory, or whatever an embedding host passes to
/// [`FsTools::new`].
#[derive(Debug, Clone)]
pub struct FsTools {
    root: Arc<Path>,
    /// Bodies of writes this server refused, keyed by the retry token the
    /// refusal returned (`write.rs`). In-memory only: a restarted server has
    /// no preserved bodies, and the expired-token message says so.
    rejected_writes: Arc<Mutex<RejectedWrites>>,
}

#[derive(Debug, Default)]
struct RejectedWrites {
    bodies: HashMap<String, RejectedWrite>,
    /// Insertion order over `bodies`, so eviction at [`REJECTED_WRITE_LIMIT`]
    /// drops the oldest rejection deterministically.
    order: VecDeque<String>,
}

/// The sentinel every retry token opens with — recognizable on sight, and
/// shaped so no plausible file body is one.
pub(crate) const REJECTED_WRITE_PREFIX: &str = "write-retry-";

/// The deterministic stand-in for one preserved body: a digest of the text it
/// stands for, sentinel-prefixed so ordinary file content can never be mistaken
/// for one. In-memory state means tokens die with the process anyway, so the
/// hasher's cross-process stability is irrelevant.
fn rejected_write_token(contents: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    format!("{REJECTED_WRITE_PREFIX}{:016x}", hasher.finish())
}

/// Whether `contents` is shaped exactly like a [`rejected_write_token`] output.
/// Only a token-shaped string that fails lookup is reported as expired or
/// unknown; content that merely resembles a token is written literally.
pub(crate) fn is_rejected_write_token(contents: &str) -> bool {
    contents
        .strip_prefix(REJECTED_WRITE_PREFIX)
        .is_some_and(|digest| digest.len() == 16 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl FsTools {
    /// Preserve the body of a refused write under its retry token, evicting the
    /// oldest preserved body past [`REJECTED_WRITE_LIMIT`]. Re-preserving an
    /// identical body (the model re-sends the same write and is refused again)
    /// keeps its place rather than adding a second entry.
    pub(crate) fn preserve_rejected_write(&self, path: PathBuf, contents: String) -> String {
        let token = rejected_write_token(&contents);
        let mut rejected = self.rejected_writes();
        if rejected
            .bodies
            .insert(token.clone(), RejectedWrite { path, contents })
            .is_none()
        {
            rejected.order.push_back(token.clone());
        }
        while rejected.bodies.len() > REJECTED_WRITE_LIMIT {
            let Some(oldest) = rejected.order.pop_front() else {
                break;
            };
            rejected.bodies.remove(&oldest);
        }
        token
    }

    /// Consume the preserved body named by `token`, if it is still held — the
    /// token is single-use, and a second use reports it as expired.
    pub(crate) fn take_rejected_write(&self, token: &str) -> Option<RejectedWrite> {
        let mut rejected = self.rejected_writes();
        let entry = rejected.bodies.remove(token)?;
        rejected.order.retain(|t| t != token);
        Some(entry)
    }

    fn rejected_writes(&self) -> MutexGuard<'_, RejectedWrites> {
        self.rejected_writes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}

impl FsTools {
    /// Tools that resolve relative paths against `root`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: normalize_path(&root).into(),
            rejected_writes: Arc::default(),
        }
    }

    /// Resolve a path as a call gave it: `~` expanded, then taken as-is if
    /// absolute and against the root otherwise.
    pub(crate) fn resolve_path(&self, path_str: &str) -> PathBuf {
        let path = PathBuf::from(&*shellexpand::tilde(path_str));
        normalize_path(&self.root.join(path))
    }

    /// Run a tool body on tokio's blocking pool. Every tool here is
    /// synchronous filesystem work, which must not stall the executor that
    /// other calls share.
    pub(crate) async fn unblock<T: Send + 'static>(
        self,
        body: impl FnOnce(&Self) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        tokio::task::spawn_blocking(move || body(&self)).await?
    }
}
