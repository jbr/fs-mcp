use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::tools::FsTools;

#[derive(Serialize, Deserialize, Debug)]
pub struct SetContext {
    path: String,
    session_id: Option<String>,
}
impl SetContext {
    pub(crate) fn execute(&self, state: Arc<FsTools>) -> Result<String, anyhow::Error> {
        match &self.session_id {
            Some(session_id) => {
                let mut session_data = state.session_store.get_or_create(session_id)?;
                session_data.context_path = Some(self.path.clone().into());
                state.session_store.set(session_id, session_data)?;

                Ok(format!(
                    "Set context to {} for session '{}'",
                    self.path,
                    session_id
                ))
            }

            None => Ok(
                "No session found. Provide a session id to isolate your work from other conversations.".into()
            ),
        }
    }
}
