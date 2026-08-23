---
title: Plugins and automation
description: Extend formats and workflows through sandboxed plugins, named verbs, and the MCP server.
---

# Plugins and automation

Plugins, the editor, and MCP share `Document`, named verbs, importer/exporter
registries, and the read surface. They are consumers of one mutation boundary,
not parallel implementations.

![Editor, plugins, and MCP converge on document verbs and shared services.](../diagrams/automation.svg)
