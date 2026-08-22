# Community format plugins

These are ordinary sandboxed JavaScript plugins, not Rust crates or built-in
format handlers. Their source ships as opt-in packages; installation writes the
same editable JS files that a manually installed community plugin uses.

- `spine/` registers `import.spine` and `export.spine`. Its export template is
  a package resource read through `ankhimate.resource()`.
- `dragonbones/` registers `import.dragonbones`.

In the editor, open **File → Import → Community importers** and install the
package you want. It is copied as ordinary JavaScript into Ankhimate's platform
configuration `plugins` directory and discovered immediately. Manual installs
work too: copy the whole directory, preserving `plugin.js` and any files beside
it, then restart the editor. MCP reads the same directory when it starts.

Without these directories installed, Spine and DragonBones do not appear in the
import registry and Spine does not appear in the export formats. Plugins receive
only the input file's confined sidecars and their packaged resources; they have
no general filesystem, network, or clock access.
