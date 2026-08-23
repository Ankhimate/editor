---
title: Export and runtime contracts
description: Distinguish authoring projects from strict template output and external runtime responsibilities.
---

# Export and runtime contracts

Export presets combine atlas settings and one or more strict Handlebars templates.
The [template context reference](/editor/export-context/) is a public compatibility
contract: field names cannot be silently renamed. Missing fields are errors with
locations, never empty strings. Helpers and atlas page/region data are documented
in that reference.

Ankhimate's runtime shape is itself the `ankhimate_runtime` built-in preset, not a
special Rust serializer. This is load-bearing: if the public template engine cannot
express the project's own runtime output, it is too weak for users' engines.

Every export builds a complete plan first. Rendered paths are confined below the
chosen directory; duplicate or escaping paths fail; writes are all-or-nothing as
far as planning permits; no output is ever deleted; old unclaimed paths are orphans
for the user to review. Plugin exporters emit into this same plan.

Runtime implementations live outside this workspace. A conforming runtime consumes
the documented generated shape, respects animation/constraint semantics it claims
to support, and tests representative exports. The `.ankh` authoring container is
not a runtime contract and includes editor-only and source data by design.
