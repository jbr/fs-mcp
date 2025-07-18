use crate::tools::FsTools;
use anyhow::Result;
use mcplease::{
    traits::{Tool, WithExamples},
    types::Example,
};
use serde::{Deserialize, Serialize};

/// Set the working context path
#[derive(Serialize, Deserialize, Debug, schemars::JsonSchema, clap::Args)]
#[serde(rename = "set_working_directory")]
pub struct SetWorkingDirectory {
    /// New working directory. All relative paths will be relative to this path
    path: String,
}

impl WithExamples for SetWorkingDirectory {
    fn examples() -> Vec<Example<Self>> {
        vec![Example {
            description: "setting context to a development project",
            item: Self {
                path: "/usr/local/projects/cobol".into(),
            },
        }]
    }
}

impl Tool<FsTools> for SetWorkingDirectory {
    fn execute(self, state: &mut FsTools) -> Result<String> {
        let new_context_path = state.resolve_path(&self.path, None)?;
        let response = format!("Set context to {}", new_context_path.display());
        state.set_working_directory(new_context_path, None)?;
        Ok(response)
    }
}
