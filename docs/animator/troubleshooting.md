---
title: Troubleshooting by symptom
description: Diagnose snapping transforms, bad deformation, constraint surprises, missing assets, and export errors.
---

# Troubleshooting by symptom

| Symptom | Check |
|---|---|
| A transform snaps back | In Animate mode, key the changed property/axis; an unkeyed pending pose is temporary. |
| A structural edit is refused | Switch to Setup mode; mode requirements are enforced by the command. |
| Artwork moves with the wrong bone | Check slot ownership, active skin entry, parent, and bind weights. |
| A mesh folds or tears | Inspect triangles, UVs, influence normalization, locks, and neighboring weight gradients. |
| IK bends backward | Change bend direction and verify bone lengths and target reachability. |
| Physics changes after a seek | Allow state to settle; fixed-step physics is stateful even though integration is deterministic. |
| A curve turns the short way | Rotation takes the shortest arc; add an intermediate key for a long turn. |
| An imported image is absent | Read `LoadReport`, verify sidecars/package resources, and preserve relative paths. |
| Export misses a value | Strict templates error on absent fields; use only fields in the export-context contract. |
| Old export files remain | This is intentional: export never deletes. Remove reviewed orphans yourself. |
| Build says “Access is denied” on Windows | Close the running editor binary or use `cargo check`. |
