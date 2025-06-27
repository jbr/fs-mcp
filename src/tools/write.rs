use crate::{tools::FsTools, traits::WithExamples, types::Example};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use size::Size;
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write as _},
};

/// Write contents to a file, optionally creating any directories needed
///
/// Usage recommendation: If the content you're going to write is more than several hundred lines,
/// it is best to write it with several successive `"append": true` calls because sometimes Claude
/// Desktop interrupts long responses, and there is nothing the MCP server can do to prevent this.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename = "write")]
pub struct Write {
    /// Path to write to
    /// Can be absolute, or relative to session context path.
    pub path: String,

    /// Optional session identifier for context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

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
    pub overwrite: Option<bool>,

    /// Append to existing file
    ///
    /// This is mutually exclusive with `overwrite`. This will create the file if it does not yet
    /// exist.
    ///
    /// Default: false
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append: Option<bool>,

    /// Create any directories leading up to this file if they don't already exist.
    ///
    /// This allows you to write files at arbitrary paths without needing a distinct interface or
    /// tool call for creating empty directories.
    ///
    /// Default: true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_directories: Option<bool>,
}

impl WithExamples for Write {
    fn examples() -> Option<Vec<Example<Self>>> {
        Some(vec![
            Example {
                description: "Creating a new file relative to a session",
                item: Self {
                    path: "src/main.rs".into(),
                    session_id: Some("some_rust_session_unique_id".into()),
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
                    session_id: None,
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
                    session_id: None,
                    contents: "\n\n#[test]\nfn another_test() {\n  assert!(true)\n }".into(),
                    overwrite: None,
                    create_directories: None,
                    append: Some(true),
                },
            },
        ])
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

    pub fn execute(self, state: &mut FsTools) -> Result<String> {
        let path = state.resolve_path(&self.path, self.session_id.as_deref())?;
        if self.create_directories() {
            if let Some(parent_dir) = path.parent() {
                fs::create_dir_all(parent_dir).with_context(|| {
                    format!("trying to create directories for {}", parent_dir.display())
                })?;
            }
        }

        if self.append() && self.overwrite() {
            return Ok("`overwrite` and `append` are mutually exclusive. No filesystem operation has been performed".into());
        }

        let mut open_options = OpenOptions::new();
        if self.append() {
            open_options.create(true).append(true);
        } else if self.overwrite() {
            open_options.truncate(true).create(true);
        } else {
            open_options.write(true).create_new(true);
        }

        {
            let mut file = match open_options.open(&path) {
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    return Ok(format!(
                        "File {} already exists, use \"overwrite\": true if you intend to replace it, \
                         or \"append\": true if you intend to add content to the end of the file.",
                        path.display()
                    ));
                }

                Err(e) => return Err(anyhow!(e)),
                Ok(file) => file,
            };

            file.write_all(self.contents.as_bytes())?;
        }

        let metadata = fs::metadata(&path)?;
        let size = Size::from_bytes(metadata.len());

        Ok(format!(
            "Successfully wrote {} bytes to {} (total: {size})",
            self.contents.len(),
            path.display()
        ))
    }
}
