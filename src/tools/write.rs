use crate::tools::FsTools;
use anyhow::{Context, Result, bail, anyhow};
use clap::ArgAction;
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
};
use serde::{Deserialize, Serialize};
use size::Size;
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write as _},
};

/// Write contents to a file, optionally creating any directories needed
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "write")]
pub struct Write {
    /// Path to write to
    /// Can be absolute, or relative to the server's root directory.
    pub path: String,

    /// The full contents of the file, expressed as a utf8 string, or in the case of `append`, the
    /// next chunk to write.
    pub contents: String,

    /// Overwrite file if it exists
    ///
    /// Only use if you have recently read the file and intend to replace it.
    /// This will fail if the file does not exist, and is mutually exclusive with `append`
    ///
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub overwrite: Option<bool>,

    /// Append to existing file
    ///
    /// This is mutually exclusive with `overwrite`. This will create the file if it does not yet
    /// exist.
    ///
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub append: Option<bool>,

    /// Create any directories leading up to this file if they don't already exist.
    ///
    /// This allows you to write files at arbitrary paths without needing a distinct interface or
    /// tool call for creating empty directories.
    ///
    /// Default: true
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long, action = ArgAction::SetTrue)]
    pub create_directories: Option<bool>,
}

impl ToolMeta for Write {
    fn title() -> Option<&'static str> {
        Some("Write file")
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(false),
            // `overwrite` replaces an existing file's contents outright.
            destructive_hint: Some(true),
            // Repeating an `append` write appends again.
            idempotent_hint: Some(false),
            open_world_hint: Some(false),
            ..Default::default()
        })
    }

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Creating a new file relative to the root directory",
                item: Self {
                    path: "src/main.rs".into(),
                    contents: "#[main]\nfn main() {\n  todo!()\n }".into(),
                    overwrite: None,
                    create_directories: None,
                    append: None,
                },
            },
            Example {
                description: "Intentionally overwriting a file",
                item: Self {
                    path: "/some/absolute/path/src/main.rs".into(),
                    contents: "#[main]\nfn main() {\n  todo!()\n }".into(),
                    overwrite: Some(true),
                    create_directories: Some(false),
                    append: None,
                },
            },
            Example {
                description: "Appending to a file",
                item: Self {
                    path: "/some/absolute/path/tests/tests.rs".into(),
                    contents: "\n\n#[test]\nfn another_test() {\n  assert!(true)\n }".into(),
                    overwrite: None,
                    create_directories: None,
                    append: Some(true),
                },
            },
        ]
    }
}

impl Write {
    fn overwrite(&self) -> bool {
        self.overwrite.unwrap_or_default()
    }

    fn append(&self) -> bool {
        self.append.unwrap_or_default()
    }

    fn create_directories(&self) -> bool {
        self.create_directories.unwrap_or(true)
    }

    /// Replace an existing file's contents in one observable step: write a
    /// sibling temp file, carry the original's permissions onto it, then rename
    /// over the target. `rename(2)` within a directory is atomic, so every
    /// reader sees old-or-new and never the empty or half-written middle.
    ///
    /// The temp file is a *sibling*, not in `$TMPDIR`: rename is only atomic
    /// within a filesystem, and a temp dir is routinely on a different one,
    /// where the call degrades to a copy and the atomicity is silently gone.
    ///
    /// Permissions are copied because rename replaces the directory entry — the
    /// new inode carries the temp file's mode (0600 from `NamedTempFile`), not
    /// the original's, so an executable script or a group-readable config would
    /// quietly lose its bits.
    fn replace_atomically(&self, path: &std::path::Path, tail_content: String) -> Result<String> {
        let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(dir).with_context(|| {
            format!(
                "Failed to create a temporary file beside {} for an atomic replace",
                path.display()
            )
        })?;
        temp.write_all(self.contents.as_bytes())
            .with_context(|| format!("Failed to write to {}", path.display()))?;
        // Durable before it is visible: the rename can otherwise be ordered
        // ahead of the data on a crash, leaving the target present but empty.
        temp.as_file()
            .sync_all()
            .with_context(|| format!("Failed to flush the replacement for {}", path.display()))?;
        if let Ok(metadata) = fs::metadata(path) {
            let _ = fs::set_permissions(temp.path(), metadata.permissions());
        }
        temp.persist(path)
            .map_err(|e| anyhow!("Failed to replace {}: {}", path.display(), e.error))?;

        let size = Size::from_bytes(
            fs::metadata(path)
                .with_context(|| format!("Failed to get metadata for {}", path.display()))?
                .len(),
        );
        let mut result = format!(
            "Successfully wrote {} bytes to {} (total: {size})",
            self.contents.len(),
            path.display()
        );
        if self.append() && (!tail_content.is_empty() || !self.contents.is_empty()) {
            result.push_str(&Self::format_seam_display(&tail_content, &self.contents, 3));
        }
        Ok(result)
    }

