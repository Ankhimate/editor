# Community format plugins

These are ordinary sandboxed JavaScript plugins, not Rust crates and not
compiled into Ankhimate.

- `spine/` registers `import.spine` and `export.spine`. Its export template is
  a package resource read through `ankhimate.resource()`.
- `dragonbones/` registers `import.dragonbones`.

To install one, copy its whole directory into Ankhimate's platform configuration
`plugins` directory, preserving `plugin.js` and any files beside it, then reload
plugins or restart the editor. MCP reads the same directory when it starts.

Without these directories installed, Spine and DragonBones do not appear in the
import registry and Spine does not appear in the export formats. Plugins receive
only the input file's confined sidecars and their packaged resources; they have
no general filesystem, network, or clock access.
