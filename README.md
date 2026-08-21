# Ankhimate MCP server

`ankhimate-mcp` lets an MCP client create, inspect, edit, save, and export an
Ankhimate rig without opening the editor. It communicates over stdio.

```bash
cargo run -p ankhimate-mcp
```

Configure an MCP client to launch the built binary with no arguments. The server
keeps one rig open for the life of that process and advertises seven tools:

- `open_rig`, `new_rig`, `describe_rig`
- `list_verbs`, `run_script`
- `save_rig`, `export_rig`

`run_script` is the editing surface: its sandbox exposes the same `ops.invoke`,
`rig()`, and `names()` API documented in `docs/plugin-api.md`. It has no
filesystem, network, or clock access.

The server never saves over the file it opened. Name a new destination with
`save_rig`. Export paths are confined to the chosen output directory; the whole
export is planned before writing and existing orphan files are reported, never
deleted.
