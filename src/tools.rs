use crate::state::FsTools;

mcplease::tools!(
    FsTools,
    (Delete, delete, "delete"),
    (List, list, "list"),
    (Move, r#move, "move"),
    (Search, search, "search"),
    (Write, write, "write"),
    (Read, read, "read")
);
