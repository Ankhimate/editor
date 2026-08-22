//! The foreign-format importers shipped as plugins, against their public
//! registry boundary rather than their parser implementation.

#![cfg(all(feature = "import-spine", feature = "import-dragonbones"))]

use ankhimate_formats::Importers;
use std::path::Path;

const SPINE: &str = r#"{
    "skeleton": { "spine": "4.2.00" },
    "bones": [{ "name": "root" }, { "name": "arm", "parent": "root", "length": 30 }],
    "slots": [],
    "animations": {}
}"#;

const DRAGONBONES: &str = r#"{
    "name": "golem", "frameRate": 24, "version": "5.5",
    "armature": [{
        "name": "golem",
        "bone": [{ "name": "root" }, { "name": "arm", "parent": "root", "length": 30 }],
        "slot": [], "skin": [], "animation": []
    }]
}"#;

fn importers() -> Importers {
    let mut importers = Importers::new();
    ankhimate_plugins::bundled::register_importers(&mut importers);
    importers
}

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture");
    path
}

#[test]
fn bundled_plugins_register_stable_public_ids() {
    let importers = importers();
    let ids: Vec<&str> = importers.ids().collect();
    assert_eq!(ids, ["import.dragonbones", "import.spine"]);
}

#[test]
fn spine_and_dragonbones_still_disambiguate_json() {
    let dir = tempfile::tempdir().unwrap();
    let spine_path = write(dir.path(), "hero.json", SPINE);
    let dragon_path = write(dir.path(), "golem_ske.json", DRAGONBONES);
    let importers = importers();

    let (spine_id, spine) = importers
        .read_any(&spine_path)
        .expect("Spine plugin reads it");
    let (dragon_id, dragon) = importers
        .read_any(&dragon_path)
        .expect("DragonBones plugin reads it");

    assert_eq!(spine_id, "import.spine");
    assert_eq!(spine.skeleton.bones.len(), 2);
    assert_eq!(dragon_id, "import.dragonbones");
    assert_eq!(dragon.name, "golem");
    assert_eq!(dragon.skeleton.bones.len(), 2);
}

#[test]
fn unrelated_json_is_not_mangled() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "notes.json", r#"{"hello":"world"}"#);
    assert!(importers().read_any(&path).is_err());
}

#[test]
fn plugin_importers_keep_version_reporting() {
    let dir = tempfile::tempdir().unwrap();
    let spine_path = write(dir.path(), "hero.json", SPINE);
    let dragon_path = write(dir.path(), "golem_ske.json", DRAGONBONES);
    let importers = importers();

    assert_eq!(
        importers
            .get("import.spine")
            .unwrap()
            .declared_version(&spine_path)
            .as_deref(),
        Some("4.2.00")
    );
    assert_eq!(
        importers
            .get("import.dragonbones")
            .unwrap()
            .declared_version(&dragon_path)
            .as_deref(),
        Some("5.5")
    );
}