    fn read_file_tail(path: &std::path::Path, lines: usize) -> Result<String> {
        if !path.exists() {
            return Ok(String::new());
        }

        let content = fs::read_to_string(path)?;
        let file_lines: Vec<&str> = content.lines().collect();

        if file_lines.is_empty() {
            return Ok(String::new());
        }

        let start = file_lines.len().saturating_sub(lines);
        Ok(file_lines[start..].join("\n"))
    }

    fn format_seam_display(tail: &str, appended: &str, lines_to_show: usize) -> String {
        let mut result = String::new();

        if !tail.is_empty() {
            result.push_str(&format!(
                "\nContext around append point (last {lines_to_show} lines):\n"
            ));
            result.push_str(tail);
            result.push('\n');
        }

        result.push_str("\n<<< APPENDED >>>\n");

        // Show first few lines of appended content
        let appended_lines: Vec<&str> = appended.lines().collect();
        let lines_to_show = lines_to_show.min(appended_lines.len());

        if lines_to_show > 0 {
            result.push_str(&appended_lines[..lines_to_show].join("\n"));

            if appended_lines.len() > lines_to_show {
                result.push_str(&format!(
                    "\n... ({} more lines)",
                    appended_lines.len() - lines_to_show
                ));
            }
        }

        result
    }
}

impl Tool<FsTools> for Write {
    type Output = String;

    async fn execute(self, state: FsTools, _context: RequestContext) -> Result<String> {
        state.unblock(move |state| self.run(state)).await
    }
}

impl Write {
    pub(crate) fn run(mut self, state: &FsTools) -> Result<String> {
        // A token as `contents` stands in for the body of a write this server
        // refused earlier: "the text I sent when this was refused, and yes I
        // meant it". The path and flags come from *this* call; only the text
        // comes from state. The protocol is progressive reveal — it is
        // described nowhere except in a refusal, so it costs nothing in the
        // tool description until the scenario fires.
        if let Some(rejected) = state.take_rejected_write(&self.contents) {
            self.contents = rejected.contents;
        } else if crate::state::is_rejected_write_token(&self.contents) {
            bail!(
                "`{}` names no preserved write — it was already confirmed, or \
                 this server restarted since the refusal. Send the full contents.",
                self.contents
            );
        }

        let path = state.resolve_path(&self.path);
        if self.create_directories() {
            if let Some(parent_dir) = path.parent() {
                fs::create_dir_all(parent_dir).with_context(|| {
                    format!("Failed to create directories for {}", parent_dir.display())
                })?;
            }
        }

        if self.append() && self.overwrite() {
            // The body would otherwise be dropped and the model left to
            // regenerate it from memory — where drift sets in. Preserve it.
            let token =
                state.preserve_rejected_write(path.clone(), std::mem::take(&mut self.contents));
            // A refusal is returned as an error, not as a successful result carrying
            // refusal text: a caller that reads only the first line of a success
            // otherwise reads a write that did not happen as one that did.
            bail!(
                "`overwrite` and `append` are mutually exclusive. No filesystem operation has \
                 been performed.\n\n\
                 Your contents are preserved under `{token}`: repeat the call with only one of \
                 the two flags and \"contents\": \"{token}\" to write the same text."
            );
        }

        // For append operations, read the tail before writing for seam display
        let tail_content = if self.append() {
            Self::read_file_tail(&path, 3).unwrap_or_default()
        } else {
            String::new()
        };

        // Replacing an existing file goes through a sibling temp file and a
        // rename, so a concurrent reader sees the old bytes or the new bytes and
        // never a state that existed only mid-write. Opening the real path with
        // `truncate(true)` made the file transiently empty, then partial —
        // observable by anything holding it open, and not something a reader can
        // protect itself against.
        //
        // Only this branch. `append` must not be rewritten this way: appending
        // through a temp copy would mean reading the whole file first, turning an
        // O(added) operation into O(file) and inventing a lost-update race that
        // the append syscall does not have. And `create_new` is left alone
        // because its atomicity is load-bearing elsewhere — that single open *is*
        // the exclusive-existence check the already-exists refusal below depends
        // on, and a rename cannot make that check and the write one operation.
        if self.overwrite() && path.exists() {
            return self.replace_atomically(&path, tail_content);
        }

        let mut open_options = OpenOptions::new();
        if self.append() {
            open_options.create(true).append(true);
        } else if self.overwrite() {
            // `create(true)`, not just truncate: `overwrite` answers "there may
            // be a file there and I mean to replace it", which is true of a path
            // that turns out to be empty. Without it the call failed with ENOENT
            // on a path that did not exist yet — a refusal the caller cannot act
            // on, since the flag it would be told to drop is the one it already
            // has. (The existing-file case never reaches here; it is replaced
            // through the atomic rename path above.)
            open_options.write(true).create(true).truncate(true);
        } else {
            open_options.write(true).create_new(true);
        }

        {
            let mut file = match open_options.open(&path) {
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    // Same preserve-not-drop contract as the flag conflict
                    // above: the refusal is a confirmation question, and the
                    // answer should not cost the model a regeneration.
                    let bytes = self.contents.len();
                    let token = state
                        .preserve_rejected_write(path.clone(), std::mem::take(&mut self.contents));
                    // Errors, for the same reason as the flag conflict above: nothing
                    // was written, and the retry token is only actionable if the caller
                    // reads the body as a refusal.
                    bail!(
                        "File {} already exists, use \"overwrite\": true if you intend to replace it, \
                         or \"append\": true if you intend to add content to the end of the file.\n\n\
                         Your {bytes} bytes of contents are preserved under `{token}`: repeat the \
                         call with \"overwrite\": true and \"contents\": \"{token}\" to confirm this \
                         exact write.",
                        path.display()
                    );
                }

                Err(e) => bail!("Failed to open {} for writing: {e}", path.display()),
                Ok(file) => file,
            };

            file.write_all(self.contents.as_bytes())
                .with_context(|| format!("Failed to write to {}", path.display()))?;
        }

