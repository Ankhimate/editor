//! A complete importer in JavaScript: artwork, a mesh, and animation.
//!
//! `docs/export-plan.md` requires our own runtime format to be a template, so a
//! format the engine cannot express is found before a user finds it. This is
//! the import side of that rule, and the answer to "could a shipped importer be
//! a plugin?" — everything the Rust readers do to a rig, done from script.
//!
//! What is deliberately *not* here is a re-implementation of Spine. The point is
//! the surface, not the format: a plugin that can bring across images, a
//! weighted mesh, keyframes and a report can bring across anything those pieces
//! compose into.

use ankhimate_plugins::Host;

/// The smallest valid PNG — 1×1, transparent RGBA.
///
/// Generated rather than hand-written: the first attempt had a wrong IDAT
/// length and a wrong CRC, which `image` rejected, so the size came back
/// `(0, 0)` and the test read as a base64 bug rather than a bad fixture.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x00, 0x02, 0x00,
    0x00, 0x05, 0x00, 0x01, 0x7A, 0x5E, 0xAB, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

/// An importer for a made-up format that carries everything a real one does.
const PLUGIN: &str = r#"
ankhimate.registerImporter({
  id: "import.full",
  label: "Full Toy Rig",
  extensions: ["toy"],

  read(text, fileName) {
    const rig = JSON.parse(text);

    // 1. Artwork, from a binary sidecar.
    for (const image of rig.images ?? []) {
      const bytes = ankhimate.sidecarBytes(image.file);
      if (bytes == null) {
        ops.invoke("import.report", {
          what: "image", where: image.file,
          detail: "the file was not beside the rig, so nothing draws for it",
        });
        continue;
      }
      ops.invoke("asset.add_image", { name: image.name, bytes_base64: bytes });
    }

    // 2. Structure.
    for (const bone of rig.bones) {
      ops.invoke("bone.create", {
        name: bone.name, parent: bone.parent,
        x: bone.x ?? 0, y: bone.y ?? 0, rotation: bone.rotation ?? 0,
      });
    }
    for (const slot of rig.slots ?? []) {
      ops.invoke("slot.create", { name: slot.name, bone: slot.bone });
    }

    // 3. Attachments — regions and meshes alike.
    for (const att of rig.attachments ?? []) {
      if (att.vertices) {
        ops.invoke("attachment.create_mesh", {
          slot: att.slot, name: att.name, texture: att.texture,
          vertices: att.vertices, uvs: att.uvs, triangles: att.triangles,
          weights: att.weights,
        });
      } else {
        ops.invoke("attachment.create_region", {
          slot: att.slot, name: att.name, texture: att.texture,
          x: att.x ?? 0, y: att.y ?? 0,
        });
      }
    }

    // 4. Animation.
    for (const clip of rig.clips ?? []) {
      ops.invoke("anim.create", { name: clip.name, duration: clip.seconds });
      for (const key of clip.keys ?? []) {
        ops.invoke("anim.key_bone", {
          animation: clip.name, bone: key.bone, property: key.property,
          axis: key.axis, time: key.time, value: key.value,
          interp: key.interp,
        });
      }
      for (const swap of clip.swaps ?? []) {
        ops.invoke("anim.key_attachment", {
          animation: clip.name, slot: swap.slot,
          time: swap.time, attachment: swap.attachment,
        });
      }
    }

    // 5. What could not come across.
    for (const gap of rig.unsupported ?? []) {
      ops.invoke("import.report", {
        what: gap.what, where: gap.where, detail: gap.detail,
      });
    }
  },
});
"#;

const RIG: &str = r#"{
  "images": [{ "name": "torso", "file": "torso.png" }],
  "bones": [
    { "name": "root" },
    { "name": "spine", "parent": "root", "y": 40, "rotation": 15 }
  ],
  "slots": [{ "name": "body", "bone": "spine" }],
  "attachments": [
    {
      "slot": "body", "name": "cloth", "texture": "torso",
      "vertices": [0, 0, 10, 0, 10, 10],
      "uvs": [0, 0, 1, 0, 1, 1],
      "triangles": [0, 1, 2],
      "weights": [
        [{ "bone": "root", "weight": 1 }],
        [{ "bone": "root", "weight": 0.5 }, { "bone": "spine", "weight": 0.5 }],
        [{ "bone": "spine", "weight": 1 }]
      ]
    }
  ],
  "clips": [{
    "name": "walk", "seconds": 1.0,
    "keys": [
      { "bone": "spine", "property": "rotate", "time": 0.0, "value": 0 },
      { "bone": "spine", "property": "rotate", "time": 0.5, "value": 45 },
      { "bone": "spine", "property": "translate", "axis": "y", "time": 0.5, "value": 12 }
    ],
    "swaps": [
      { "slot": "body", "time": 0.0, "attachment": "cloth" },
      { "slot": "body", "time": 0.8 }
    ]
  }],
  "unsupported": [
    { "what": "constraint", "where": "spine_ik", "detail": "IK is not read yet" }
  ]
}"#;

