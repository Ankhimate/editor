# Ankh project format

Status: proposed v4 contract. Versions 1–3 remain readable during migration.

## Goals

- Small, fast authoring files by default.
- Images are ordinary external files that Photoshop and build tools can replace.
- A readable JSON representation remains available for debugging and interchange.
- A compact JSON representation is available where a text-only transport is
  required.
- Every cross-reference remains name-based. Runtime slotmap keys never go on disk.

## Project profiles

All profiles represent the same logical v4 schema and carry `version = 4` and a
profile identifier. Conversion between profiles must preserve the logical model.

| Profile | Suggested suffix | Purpose |
|---|---|---|
| Binary | `.ankh` | Default editor and pipeline format |
| Readable JSON | `.ankh.json` | Review, debugging, hand editing |
| Minified JSON | `.ankh.min.json` | Standard JSON with minimum text overhead |

Binary is the canonical default, not a serialization of in-memory slotmap keys.
It encodes the same name-keyed schema as JSON. The binary codec and its canonical
ordering are part of the public format contract; changing Rust structs alone must
never change bytes on disk.

Readable JSON uses descriptive field names and no insignificant whitespace is
required. Editors may pretty-print it. Minified JSON is always minified and uses
the stable tags below. Compact tags are aliases, not a second data model.

## External assets

Images are not embedded in any project profile. For `hero.ankh`, the default
asset root is the sibling directory `hero.assets/`:

```text
hero.ankh
hero.assets/
  8f/8f3d…c2.png
  b1/b19a…44.webp
```

Each asset record contains:

- logical `name`, referenced by attachments;
- relative `uri`, resolved from the project file's directory;
- `sha256`, lower-case hexadecimal content hash;
- pixel `width` and `height`;
- optional advisory `source_uri` for reload-from-source.

The default writer stores files by content hash under `<stem>.assets/`. This
deduplicates identical images and makes overwrites safe. Readers also accept any
confined relative URI so a hand-authored project may point at `art/head.png`.
Absolute asset URIs are rejected in the portable profiles. `..`, drive prefixes,
and paths escaping the project directory are rejected.

Missing images do not prevent the rig from loading. They are reported as
dangling assets and render as missing, matching the existing tolerant load rule.

### Save transaction

Saving is ordered:

1. encode project bytes to a temporary sibling file;
2. write missing content-addressed assets atomically;
3. atomically replace the project file last;
4. report unreferenced asset files, but never delete them automatically.

A crash can therefore leave an orphan image, but cannot publish a project that
names an image the save did not finish. Repeating the save is idempotent.

## Binary envelope

The binary profile begins with a fixed envelope independent of the payload codec:

```text
offset  size  value
0       4     ASCII `ANKH`
4       2     format version, little-endian (`4`)
6       1     codec (`1` = compact schema binary)
7       1     flags (`bit 0` = payload compressed)
8       4     uncompressed payload length, little-endian
12      4     CRC-32 of the uncompressed payload
16      ...   payload
```

The v4 payload is the compact tagged schema encoded as deterministic binary maps
and arrays. Maps are emitted in ascending tag order. Floats are IEEE-754 little-
endian; non-finite floats are invalid. Strings are UTF-8. Unknown map tags must be
skipped and preserved when the codec can retain their raw value.

Compression is an envelope concern. Writers enable it only when it makes the
payload smaller; readers must support both flag states.

## Minified JSON tags

Top-level tags are stable and never reused:

| Tag | Readable field |
|---|---|
| `v` | `version` |
| `n` | `name` |
| `f` | `fps` |
| `as` | `assets` |
| `bo` | `bones` |
| `sl` | `slots` |
| `do` | `draw_order` |
| `sk` | `skins` |
| `ds` | `default_skin` |
| `co` | `constraints` |
| `oo` | `constraint_order` |
| `an` | `animations` |
| `gr` | `groups` |
| `pp` | `psd_layer_paths` |
| `ep` | `export_presets` |

Nested records use a separate tag table per record type, documented beside the
schema implementation. A tag's meaning is scoped by its record type. Tags may be
one or two ASCII characters; once published they are never renamed or reused.
Unknown tags survive readable/compact/binary round trips.

Minified JSON contains no indentation or line breaks. It is UTF-8 and must use
JSON numbers and booleans normally; numeric values are not converted to strings.

## Compatibility and rollout

The migration is expand-first:

1. Add v4 readers while retaining the v1–v3 ZIP/`project.json` reader.
2. Add explicit writers for all three v4 profiles.
3. Add an editor save-profile choice and make Binary the default only after all
   first-party tools can read it.
4. Update the AS3 exporter and runtime fixtures.
5. Keep legacy reads indefinitely. Do not rewrite a file merely because it was
   opened.

Rollback is conversion to readable `.ankh.json`; it is the normative escape
hatch for tools that do not yet implement the binary codec. Legacy ZIP export may
remain available during the compatibility window but must not be labeled v4.

## Acceptance

- Binary, readable JSON, and minified JSON decode to value-equal projects.
- Saving the same project twice produces identical project bytes.
- Existing v1–v3 fixtures still load, including embedded images.
- A v4 save contains no image bytes and every URI stays inside the project tree.
- Duplicate image contents produce one external asset file.
- Missing assets are reported without failing the rig load.
- Interrupted saves leave the old project readable.
