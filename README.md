# fs-mcp

A filesystem MCP (Model Context Protocol) server designed specifically for AI collaboration, with session support and respectful user experience design.

## Features

### Core Functionality
- **Session-aware operations**: Persistent context that survives MCP server restarts
- **Context management**: Set working directories per session for relative path operations
- **Gitignore awareness**: Respects `.gitignore` by default, with override options
- **Educational UX**: Provides helpful guidance about session management

### Design Principles
- **Respectful tooling**: No session amnesia - state persists across restarts
- **Low frustration**: Clear error messages and helpful guidance
- **AI-first design**: Interface designed specifically for AI collaboration patterns

## Usage

### Setting Context
```json
{
  "name": "set_context",
  "arguments": {
    "path": "/path/to/project",
    "session_id": "my_project_work"
  }
}
```

### Listing Directories
```json
{
  "name": "list_directory",
  "arguments": {
    "path": "./src",
    "session_id": "my_project_work",
    "include_gitignore": false
  }
}
```

## Session Management

Sessions provide isolated contexts for filesystem operations:
- Each session has its own working directory context
- Sessions persist to disk automatically (`~/.ai-tools/sessions/fs/`)
- Use meaningful session IDs for better organization
- Sessions survive MCP server restarts

## Architecture

### Session Store
Generic session storage system (`SessionStore<T>`) that:
- Handles persistence and serialization automatically
- Manages session lifecycle (creation, updates, cleanup)
- Provides thread-safe access to session data
- Could be extracted as a reusable component for other MCP tools

### File System Tools
- Context-aware path resolution
- Gitignore integration (basic implementation)
- Extensible for additional filesystem operations

## Development

```bash
# Build
cargo build

# Test
cargo test

# Run
cargo run
```

## Future Enhancements

### Planned Features
- **Glob patterns**: `src/**/*.rs` style directory listing
- **Advanced gitignore**: Full gitignore spec support
- **Content filtering**: `--text-only`, `--exclude-tests`, `--max-size` flags
- **File operations**: Read, write, move, delete with session context
- **Cross-tool sessions**: Shared session context across multiple MCP tools

### Session Store Extraction
The `SessionStore<T>` component is designed to be extracted into a separate crate for reuse across MCP tools, enabling consistent session management patterns.

## License

[Add your license here]
