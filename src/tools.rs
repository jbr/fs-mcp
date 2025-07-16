use crate::state::FsTools;

mcplease::tools!(
    FsTools,
    (Delete, delete, "delete"),
    (List, list, "list"),
    (Move, r#move, "move"),
    (
        SetWorkingDirectory,
        set_working_directory,
        "set_working_directory"
    ),
    (Search, search, "search"),
    (Write, write, "write"),
    (Read, read, "read")
);
