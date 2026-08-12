use crate::tools::FsTools;
use anyhow::{Context, Result, anyhow};
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};
use std::{io::Read as _, path::Path};

/// Read utf8 contents from a file. Non-utf8 characters will be replaced lossily
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema, clap::Args)]
#[serde(rename = "read")]
pub struct Read {
    /// Path or paths to read
    /// Can be absolute, or relative to session context path.
    pub paths: Vec<String>,

    /// Max length in bytes to read. Will truncate response and indicate truncation.
    /// Final character may be a replacement character if truncated mid code point
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub max_length: Option<usize>,

    /// Start line (1-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub from_line: Option<usize>,

    /// End line (1-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[arg(long)]
    pub to_line: Option<usize>,
}

impl WithExamples for Read {
    fn examples() -> Vec<Example<Self>> {
        vec![
            Example {
                description: "Reading a file relative to a session context",
                item: Self {
                    paths: vec!["src/main.rs".into()],
                    max_length: None,
                    from_line: None,
                    to_line: None,
                },
            },
            Example {
                description: "Reading the head of a file by absolute path",
                item: Self {
                    paths: vec!["/some/absolute/path/src/main.rs".into()],
                    max_length: Some(100),
                    from_line: None,
                    to_line: None,
                },
            },
            Example {
                description: "Reading several files at once",
                item: Self {
                    paths: vec![
                        "src/main.rs".into(),
                        "src/tools.rs".into(),
                        "src/tools/read.rs".into(),
                    ],
                    max_length: None,
                    from_line: None,
                    to_line: None,
                },
            },
        ]
    }
}

impl Tool<FsTools> for Read {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let separator = std::iter::repeat_with(fastrand::alphanumeric)
            .take(10)
            .collect::<String>();
        Ok(self
            .paths
            .iter()
            .map(|path| {
                self.read_file(state, path, &separator).unwrap_or_else(|e| {
                    format!(
                        "=={separator} BEGIN ERROR {path} {separator}==\n\
                        {e}\n=={separator} END ERROR {path} {separator}=="
                    )
                })
            })
            .collect())
    }
}

impl Read {
    #[allow(dead_code)]
    fn read_head(
        &self,
        path: &Path,
        max_length: usize,
        actual_length: usize,
        separator: &str,
    ) -> Result<String> {
        let mut bytes = vec![0u8; max_length];
        std::fs::File::open(path)
            .with_context(|| format!("Unable to open {}", path.display()))?
            .read_exact(&mut bytes)
            .with_context(|| format!("Unable to read from {}", path.display()))?;
        Ok(format!(
            "=={separator} BEGIN TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n\
            {content}\n\
            =={separator} END TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n",
            path = path.display(),
            content = String::from_utf8_lossy(&bytes)
        ))
    }

    fn read_head_from_string(
        &self,
        path: &Path,
        full_contents: &str,
        max_length: usize,
        actual_length: usize,
        separator: &str,
    ) -> Result<String> {
        let truncated = &full_contents[..max_length];
        Ok(format!(
            "=={separator} BEGIN TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n\
            {content}\n\
            =={separator} END TRUNCATED {path}, FULL LENGTH: {actual_length}, TRUNCATED LENGTH: {max_length} {separator}==\n",
            path = path.display(),
            content = truncated
        ))
    }

    fn read_file(&self, state: &mut FsTools, path: &str, separator: &str) -> Result<String> {
        let path = state.resolve_path(path, None)?;

        if !path.exists() {
            return Err(anyhow!("{} does not exist", path.display()));
        }

        let full_contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Unable to read {}", path.display()))?;

        if let Some(max_length) = self.max_length {
            let actual_length = full_contents.len();
            if max_length < actual_length {
                return self.read_head_from_string(
                    &path,
                    &full_contents,
                    max_length,
                    actual_length,
                    separator,
                );
            }
        }

        let mut content_to_return = full_contents.clone();

        if let Some(from_line) = self.from_line {
            let lines: Vec<&str> = full_contents.lines().collect();
            let start = from_line.saturating_sub(1);
            if start >= lines.len() {
                return Err(anyhow!(
                    "from_line {} is beyond end of file ({} lines)",
                    from_line,
                    lines.len()
                ));
            }

            let end = self
                .to_line
                .unwrap_or(lines.len())
                .saturating_sub(1)
                .min(lines.len() - 1);

            if start > end {
                return Err(anyhow!(
                    "from_line {} must be <= to_line {}",
                    from_line,
                    self.to_line.unwrap_or(lines.len())
                ));
            }

            content_to_return = lines[start..=end].join("\n");
            // If the original file didn't end with a newline, joining might add one or lose context.
            // But for a tool result, this is generally acceptable.
        }

        Ok(format!(
            "=={separator} BEGIN {path}, LENGTH: {len} {separator}==\n\
            {content_to_return}\n=={separator} END {path}, LENGTH: {len} {separator}==\n",
            path = path.display(),
            len = full_contents.len(),
        ))
    }
}
