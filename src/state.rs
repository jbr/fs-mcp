use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use mcplease::session::SessionStore;

/// How many rejected-write bodies to hold at once. One is usually enough (the
/// retry normally follows the refusal directly), but a burst of rejections
/// before any retry should not silently drop earlier bodies the model may
/// still confirm.
const REJECTED_WRITE_LIMIT: usize = 8;

/// Shared context data that can be used across multiple MCP servers
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SharedContextData {
    /// Current working context path
    pub context_path: Option<PathBuf>,
}

/// A rejected write's preserved body. The retry names it by token and supplies
/// its own path and flags; only the text comes from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RejectedWrite {
    pub(crate) path: PathBuf,
    pub(crate) contents: String,
}

/// Filesystem tools with session support
#[derive(Debug, fieldwork::Fieldwork)]
pub struct FsTools {
    /// Shared context store for cross-server communication
    #[fieldwork(get, get_mut)]
    shared_context_store: SessionStore<SharedContextData>,
    /// Bodies of writes this server refused, keyed by the retry token the
    /// refusal returned (`write.rs`). In-memory only: a restarted server has
    /// no preserved bodies, and the expired-token message says so.
    rejected_writes: HashMap<String, RejectedWrite>,
    /// Insertion order over `rejected_writes`, so eviction at
    /// [`REJECTED_WRITE_LIMIT`] drops the oldest rejection deterministically.
    rejected_write_order: VecDeque<String>,
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
    /// is a no-op that returns the same token.
    pub(crate) fn preserve_rejected_write(&mut self, path: PathBuf, contents: String) -> String {
        let token = rejected_write_token(&contents);
        if self
            .rejected_writes
            .insert(token.clone(), RejectedWrite { path, contents })
            .is_none()
        {
            self.rejected_write_order.push_back(token.clone());
        }
        while self.rejected_writes.len() > REJECTED_WRITE_LIMIT {
            let Some(oldest) = self.rejected_write_order.pop_front() else {
                break;
            };
            self.rejected_writes.remove(&oldest);
        }
        token
    }

    /// Consume the preserved body named by `token`, if it is still held — the
    /// token is single-use, and a second use reports it as expired.
    pub(crate) fn take_rejected_write(&mut self, token: &str) -> Option<RejectedWrite> {
        let entry = self.rejected_writes.remove(token)?;
        self.rejected_write_order.retain(|t| t != token);
        Some(entry)
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
    /// Create a new FsTools instance
    pub fn new() -> Result<Self> {
        // Use shared context store for cross-server communication
        let mut shared_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        shared_path.push(".ai-tools");
        shared_path.push("sessions");
        shared_path.push("shared-context.json");

        let shared_context_store = SessionStore::new(Some(shared_path))?;

        Ok(Self {
            shared_context_store,
            rejected_writes: HashMap::new(),
            rejected_write_order: VecDeque::new(),
        })
    }

    /// Construct with an **in-memory** session store (no disk, no cross-process
    /// sharing) and the working directory pre-seeded — for in-process embedding
    /// (e.g. the efference harness). Unlike [`new`](Self::new), this does not
    /// touch the shared `~/.ai-tools` store, so embedding never clobbers other
    /// mcplease servers' session state.
    #[allow(
        dead_code,
        reason = "used by library consumers (embedding), not the bin"
    )]
    pub fn embedded(working_directory: PathBuf) -> Result<Self> {
        let mut tools = Self {
            shared_context_store: SessionStore::new(None)?,
            rejected_writes: HashMap::new(),
            rejected_write_order: VecDeque::new(),
        };
        tools.set_working_directory(working_directory, None)?;
        Ok(tools)
    }

    fn default_session_id(&self) -> &'static str {
        "default"
    }

    /// Resolve a path relative to session context if needed
    pub(crate) fn resolve_path(
        &mut self,
        path_str: &str,
        session_id: Option<&str>,
    ) -> Result<PathBuf> {
        let path = PathBuf::from(&*shellexpand::tilde(path_str));

        if path.is_absolute() {
            return Ok(normalize_path(&path));
        }

        let session_id = session_id.unwrap_or_else(|| self.default_session_id());

        match self.get_context(Some(session_id))? {
            Some(context) => Ok(normalize_path(&context.join(path_str))),
            None => Err(anyhow!(
                "Use set_working_directory first or provide an absolute path.",
            )),
        }
    }

    /// Get context for a session
    pub fn get_context(&mut self, session_id: Option<&str>) -> Result<Option<PathBuf>> {
        let session_data = self
            .shared_context_store
            .get_or_create(session_id.unwrap_or_else(|| self.default_session_id()))?;
        Ok(session_data.context_path.clone())
    }

    pub(crate) fn set_working_directory(
        &mut self,
        path: PathBuf,
        session_id: Option<&str>,
    ) -> Result<()> {
        self.shared_context_store.update(
            session_id.unwrap_or_else(|| self.default_session_id()),
            |shared_data| {
                shared_data.context_path = Some(path);
            },
        )
    }
}
