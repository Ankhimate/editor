# ADR 004: .ankh zip container, name-keyed serialization

- **Status:** Accepted
- **Date:** 2026-07-30
- **Decision owner:** Ankhimate core team
- **Records:** PLAN §6.1, §1.2 (defect D8)

## Context

The original save path serialized `SlotMap` keys via `bincode` + serde on the
slotmap (defect D8). Slotmap keys are **not stable across versions** — a key
generated today is not guaranteed to be valid tomorrow, and serialized slotmaps
embed internal state that breaks forward/backward compatibility. Additionally,
the save format stored bincode (opaque binary), making round-trip debugging and
migration painful.

We need an on-disk format that survives crate upgrades and is inspectable.

## Decision

The `.ankh` project file is a **zip** container:

```
project.ankh (zip)
├─ project.json      # version, name, fps, document (name-keyed JSON)
├─ images/…          # original source PNGs (referenced by asset id)
└─ thumbs/cover.png  # optional, for startup window
```

- `project.json` keys all entities by **name strings**
  (`"bones": [{"name": "arm", "parent": "shoulder", …}]`), never slotmap keys.
- `"version": 1` is mandatory.
- Unknown fields are preserved on round-trip where feasible (serde flatten).
- `formats/src/migrate.rs` matches on `version` (identity for v1 now; the
  scaffold exists so v2 migration is a known extension point).

## Alternatives considered

- **Single JSON file:** no place for bundled images; becomes unusably large.
- **Keep bincode of slotmaps:** version-fragile, opaque, rejected (this is D8).
- **Custom binary container:** zip is already well-understood and supported by
  the `zip` crate; no need to reinvent.

## Consequences

- Names must be **unique per entity kind** — `Skeleton::add_bone` etc. enforce
  this by appending `_2` on collision.
- Loading maps names back to fresh slotmap keys (one pass at load time).
- Round-trip golden tests guarantee value-equality across save/load.
- The bincode path is removed from `export/`.
