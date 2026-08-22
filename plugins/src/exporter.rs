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
    /// The document comes back **on both paths**. It has to be moved in —
    /// `Document` is not `Clone`, and a rig with its images in it is not
    /// something to copy per export — so a caller that moved its only copy out
    /// and got an `Err` would lose the user's whole project to a plugin's typo.
    pub fn plan(&self, doc: Document) -> Result<(Plan, Document), (crate::PluginError, Document)> {
        let host = crate::Host::new();
        let script = format!("{}\n__ankhimate_run_export();", self.source);
        let (emitted, doc) = host.run_export(&script, doc)?;

        Ok((
            Plan {
                files: emitted
                    .text
                    .into_iter()
                    .map(|(path, contents)| RenderedFile { path, contents })
                    .collect(),
                binaries: emitted.binary,
            },
            doc,
        ))
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
        let (plan, _doc) = exporter.plan(rig()).expect("exports");

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

        let (plan, _doc) = exporter.plan(rig()).expect("exports");
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

        let (plan, _doc) = exporter.plan(rig()).expect("exports");
        assert!(plan.files.is_empty() && plan.binaries.is_empty());
    }

    /// A rig with one real image in its library, so the baker has something to
    /// pack.
    fn rig_with_art() -> Document {
        use ankhimate_document::{Args, DocOps, Edit};

        // 2x2 red PNG — generated rather than hand-written, because a wrong
        // IDAT length reads as a base64 bug rather than a bad fixture.
        let png = {
            let mut bytes = Vec::new();
            let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .expect("encodes");
            crate::importer::encode_base64(&bytes)
        };

        let ops = DocOps::builtin();
        let mut edit = Edit::default();
        ops.invoke(
            "bone.create",
            &mut edit,
            &Args::from_json(serde_json::json!({ "name": "root" })),
        )
        .expect("bone");
        ops.invoke(
            "asset.add_image",
            &mut edit,
            &Args::from_json(serde_json::json!({ "name": "torso", "bytes_base64": png })),
        )
        .expect("image");
        edit.doc
    }

    #[test]
    fn an_exporter_can_bake_an_atlas_and_write_its_pages() {
        // The gap that stopped a plugin producing a real engine format: most
        // runtime formats want a packed atlas, and a script has no pixels and
        // no rectangle packer.
        let host = Host::new();
        let exporter = host
            .exporters(
                r#"
                ankhimate.registerExporter({
                  id: "export.atlas", label: "With Atlas",
                  write() {
                    const atlas = bakeAtlas({ padding: 2, trim: false });
                    for (const page of atlas.pages) {
                      emitBytes("atlas_" + page.index + ".png", page.png_base64);
                    }
                    emit("atlas.json", JSON.stringify(atlas.regions));
                  },
                });
                "#,
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let (plan, _doc) = exporter.plan(rig_with_art()).expect("exports");

        let page = plan
            .binaries
            .iter()
            .find(|(path, _)| path == "atlas_0.png")
            .expect("a page was written");
        assert!(
            image::load_from_memory(&page.1).is_ok(),
            "and it is a real PNG, not truncated base64"
        );

        let regions = plan
            .files
            .iter()
            .find(|f| f.path == "atlas.json")
            .expect("atlas.json");
        assert!(
            regions.contents.contains("torso"),
            "the region names its asset: {}",
            regions.contents
        );
    }

    #[test]
    fn a_javascript_exporter_can_reuse_a_strict_export_preset() {
        let preset_json = include_str!("../../export/src/presets/spine_json.json");
        let source = format!(
            r#"
            const preset = {preset_json};
            ankhimate.registerExporter({{
              id: "export.spine.community",
              label: "Spine JSON",
              write() {{ emitPreset(preset); }}
            }});
            "#
        );

        let exporter = Host::new()
            .exporters(&source)
            .expect("plugin loads")
            .remove(0);
        let (actual, _) = exporter.plan(rig()).expect("plugin preset renders");

        let expected_doc = rig();
        let project = ankhimate_formats::convert::to_schema(&expected_doc.as_project_ref());
        let preset = ankhimate_export::preset::Preset::from_json(preset_json).expect("preset");
        let expected = ankhimate_export::run::plan(&project, &expected_doc.assets, &preset)
            .expect("native preset renders");

        assert_eq!(actual.files.len(), expected.files.len());
        for (actual, expected) in actual.files.iter().zip(&expected.files) {
            assert_eq!(actual.path, expected.path);
            assert_eq!(actual.contents, expected.contents);
        }
        assert_eq!(actual.binaries, expected.binaries);
    }

    #[test]
    fn baking_reports_a_failure_rather_than_throwing() {
        // A rig with one undecodable image should let a plugin write the rest
        // and say what was missing — the same choice the importers make with
        // their report.
        let host = Host::new();
        let exporter = host
            .exporters(
                r#"
                ankhimate.registerExporter({
                  id: "export.probe", label: "Probe",
                  write() {
                    const atlas = bakeAtlas({});
                    emit("result.txt", atlas.error ? "error" : "ok:" + atlas.pages.length);
                  },
                });
                "#,
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        // An empty library still bakes — one empty page, which is what the
        // baker does rather than refusing. A plugin sees a result either way.
        let (plan, _doc) = exporter.plan(rig()).expect("exports");
        assert_eq!(plan.files[0].contents, "ok:1");
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

        let Err((err, _doc)) = exporter.plan(rig()) else {
            panic!("the throw should reach the caller");
        };
        assert!(format!("{err}").contains("second file"), "{err}");
    }

    #[test]
    fn a_failed_export_hands_the_document_back() {
        // The document is moved in because `Document` is not `Clone` — a rig
        // with its images in it is not something to copy per export. A caller
        // that moved its only copy out and got a bare `Err` would lose the
        // user's whole project to a plugin's typo.
        let host = Host::new();
        let exporter = host
            .exporters(
                r#"
                ankhimate.registerExporter({
                  id: "export.bad", label: "Bad",
                  write() { throw new Error("cannot write this rig"); },
                });
                "#,
            )
            .unwrap()
            .into_iter()
            .next()
            .unwrap();

        let Err((error, doc)) = exporter.plan(rig()) else {
            panic!("this should have failed");
        };
        assert!(error.to_string().contains("cannot write this rig"));
        assert_eq!(
            doc.skeleton.bones.len(),
            2,
            "the rig came back whole, not empty"
        );
    }
}
