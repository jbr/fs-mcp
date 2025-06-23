use crate::{state::FsTools, traits::AsToolSchema, types::ToolSchema};
use anyhow::Result;
use serde::{Deserialize, Serialize};

mod delete;
mod list;
mod r#move;
mod read;
mod set_context;
mod write;

macro_rules! tools {
    ($state:tt, $(($capitalized:tt, $lowercase:tt, $string:literal)),+) => {
        $(pub use $lowercase::$capitalized;)+

        #[derive(Debug, Serialize, Deserialize)]
        #[serde(tag = "name")]
        pub enum Tools {
            $(#[serde(rename = $string)] $capitalized { arguments: $capitalized },)+
        }

        impl Tools {
            pub fn execute(self, state: &mut $state) -> Result<String> {
                match self {
                    $(Tools::$capitalized { arguments} => arguments.execute(state),)+
                }
            }

            pub fn schema() -> Vec<ToolSchema> {
                vec![$($capitalized::as_tool_schema(),)+]
            }
        }
    };
}

tools!(
    FsTools,
    (Delete, delete, "delete"),
    (List, list, "list"),
    (Move, r#move, "move"),
    (SetContext, set_context, "set_context"),
    (Write, write, "write"),
    (Read, read, "read")
);
