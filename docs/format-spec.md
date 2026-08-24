# Ankh v1 project format

`.ankh` is the binary authoring format. This is version 1; no earlier Ankh
format is supported.

## Profiles

| Profile | Suffix | Encoding |
|---|---|---|
| Binary | `.ankh` | Default; compressed MessagePack with short keys |
| Readable JSON | `.ankh.json` | Standard JSON with descriptive keys |
| Minified JSON | `.ankh.min.json` | Standard minified JSON with short keys |

All three encode the same name-keyed schema. Slotmap ids never go on disk.
Angles are degrees and coordinates are Y-up. Encoding the same project twice is
deterministic.

## Binary envelope

```text
offset  size  value
0       4     ASCII `ANKH`
4       2     version, little-endian (`1`)
6       1     codec (`1` = MessagePack compact schema)
7       1     flags (`bit 0` = Deflate payload)
8       4     uncompressed payload length, little-endian
12      4     CRC-32 of the uncompressed payload
16      ...   payload
```

The payload is the same short-key value tree used by `.ankh.min.json`, encoded
as MessagePack. Writers Deflate it only when compression makes it smaller.

## Short keys

Known schema keys map to one or two base-62 characters through the append-only
table in `formats/src/compact.rs`. A published table entry is never reordered or
reused. Unknown fields are escaped with `~` and survive expansion, so a readable
JSON round trip does not confuse them with known wire tags.

`.ankh.min.json` is valid UTF-8 JSON with no indentation or line breaks. The
final `.json` suffix lets ordinary editors and syntax highlighters recognize it.

## External images

Images are never embedded. For `hero.ankh`, they live under the sibling
`hero.assets/` directory. Each schema asset contains a logical name, dimensions,
and a relative `file` URI such as:

```text
hero.assets/8f/8f3d…c2.png
```

The filename is the SHA-256 of the original encoded bytes. Equal images produce
one file. Readers reject absolute paths, `..`, drive prefixes, and every URI that
could escape the asset root. Missing images are reported as dangling assets but
do not prevent the rig from loading.

Saving writes missing assets first and the project last. Existing hash-named
assets are reused. Unreferenced files are never deleted automatically.

## Required behavior

- Binary, readable JSON, and minified JSON decode to value-equal projects.
- `.ankh` starts with `ANKH`, not ZIP magic, and contains no image bytes.
- Corrupt length or CRC fails the load.
- Duplicate images are stored once.
- A missing image reports a dangling asset and preserves the rig.
- Project references are names in every profile.
