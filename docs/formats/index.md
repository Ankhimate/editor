---
title: File and export formats
description: Understand the editable .ankh container and the separate user-authored runtime export contract.
---

# File and export formats

The `.ankh` authoring file and an engine export solve different problems. The
authoring file preserves editable rig structure and embedded artwork. Runtime
output is generated from a user-authored preset and contains only what a target
engine contract asks for.

The authoring contract is documented in [Ankh v1](../format-spec.md): binary by
default, standard JSON by option, and external content-addressed images.
