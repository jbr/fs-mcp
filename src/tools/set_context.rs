use crate::{tools::FsTools, traits::WithExamples, types::Example};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Set the working context path for a session
#[derive(Serialize, Deserialize, Debug, schemars::JsonSchema)]
#[serde(rename = "set_context")]
pub struct SetContext {
    /// Directory path to set as context
    path: String,
    /// Session identifier
    /// Can be absolutely anything, as long as it's unlikely to collide with another session, (ie not "claude")
    session_id: String,
}

impl WithExamples for SetContext {
    fn examples() -> Option<Vec<Example<Self>>> {
        Some(vec![Example {
            description: "setting context to a development project",
            item: Self {
                path: "/usr/local/projects/cobol".into(),
                session_id: "GraceHopper1906".into(),
            },
        }])
    }
}

impl SetContext {
    pub(crate) fn execute(self, state: FsTools) -> Result<String> {
        let Self { path, session_id } = self;
        let mut session_data = state.session_store.get_or_create(&session_id)?;
        let response = format!("Set context to {path} for session '{session_id}'");
        session_data.context_path = Some(path.into());
        state.session_store.set(&session_id, session_data)?;
        Ok(response)
    }
}