        let metadata = fs::metadata(&path)
            .with_context(|| format!("Failed to get metadata for {}", path.display()))?;
        let size = Size::from_bytes(metadata.len());

        let mut result = format!(
            "Successfully wrote {} bytes to {} (total: {size})",
            self.contents.len(),
            path.display()
        );

        // Add seam display for append operations
        if self.append() && (!tail_content.is_empty() || !self.contents.is_empty()) {
            result.push_str(&Self::format_seam_display(&tail_content, &self.contents, 3));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::Path;

    fn tools(root: &Path) -> FsTools {
        FsTools::new(root.to_path_buf())
    }

    fn call(
        state: &FsTools,
        path: &str,
        contents: &str,
        overwrite: Option<bool>,
        append: Option<bool>,
    ) -> Result<String> {
        Write {
            path: path.into(),
            contents: contents.into(),
            overwrite,
            append,
            create_directories: None,
        }
        .run(state)
    }

    /// An overwrite goes through a rename, so the file's contents change in one
    /// observable step. This pins the outcome; the concurrency property is
    /// below.
    #[test]
    fn overwrite_replaces_the_contents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("notes.txt"), "old contents\n").unwrap();
        let out = call(&tools(root), "notes.txt", "new\n", Some(true), None).unwrap();
        assert!(out.contains("Successfully"), "{out}");
        assert_eq!(fs::read_to_string(root.join("notes.txt")).unwrap(), "new\n");
    }

    /// `overwrite: true` against a path that does not exist yet **creates** it.
    /// The branch opened with `truncate` alone, so it failed with ENOENT — and
    /// the refusal was unactionable, since the flag a caller would be told to
    /// drop is the one it already had.
    #[test]
    fn overwrite_creates_a_file_that_does_not_exist_yet() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let out = call(&tools(root), "fresh.txt", "new\n", Some(true), None).unwrap();
        assert!(out.contains("Successfully"), "{out}");
        assert_eq!(fs::read_to_string(root.join("fresh.txt")).unwrap(), "new\n");
    }

    /// Rename replaces the directory entry, so the new inode carries the temp
    /// file's mode unless the original's is carried over. Without this an
    /// executable script silently stops being executable when it is written.
    #[cfg(unix)]
    #[test]
    fn an_overwrite_keeps_the_original_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("run.sh");
        fs::write(&path, "#!/bin/sh\necho old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

        call(
            &tools(root),
            "run.sh",
            "#!/bin/sh\necho new\n",
            Some(true),
            None,
        )
        .unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "the executable bit survived the replace");
    }

    /// The temp file is a sibling, so it must not survive as a stray entry.
    #[test]
    fn an_overwrite_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("notes.txt"), "old\n").unwrap();
        call(&tools(root), "notes.txt", "new\n", Some(true), None).unwrap();

        let entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["notes.txt".to_string()], "{entries:?}");
    }

    /// **A reader never observes a state that did not exist as a whole.** The
    /// truncate-in-place path made the file transiently empty and then partial;
    /// a reader racing it could see either.
    ///
    /// Sound in one direction on purpose: a reader that happens never to land
    /// inside the window still passes, so this cannot fail spuriously — but with
    /// `truncate(true)` restored it catches the empty read quickly.
    #[test]
    fn a_concurrent_reader_never_sees_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("notes.txt");
        let old = "old\n".repeat(4096);
        let new = "new\n".repeat(4096);
        fs::write(&path, &old).unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (path, old, new, stop) = (path.clone(), old.clone(), new.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut torn = None;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(seen) = fs::read_to_string(&path)
                        && seen != old
                        && seen != new
                    {
                        torn = Some(seen.len());
                        break;
                    }
                }
                torn
            })
        };

        let state = tools(root);
        for _ in 0..40 {
            call(&state, "notes.txt", &new, Some(true), None).unwrap();
            call(&state, "notes.txt", &old, Some(true), None).unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);

        assert_eq!(
            reader.join().unwrap(),
            None,
            "a reader observed a file state that never existed as a whole"
        );
    }

    /// `append` is deliberately *not* routed through the replace: rewriting it
    /// that way would mean reading the whole file to add to its end.
    #[test]
    fn append_still_adds_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("log.txt"), "one\n").unwrap();
        call(&tools(root), "log.txt", "two\n", None, Some(true)).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("log.txt")).unwrap(),
            "one\ntwo\n"
        );
    }

    /// The refusal that used to drop the body now returns a token, and the
    /// retry with `overwrite: true` plus that token writes the exact text —
    /// the regeneration-with-drift failure mode this closes.
    ///
    /// The refusal arrives as an `Err`, so a caller reading the outcome flag
    /// alone sees that no file was written; the body (token and all) is the
    /// error's message.
    #[test]
    fn rejected_write_is_retryable_by_token_without_regenerating() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("notes.txt"), "old\n").unwrap();

        let body = "line one\nline two\nline three\n";
        // one state across refusal and retry — the stash lives there
        let state = tools(root);
        let refusal = call(&state, "notes.txt", body, None, None)
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("already exists"), "{refusal}");
        assert!(
            !refusal.contains("Successfully"),
            "the write was refused: {refusal}"
        );
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "old\n",
            "the refused write left the file alone"
        );
        // the message names the token inline; pull it out of the message
        let token = refusal
            .split("preserved under `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .unwrap()
            .to_string();
        assert!(token.starts_with(crate::state::REJECTED_WRITE_PREFIX));

        let confirmed = call(&state, "notes.txt", &token, Some(true), None).unwrap();
        assert!(confirmed.contains("Successfully wrote"), "{confirmed}");
        assert_eq!(fs::read_to_string(root.join("notes.txt")).unwrap(), body);
    }

    /// The flag-conflict refusal preserves the body too — it dropped it the
    /// same way the overwrite refusal did.
    #[test]
    fn mutual_exclusion_refusal_preserves_the_body() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let body = "append me\n";
        // one state across refusal and retry — the stash lives there
        let state = tools(root);
        let refusal = call(&state, "new.txt", body, Some(true), Some(true))
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("mutually exclusive"), "{refusal}");
        assert!(
            refusal.contains("No filesystem operation has been performed"),
            "{refusal}"
        );
        let token = refusal
            .split("preserved under `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .unwrap()
            .to_string();

        let confirmed = call(&state, "new.txt", &token, Some(true), None).unwrap();
        assert!(confirmed.contains("Successfully"), "{confirmed}");
        assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), body);
    }

    /// A token-shaped `contents` naming nothing is an error that says what
    /// happened and how to recover — not a silent literal write of the token.
    #[test]
    fn expired_or_unknown_token_is_reported_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let err = call(
            &tools(root),
            "x.txt",
            "write-retry-0123456789abcdef",
            Some(true),
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("names no preserved write"),
            "{err}"
        );

        // single-use: after a confirmed retry, the same token names nothing
        fs::write(root.join("y.txt"), "old\n").unwrap();
        let state = tools(root);
        let refusal = call(&state, "y.txt", "body\n", None, None)
            .unwrap_err()
            .to_string();
        let token = refusal
            .split("preserved under `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
            .unwrap()
            .to_string();
        call(&state, "y.txt", &token, Some(true), None).unwrap();
        let err = call(&state, "y.txt", &token, Some(true), None).unwrap_err();
        assert!(err.to_string().contains("names no preserved write"));
    }

    /// Content that merely resembles a token (sentinel without a digest) is
    /// written literally — only exact-shaped strings are protocol.
    #[test]
    fn content_starting_with_the_sentinel_is_written_literally() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let out = call(
            &tools(root),
            "hashes.txt",
            "write-retry- is mentioned in these notes\n",
            None,
            None,
        )
        .unwrap();
        assert!(out.contains("Successfully"), "{out}");
        assert_eq!(
            fs::read_to_string(root.join("hashes.txt")).unwrap(),
            "write-retry- is mentioned in these notes\n"
        );
    }
}
