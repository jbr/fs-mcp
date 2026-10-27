# fs-mcp

A filesystem MCP (Model Context Protocol)

## Tools
```
  delete                 Remove a file from disk
  list                   List file system contents with globbing
  move                   Move a file from one location to another
  search                 Search for text patterns in files using ripgrep-like functionality
  write                  Write contents to a file, optionally creating any directories needed
  read                   Read utf8 contents from a file. Non-utf8 characters will be replaced lossily
  help                   Print this message or the help of the given subcommand(s)
```

## Installation

```bash
$ cargo install fs-mcp
```

## Usage with Claude Desktop

Add this to your Claude Desktop MCP configuration:

```json
{
  "mcpServers": {
    "fs-mcp": {
      "command": "/path/to/fs-mcp/fs-mcp",
      "args": ["serve", "--root", "/path/to/project"]
    }
  }
}
```

Paths are absolute, or relative to the root: `--root` if given, otherwise the directory the server
was started in.


## License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

---

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
