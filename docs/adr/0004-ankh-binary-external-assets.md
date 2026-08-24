# ADR 0004: binary Ankh v1 with external assets

- **Status:** Accepted
- **Date:** 2026-08-25
- **Decision owner:** Ankhimate core team
- **Records:** `docs/format-spec.md`

## Context

The project has not shipped, so there is no compatibility obligation. A ZIP
prototype embedded images and pretty JSON; a five-animation Tweegee project was
5.54 MB, with 5.49 MB spent on uncompressed JSON. Embedded images also coupled
animation data to artwork managed separately by the game.

## Decision

Start the public format at v1 with three equivalent profiles: binary `.ankh` by
default, readable `.ankh.json`, and short-key `.ankh.min.json`. The binary uses a
small checksummed envelope around compressed MessagePack. Images are external,
content-addressed files in a sibling asset directory.

The schema remains name-keyed. Binary compactness never means serializing Rust
slotmap ids or in-memory layout.

## Consequences

- Projects are small and fast to parse; artwork can be replaced independently.
- Moving a project requires moving its sibling asset directory.
- Compact key tags and the binary envelope are public contracts.
- Saves are multi-file operations: assets land before the project, orphans are
  reported rather than deleted.
- No reader, writer, migration, fixture, or documentation for the ZIP prototype
  remains.
