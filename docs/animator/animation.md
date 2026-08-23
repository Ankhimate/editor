---
title: Animation and graph editing
description: Navigate the timeline, key properties explicitly, shape curves, and animate discrete changes.
---

# Animation and graph editing

Create or select a clip, set its duration and loop behavior, then move the playhead.
The dopesheet nests rows by animated object and property. Summary markers combine
children for navigation; property rows contain the actual keys. Select, drag, copy,
or delete keys; snapping aligns time without changing values.

Transform properties key per property and per axis. Explicit key controls record
only the intended channel. Auto-key records supported changes automatically. A
pending pose is a changed Animate-mode value that has not yet been keyed: it can be
useful for comparison, but moving the playhead may replace it with evaluated data.

Linear interpolation changes uniformly. Stepped holds the earlier value. Bézier
handles control time/value slope and may overshoot beyond endpoint values. Rotation
uses the shortest arc across ±180°, so use intermediate keys when a deliberate long
turn is required.

The graph editor displays continuous tracks, supports handle editing, pan/zoom,
and value inspection. See the normative [interaction rationale](/editor/graph-editor/).
Discrete attachment, visibility, event, and draw-order tracks do not grow Bézier
curves; a winning key supplies their value.

Animations can also key slot color and visibility, attachment swaps, mesh deform,
events, ruler markers, and draw order. Bone track offsets retime secondary parts
without destructively moving every key. Duplicate before broad retiming changes.

**Experimental:** onion skinning is available in Animate mode when a clip is
active. Use the transport toggle; right-click it to choose 1–5 neighboring steps,
step between authored keys, or limit ghosts to selected bones. It is still awaiting
the complete T-703 interaction acceptance pass.
