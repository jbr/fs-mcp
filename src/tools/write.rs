use crate::tools::FsTools;
use anyhow::{Context, Result, bail};
use clap::ArgAction;
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};
use size::Size;
use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write as _},
};

/// Write contents to a file, optionally creating any directories needed
///
/// Usage recommendation: For very large files, you may want to use multiple append operations
/// if you encounter interruption issues.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "write")]
pub struct Write {
    /// Path to write to
    /// Can be absolute, or relative to session context path.
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

impl WithExamples for Write {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Creating a new file relative to a session",
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
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let path = state
            .resolve_path(&self.path, None)
            .with_context(|| format!("Failed to resolve {}", self.path))?;
        if self.create_directories() {
            if let Some(parent_dir) = path.parent() {
                fs::create_dir_all(parent_dir).with_context(|| {
                    format!("Failed to create directories for {}", parent_dir.display())
                })?;
            }
        }

        if self.append() && self.overwrite() {
            return Ok("`overwrite` and `append` are mutually exclusive. No filesystem operation has been performed".into());
        }

        // For append operations, read the tail before writing for seam display
        let tail_content = if self.append() {
            Self::read_file_tail(&path, 3).unwrap_or_default()
        } else {
            String::new()
        };

        let mut open_options = OpenOptions::new();
        if self.append() {
            open_options.create(true).append(true);
        } else if self.overwrite() {
            open_options.write(true).truncate(true);
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
