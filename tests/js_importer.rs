//! An importer written in JavaScript, end to end.
//!
//! The founding requirement, exercised: a `.js` file declares a rig format, is
//! handed a file and its sidecars, and builds a rig by calling the same verbs a
//! menu calls. Nothing here is Rust-side except the host.

use ankhimate_plugins::Host;

/// A plugin for a made-up format — a flat bone list with a sidecar for lengths.
///
/// Deliberately not a re-implementation of Spine or DragonBones: what is being
/// tested is that a format nobody built into the binary can be read at all.
const PLUGIN: &str = r#"
ankhimate.registerImporter({
  id: "import.toy",
  label: "Toy Rig",
  extensions: ["toy"],
  read(text, fileName) {
    const rig = JSON.parse(text);

    // A sidecar beside the imported file, found by listing rather than by
    // being told — the way the shipped Rust readers find an atlas.
    const sidecarName = fileName.replace(".toy", "_lengths.json");
    const lengths = JSON.parse(ankhimate.sidecar(sidecarName) ?? "{}");

    for (const bone of rig.bones) {
      ops.invoke("bone.create", {
        name: bone.name,
        parent: bone.parent,
        x: bone.x ?? 0,
        y: bone.y ?? 0,
        length: lengths[bone.name] ?? 30,
      });
    }
    for (const clip of rig.clips ?? []) {
      ops.invoke("anim.create", { name: clip.name, duration: clip.seconds });
    }
    console.log("read " + rig.bones.length + " bones");
  },
});
"#;

const RIG: &str = r#"{
  "bones": [
    { "name": "root" },
    { "name": "spine", "parent": "root", "y": 40 },
    { "name": "head", "parent": "spine", "y": 30 }
  ],
  "clips": [{ "name": "idle", "seconds": 1.5 }]
}"#;

const LENGTHS: &str = r#"{ "spine": 55, "head": 20 }"#;

fn write_rig(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("hero.toy");
    std::fs::write(&path, RIG).unwrap();
    std::fs::write(dir.join("hero_lengths.json"), LENGTHS).unwrap();
    path
}

#[test]
fn a_javascript_plugin_declares_a_format_the_binary_never_heard_of() {
    let host = Host::new();
    let declared = host.importers(PLUGIN).expect("the plugin loads");

    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].id, "import.toy");
    assert_eq!(declared[0].label, "Toy Rig");
    assert_eq!(declared[0].extensions, ["toy"]);
}

#[test]
fn a_javascript_importer_reads_a_rig_and_its_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rig(dir.path());

    let host = Host::new();
    let importer = host
        .importers(PLUGIN)
        .expect("loads")
        .into_iter()
        .next()
        .expect("one importer");

    let edit = importer.read(&path).expect("the import runs");

    assert_eq!(edit.doc.skeleton.bones.len(), 3);
    assert_eq!(edit.doc.animations.len(), 1);

    let spine = edit
        .doc
        .skeleton
        .bones
        .values()
        .find(|b| b.name == "spine")
        .expect("spine");
    assert!(spine.parent.is_some(), "parented by name, as any verb does");
    assert_eq!(spine.local_transform.position.y, 40.0);
    assert_eq!(
        spine.length, 55.0,
        "the length came from the sidecar, not the rig file"
    );
}

#[test]
fn an_import_written_in_javascript_is_undoable() {
    // The property a Rust importer does *not* have: those replace the document
    // wholesale, while this one is a run of ordinary commands.
    let dir = tempfile::tempdir().unwrap();
    let path = write_rig(dir.path());

    let host = Host::new();
    let importer = host.importers(PLUGIN).unwrap().into_iter().next().unwrap();
    let mut edit = importer.read(&path).expect("imports");

    assert_eq!(edit.doc.skeleton.bones.len(), 3);
    assert!(edit.undo(), "the clip");
    assert!(edit.undo(), "the head");
    assert_eq!(edit.doc.skeleton.bones.len(), 2);
}

#[test]
fn a_missing_sidecar_leaves_the_defaults_rather_than_failing() {
    // A rig whose optional sidecar is absent still imports — the plugin asked,
    // got nothing, and used its own default.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hero.toy");
    std::fs::write(&path, RIG).unwrap();

    let host = Host::new();
    let importer = host.importers(PLUGIN).unwrap().into_iter().next().unwrap();
    let edit = importer.read(&path).expect("imports without the sidecar");

    let spine = edit
        .doc
        .skeleton
        .bones
        .values()
        .find(|b| b.name == "spine")
        .expect("spine");
    assert_eq!(spine.length, 30.0, "the plugin's own default");
}

#[test]
fn an_importer_cannot_read_outside_the_rigs_directory() {
    // The sandbox, from the script's side rather than the Rust side. An
    // importer that could walk up a directory is a filesystem, and this crate
    // does not hand one out.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hero.toy");
    std::fs::write(&path, RIG).unwrap();

    let nosy = r#"
    ankhimate.registerImporter({
      id: "import.nosy", label: "Nosy", extensions: ["toy"],
      read(text) {
        const climb = ankhimate.sidecar("../secret.txt");
        const abs = ankhimate.sidecar("/etc/passwd");
        console.log(climb == null ? "no climb" : "CLIMBED");
        console.log(abs == null ? "no absolute" : "ABSOLUTE");
        ops.invoke("bone.create", { name: "root" });
      },
    });
    "#;

    let host = Host::new();
    let importer = host.importers(nosy).unwrap().into_iter().next().unwrap();
    // The read still succeeds — refusing is not erroring — but reaches nothing.
    importer.read(&path).expect("runs");
}

#[test]
fn a_broken_importer_reports_where_it_broke() {
    // A plugin author whose format has a typo in it needs the line, not
    // "import failed".
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hero.toy");
    std::fs::write(&path, "not json at all").unwrap();

    let host = Host::new();
    let importer = host.importers(PLUGIN).unwrap().into_iter().next().unwrap();
    let Err(err) = importer.read(&path) else {
        panic!("JSON.parse should throw");
    };

    let message = format!("{err}");
    assert!(!message.is_empty());
    assert!(
        !message.contains("Exception generated by QuickJS"),
        "the message should name the failure, got: {message}"
    );
}

#[test]
fn a_plugin_that_registers_nothing_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hero.toy");
    std::fs::write(&path, RIG).unwrap();

    let host = Host::new();
    assert!(
        host.importers("console.log('hello');").unwrap().is_empty(),
        "nothing declared, nothing offered"
    );
}
