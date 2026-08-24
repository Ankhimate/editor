# ADR 0008: `.ankh` v4 profiles and external assets

- **Status:** Proposed
- **Date:** 2026-08-25
- **Decision owner:** Ankhimate core team
- **Supersedes on adoption:** ADR 0004's default writer and embedded-image rule

## Context

The v1–v3 ZIP container optimized for one-file portability and readable migration.
Real animation exports show the cost: a five-animation file is 5.54 MB, of which
5.49 MB is uncompressed `project.json`. Embedded images also couple equipment art
to animation data even when the game manages those assets independently.

The editor still needs a readable interchange form and must keep name-based
references; returning to serialized slotmap keys would revive ADR 0004's original
compatibility defect.

## Decision

Adopt the three v4 profiles defined by `docs/format-spec.md`:

- compact deterministic binary as the `.ankh` default;
- descriptive JSON as an explicit interchange/debug profile;
- minified tagged standard JSON (`.ankh.min.json`) as an explicit compact text
  profile.

Images become confined external relative assets. The default layout is a sibling
content-addressed `<project-stem>.assets/` directory. Existing ZIP projects remain
readable and are never destructively upgraded on open.

## Consequences

- Project files become substantially smaller and equipment art can be replaced
  independently.
- A project is no longer portable as one file; moving it means moving its asset
  directory too. Readable JSON is the compatibility escape hatch, not a portable
  image bundle.
- Saving becomes a multi-file transaction. Assets are written before the project,
  and unused files are reported rather than deleted.
- Compact tags and the binary envelope become public contracts requiring golden
  fixtures in every runtime.
- The AS3 exporter, editor, MCP server, and external runtimes need a staged rollout
  before Binary can become the default writer.

## Rejected alternatives

- **Only minify descriptive JSON:** smaller, but still pays repeated long keys and
  parse cost.
- **Short keys in the only JSON schema:** saves bytes but destroys the format's
  debugging and migration value.
- **Embed images optionally in v4:** retains two asset ownership models and makes
  portability ambiguous. Legacy ZIP remains the explicit compatibility path.
- **Serialize Rust memory/slotmap ids:** compact but version-fragile and not
  implementable by other runtimes.
