---
title: Use the MCP server
description: Configure a client and safely inspect, edit, render, save, and export one rig per process.
---

# MCP server

Build or launch the stdio server with:

```console
cargo run -p ankhimate-mcp
```

Configure a generic MCP client with that command, the repository as working
directory, and stdio transport. The normal initialize/initialized handshake is
required. One process owns one current rig; `open_rig` or `new_rig` replaces it.
Installed plugin discovery is shared with the editor, so plugin importers and
exporters can appear without MCP-specific code.

The server exposes nine coarse tools. See their exact generated schemas in the
[MCP tool reference](/editor/reference/mcp-tools/). Structured tools return JSON.
`open_rig` combines a structured inventory with PNG preview content; rendering
tools return `image/png` content (and structured metadata where applicable).

`save_rig` refuses to overwrite the file opened as the source because this
headless session has no safe in-place recovery UI. Export retains path confinement,
plan-first writes, and never-delete behavior. Camera accepts automatic fit or a
fixed center plus zoom. Focus can dim/isolate art, show skeleton diagnostics, and
request motion trails; contact sheets use explicit times or evenly spaced frames
and a common camera.

Prefer one `run_script` call that validates names, performs related edits, and logs
a summary over many tiny calls. Discover unfamiliar operations through `list_verbs`.
An end-to-end safe flow is: open → inspect → list verbs → run one script → inspect
again → render a frame/contact sheet → save to a new `.ankh` → export to a dedicated
directory. Scripts remain sandboxed with no filesystem, network, or clock.
