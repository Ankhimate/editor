//! Exporters written in JavaScript.
//!
//! Export is already user-authored — `docs/export-plan.md` makes the case, and a
//! Handlebars template over a documented context is the answer for most formats.
//! A template is deliberately logic-less, though, and some formats need what it
//! cannot do: a checksum over what was written, a binary header, an index built
//! by counting, a layout that depends on the rig rather than on the template.
//!
//! So this is the second road, not a replacement. A preset stays the right tool
//! when a format is a projection of the context; a plugin is for when it is not.
//!
//! # An exporter emits files, and the host writes them
//!
//! A plugin does not touch the disk. It calls `emit(path, contents)` and the
//! host builds an `export::Plan` from the result — which is the same type the
//! template path produces, so everything downstream is unchanged:
//!
//! - paths are confined to the output directory (rig data can name a file, and
//!   rigs arrive from other people);
//! - the write is all-or-nothing;
//! - nothing is ever deleted, and orphans are reported.
//!
//! Those three rules are `docs/export-plan.md`'s and they are not negotiable for
//! a plugin either. Handing a script a file handle would put every one of them
//! in the plugin author's hands.

use ankhimate_document::Document;
use ankhimate_export::run::Plan;
use ankhimate_export::template::RenderedFile;

/// A rig format a script can write.
pub struct JsExporter {
    pub id: String,
    pub label: String,
    /// The script that registered it, re-run to perform an export.
    ///
    /// Source rather than a compiled function, for the reason the importer gives:
    /// a QuickJS value cannot outlive its runtime, and the host builds a fresh
    /// one per run so a plugin cannot hold state across an undo.
    pub source: String,
}

impl JsExporter {
    /// Run this exporter over `doc`, collecting the files it emits.
    ///
    /// Returns a [`Plan`] rather than writing: the caller decides where it lands
    /// and whether to overwrite, and the confinement and all-or-nothing rules
    /// stay in one place rather than being re-implemented per exporter.
    pub fn plan(&self, doc: Document) -> Result<Plan, crate::PluginError> {
        let host = crate::Host::new();
        let script = format!("{}\n__ankhimate_run_export();", self.source);
        let emitted = host.run_export(&script, doc)?;

        Ok(Plan {
            files: emitted
                .text
                .into_iter()
                .map(|(path, contents)| RenderedFile { path, contents })
                .collect(),
            binaries: emitted.binary,
        })
    }
}

/// What a script emitted during one export run.
#[derive(Default)]
pub struct Emitted {
    /// `(relative path, contents)`.
    pub text: Vec<(String, String)>,
    /// `(relative path, bytes)` — images, or anything a format packs binary.
    pub binary: Vec<(String, Vec<u8>)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Host;

    const PLUGIN: &str = r#"
    ankhimate.registerExporter({
      id: "export.toy",
      label: "Toy Format",
      write() {
        const r = rig();
        const lines = r.skeleton.bones.map(b => b.name + " " + b.rotation);
        emit("rig.txt", lines.join("\n"));
        // A count the template engine could not produce without a helper.
        emit("manifest.txt", "bones=" + r.skeleton.bones.length);
      },
    });
    "#;

    fn rig() -> Document {
        use ankhimate_document::{Args, DocOps, Edit};
        let ops = DocOps::builtin();
        let mut edit = Edit::default();
        for args in [
            serde_json::json!({ "name": "root" }),
            serde_json::json!({ "name": "spine", "parent": "root", "rotation": 30.0 }),
        ] {
            ops.invoke("bone.create", &mut edit, &Args::from_json(args))
                .expect("built");
        }
        edit.doc
    }

    #[test]
    fn a_javascript_exporter_declares_itself() {
        let host = Host::new();
        let declared = host.exporters(PLUGIN).expect("loads");
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].id, "export.toy");
        assert_eq!(declared[0].label, "Toy Format");
    }

    #[test]
    fn a_javascript_exporter_emits_files_from_the_rig() {
        let host = Host::new();
        let exporter = host.exporters(PLUGIN).unwrap().into_iter().next().unwrap();
        let plan = exporter.plan(rig()).expect("exports");

        assert_eq!(plan.files.len(), 2);
        let rig_txt = plan
            .files
            .iter()
            .find(|f| f.path == "rig.txt")
            .expect("rig.txt");
        assert!(
            rig_txt.contents.contains("spine 30"),
            "{}",
            rig_txt.contents
        );

        let manifest = plan
            .files
            .iter()
            .find(|f| f.path == "manifest.txt")
            .expect("manifest.txt");
        assert_eq!(manifest.contents, "bones=2");
    }

    #[test]
    fn an_exporter_reads_the_same_context_a_template_does() {
        // One contract, not two. Someone who has written a preset already knows
        // these field names, and `docs/export-context.md` documents both.
        let host = Host::new();
        let exporter = host
            .exporters(
                r#"
                ankhimate.registerExporter({
                  id: "export.v", label: "Version",
                  write() { emit("v.txt", String(rig().context_version)); },
                });
                "#,
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let plan = exporter.plan(rig()).expect("exports");
        assert_eq!(plan.files[0].contents, "1");
    }

    #[test]
    fn an_exporter_that_emits_nothing_produces_an_empty_plan() {
        // Not an error: a format with nothing to say about an empty rig should
        // write nothing rather than a file with nothing in it.
        let host = Host::new();
        let exporter = host
            .exporters(
                r#"
                ankhimate.registerExporter({
                  id: "export.quiet", label: "Quiet", write() {},
                });
                "#,
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let plan = exporter.plan(rig()).expect("exports");
        assert!(plan.files.is_empty() && plan.binaries.is_empty());
    }

    #[test]
    fn a_throwing_exporter_writes_nothing() {
        // All-or-nothing reaches a plugin: a script that fails halfway must not
        // leave the files it managed to emit, because a half-written export is
        // one the user has to notice is half-written.
        let host = Host::new();
        let exporter = host
            .exporters(
                r#"
                ankhimate.registerExporter({
                  id: "export.bad", label: "Bad",
                  write() {
                    emit("good.txt", "fine");
                    throw new Error("could not encode the second file");
                  },
                });
                "#,
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let Err(err) = exporter.plan(rig()) else {
            panic!("the throw should reach the caller");
        };
        assert!(format!("{err}").contains("second file"), "{err}");
    }
}
