use crate::state::FsTools;

mcplease::tools!(
    FsTools,
    (Delete, delete, "delete"),
    (List, list, "list"),
    (Move, r#move, "move"),
    (SetContext, set_context, "set_context"),
    (Search, search, "search"),
    (Write, write, "write"),
    (Read, read, "read")
);
