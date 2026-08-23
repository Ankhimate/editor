---
title: Migrations and compatibility
description: Version upgrade behavior, future-version refusal, and round-trip expectations for .ankh files.
---

# Migrations and compatibility

The current schema version is 3. Version 0 is invalid. A version newer than the
reader is refused with both found and supported versions; guessing at future
semantics would risk corruption.

Migration is sequential:

- **v1 → v2:** paired translate, scale, and shear timelines split into independent
  X and Y tracks while copying times and easing. Four transform-constraint mix
  values expand to seven named per-axis values; paired channels receive the old value.
- **v2 → v3:** weighted mesh deform keys expand each vertex XY offset across all
  of that vertex's influences. Rigid mesh offsets already have the correct shape.

These transforms preserve the evaluated appearance while allowing more expressive
future edits. JSON shape changes run before typed deserialization; typed moves and
renames run afterward. Every breaking schema change must bump the version and add
a tested migration. Older readers preserve unknown JSON fields only where the
schema type flattens an `extra` map.
