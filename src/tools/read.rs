use crate::tools::FsTools;
use anyhow::{Context, Result, anyhow};
use mcplease::{
    traits::{Tool, ToolMeta},
    types::{Example, RequestContext, ToolAnnotations},
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

impl ToolMeta for Read {
    fn title() -> Option<&'static str> {
        Some("Read files")
    }

    fn annotations() -> Option<ToolAnnotations> {
        Some(ToolAnnotations {
            read_only_hint: Some(true),
            open_world_hint: Some(false),
            ..Default::default()
        })
    }

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
    type Output = String;

    fn execute(self, state: &mut FsTools, _context: &RequestContext) -> Result<String> {
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
        // Walk back to a char boundary: slicing a `str` mid-code-point panics, and
        // `max_length` is a byte count the caller chose without knowing the encoding.
        let mut cut = max_length.min(full_contents.len());
        while cut > 0 && !full_contents.is_char_boundary(cut) {
            cut -= 1;
        }
        let truncated = &full_contents[..cut];
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

        // The line window is applied *before* `max_length`, because `max_length` is a
        // size cap on what was asked for, not a substitute for it. Truncating first
        // silently discarded `from_line`/`to_line` and returned the head of the file —
        // a caller asking for line 125 with a 20-byte cap got the first 20 bytes of
        // line 1, with nothing in the response to say so.
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

        let selected_length = content_to_return.len();
        if let Some(max_length) = self.max_length {
            if max_length < selected_length {
                return self.read_head_from_string(
                    &path,
                    &content_to_return,
                    max_length,
                    selected_length,
                    separator,
                );
            }
        }

        Ok(format!(
            "=={separator} BEGIN {path}, LENGTH: {len} {separator}==\n\
            {content_to_return}\n=={separator} END {path}, LENGTH: {len} {separator}==\n",
            path = path.display(),
            len = full_contents.len(),
        ))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mcplease::traits::Tool as _;
    use std::fs;
    use std::path::Path;

    fn tools(root: &Path) -> FsTools {
        FsTools::embedded(root.to_path_buf()).unwrap()
    }

    fn call(
        state: &mut FsTools,
        path: &str,
        max_length: Option<usize>,
        from_line: Option<usize>,
        to_line: Option<usize>,
    ) -> Result<String> {
        Read {
            paths: vec![path.into()],
            max_length,
            from_line,
            to_line,
        }
        .execute(state, &RequestContext::default())
    }

    /// Ten lines, so a window is unambiguous.
    fn numbered(root: &Path) {
        let body: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        fs::write(root.join("f.txt"), body).unwrap();
    }

    /// `max_length` is a size cap on the region that was asked for, not a
    /// substitute for it. The two options were evaluated in the wrong order, so
    /// setting both silently discarded the line window and returned the head of
    /// the file — a caller asking for line 5 got line 1 with nothing saying so.
    #[test]
    fn max_length_caps_the_line_window_rather_than_replacing_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", Some(6), Some(5), None).unwrap();
        assert!(out.contains("line 5"), "should start at the window: {out}");
        assert!(!out.contains("line 1\n"), "must not fall back to the head: {out}");
    }

    /// The window alone is unaffected by the reorder.
    #[test]
    fn line_window_without_a_cap_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", None, Some(5), Some(6)).unwrap();
        assert!(out.contains("line 5") && out.contains("line 6"), "{out}");
        assert!(!out.contains("line 7"), "{out}");
    }

    /// A cap alone still truncates from the start of the file.
    #[test]
    fn cap_without_a_window_truncates_the_head() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        numbered(root);
        let out = call(&mut tools(root), "f.txt", Some(6), None, None).unwrap();
        assert!(out.contains("TRUNCATED"), "{out}");
        assert!(out.contains("line 1"), "{out}");
        assert!(!out.contains("line 3"), "{out}");
    }

    /// `max_length` is a byte count chosen by a caller who cannot see the
    /// encoding, so it lands mid-code-point routinely. Slicing a `str` there
    /// panics — every offset must instead back off to a char boundary.
    #[test]
    fn a_cap_inside_a_multibyte_character_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("u.txt"), "héllo wörld — ünicode\n").unwrap();
        for cap in 1..=24 {
            let out = call(&mut tools(root), "u.txt", Some(cap), None, None)
                .unwrap_or_else(|e| panic!("cap {cap} failed: {e}"));
            assert!(out.contains("TRUNCATED"), "cap {cap}: {out}");
        }
    }
}