fn write_rig(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("hero.toy");
    std::fs::write(&path, RIG).unwrap();
    std::fs::write(dir.join("torso.png"), PNG_1X1).unwrap();
    path
}

fn import(dir: &std::path::Path) -> ankhimate_document::Edit {
    let host = Host::new();
    let importer = host
        .importers(PLUGIN)
        .expect("the plugin loads")
        .into_iter()
        .next()
        .expect("one importer");
    importer.read(&write_rig(dir)).expect("the import runs")
}

#[test]
fn a_javascript_importer_brings_artwork_across() {
    // The gap that made "could Spine be a plugin?" a no: a text-only sidecar
    // cannot carry a PNG, so no plugin could produce a rig that draws.
    let dir = tempfile::tempdir().unwrap();
    let edit = import(dir.path());

    let asset = edit.doc.assets.images.values().next().expect("one image");
    assert_eq!(asset.name, "torso");
    assert_eq!((asset.width, asset.height), (1, 1), "read from the pixels");
    assert!(!asset.bytes.is_empty(), "and the bytes survived base64");
}

#[test]
fn a_javascript_importer_builds_a_weighted_mesh() {
    use ankhimate_core::attachment::Attachment;

    let dir = tempfile::tempdir().unwrap();
    let edit = import(dir.path());

    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(id, _)| id)
        .expect("body");
    let skin = edit.doc.skeleton.default_skin;

    match edit.doc.skeleton.skins[skin].get(slot, "cloth") {
        Some(Attachment::Mesh(m)) => {
            assert_eq!(m.setup_vertices.len(), 3);
            assert_eq!(m.triangles, [[0, 1, 2]]);
            assert_eq!(m.weights[1].len(), 2, "a vertex shared by two bones");
        }
        other => panic!("expected a mesh, got {other:?}"),
    }
}

#[test]
fn a_javascript_importer_writes_animation() {
    use ankhimate_core::animation::{Axis, Timeline};

    let dir = tempfile::tempdir().unwrap();
    let edit = import(dir.path());

    let clip = edit.doc.animations.values().next().expect("walk");
    assert_eq!(clip.name, "walk");

    let rotate = clip
        .timelines
        .iter()
        .find_map(|t| match t {
            Timeline::BoneRotate { keys, .. } => Some(keys),
            _ => None,
        })
        .expect("a rotate track");
    assert_eq!(rotate.len(), 2);
    assert_eq!(rotate[1].value, 45.0, "degrees, as given");

    assert!(
        clip.timelines
            .iter()
            .any(|t| matches!(t, Timeline::BoneTranslate { axis: Axis::Y, .. })),
        "and the y axis went to the y track"
    );

    let swaps = clip
        .timelines
        .iter()
        .find_map(|t| match t {
            Timeline::SlotAttachment { keys, .. } => Some(keys),
            _ => None,
        })
        .expect("an attachment track");
    assert_eq!(swaps.len(), 2);
    assert_eq!(swaps[1].value, None, "hidden at the end");
}

#[test]
fn a_javascript_importer_says_what_it_could_not_carry() {
    // The honesty property. A plugin that drops half a file quietly is worse
    // than one that refuses, and until now a plugin had no way to say so.
    let dir = tempfile::tempdir().unwrap();
    let edit = import(dir.path());

    assert_eq!(edit.report.len(), 1);
    assert_eq!(edit.report[0].what, "constraint");
    assert_eq!(edit.report[0].where_, "spine_ik");
}

#[test]
fn a_missing_image_is_reported_rather_than_guessed_at() {
    // The rig names a sidecar that is not there. An importer that carried on
    // silently would produce a rig whose artwork is missing for no stated
    // reason — which is the failure this whole reporting path exists to avoid.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hero.toy");
    std::fs::write(&path, RIG).unwrap();
    // Deliberately no torso.png.

    let host = Host::new();
    let importer = host.importers(PLUGIN).unwrap().into_iter().next().unwrap();
    let edit = importer.read(&path).expect("the rig still imports");

    assert!(edit.doc.assets.images.is_empty());
    assert!(
        edit.report.iter().any(|r| r.what == "image"),
        "the missing image is named: {:?}",
        edit.report
    );
    assert_eq!(
        edit.doc.skeleton.bones.len(),
        2,
        "and the rest of the rig came across anyway"
    );
}

#[test]
fn the_whole_import_undoes() {
    // What a Rust importer cannot do: those replace the document wholesale.
    // A plugin's import is a run of commands, so it walks back.
    let dir = tempfile::tempdir().unwrap();
    let mut edit = import(dir.path());

    assert_eq!(edit.doc.skeleton.bones.len(), 2);
    while edit.undo() {}
    assert_eq!(edit.doc.skeleton.bones.len(), 0);
    assert!(edit.doc.animations.is_empty());
}
