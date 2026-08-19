//! The importer registry, against files rather than assumptions.
//!
//! The case that matters: both shipped readers claim `.json`, so a caller with
//! a file and no idea which format it is has to be able to find out. That is
//! what `read_any` exists for, and it is only interesting when two importers
//! disagree about the same extension.

use ankhimate_formats::Importers;
use std::path::Path;

/// A minimal Spine skeleton — enough to be recognised, not to be complete.
const SPINE: &str = r#"{
    "skeleton": { "spine": "4.2.00" },
    "bones": [{ "name": "root" }, { "name": "arm", "parent": "root", "length": 30 }],
    "slots": [],
    "animations": {}
}"#;

/// A minimal DragonBones armature.
const DRAGONBONES: &str = r#"{
    "name": "golem", "frameRate": 24, "version": "5.5",
    "armature": [{
        "name": "golem",
        "bone": [{ "name": "root" }, { "name": "arm", "parent": "root", "length": 30 }],
        "slot": [], "skin": [], "animation": []
    }]
}"#;

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

#[test]
fn both_built_ins_register_and_are_listed() {
    let importers = Importers::builtin();
    let ids: Vec<&str> = importers.ids().collect();
    assert!(ids.contains(&"import.spine"), "{ids:?}");
    assert!(ids.contains(&"import.dragonbones"), "{ids:?}");
}

#[test]
fn ids_are_dotted_and_labels_are_human() {
    // The id is what a keymap, a plugin and an MCP tool name; the label is what
    // a menu shows. Confusing the two gives a menu full of `import.spine`.
    let importers = Importers::builtin();
    for importer in importers.iter() {
        assert!(
            importer.id().contains('.'),
            "{} is not dotted",
            importer.id()
        );
        assert!(
            !importer.label().contains('.'),
            "{} looks like an id, not a label",
            importer.label()
        );
        assert!(!importer.extensions().is_empty());
    }
}

#[test]
fn a_spine_rig_is_read_by_the_spine_importer() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "hero.json", SPINE);

    let importers = Importers::builtin();
    let (id, loaded) = importers.read_any(&path).expect("some importer reads it");

    assert_eq!(id, "import.spine");
    assert_eq!(loaded.skeleton.bones.len(), 2);
    assert!(loaded.skeleton.bones.values().any(|b| b.name == "arm"));
}

#[test]
fn a_dragonbones_rig_is_read_by_the_dragonbones_importer() {
    // The point of `read_any`: this file and the one above share an extension,
    // and nothing but trying them apart tells them apart.
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "golem_ske.json", DRAGONBONES);

    let importers = Importers::builtin();
    let (id, loaded) = importers.read_any(&path).expect("some importer reads it");

    assert_eq!(id, "import.dragonbones");
    assert_eq!(loaded.skeleton.bones.len(), 2);
    assert_eq!(loaded.name, "golem", "its own name, not the file stem");
}

#[test]
fn a_json_that_is_neither_is_refused_rather_than_mangled() {
    // Every importer claims `.json`, so "no importer accepted this" has to be a
    // real answer rather than whichever one ran first pretending.
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "notes.json", r#"{"hello": "world"}"#);

    let importers = Importers::builtin();
    assert!(importers.read_any(&path).is_err());
}

#[test]
fn an_extension_nobody_claims_narrows_to_nothing() {
    let importers = Importers::builtin();
    let path = Path::new("rig.blend");
    assert_eq!(importers.claiming(path).count(), 0);
}

#[test]
fn a_named_importer_can_be_asked_for_directly() {
    // What File▸Import▸DragonBones does: the user already said which format,
    // so guessing would be worse than obeying.
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "golem_ske.json", DRAGONBONES);

    let importers = Importers::builtin();
    let importer = importers.get("import.dragonbones").expect("registered");
    let loaded = importer.read(&path).expect("reads");
    assert_eq!(loaded.name, "golem");
}

#[test]
fn a_declared_version_is_reported_when_the_file_carries_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = write(dir.path(), "golem_ske.json", DRAGONBONES);
    let spine = write(dir.path(), "hero.json", SPINE);

    let importers = Importers::builtin();
    assert_eq!(
        importers
            .get("import.dragonbones")
            .unwrap()
            .declared_version(&db)
            .as_deref(),
        Some("5.5")
    );
    assert_eq!(
        importers
            .get("import.spine")
            .unwrap()
            .declared_version(&spine)
            .as_deref(),
        Some("4.2.00")
    );
}

#[test]
fn a_plugin_registers_through_the_same_door_a_built_in_does() {
    // The property the whole registry exists for. If this needs anything a
    // built-in does not, the door is not the same one.
    use ankhimate_formats::{ImportError, Importer};

    struct Pretend;
    impl Importer for Pretend {
        fn id(&self) -> &'static str {
            "import.pretend"
        }
        fn label(&self) -> &str {
            "Pretend Format"
        }
        fn extensions(&self) -> &[&str] {
            &["pretend"]
        }
        fn read(&self, _path: &Path) -> Result<ankhimate_formats::Loaded, ImportError> {
            Err(ImportError::NotThisFormat)
        }
    }

    let mut importers = Importers::builtin();
    importers.register(Box::new(Pretend));

    assert!(importers.get("import.pretend").is_some());
    assert_eq!(
        importers.claiming(Path::new("x.pretend")).count(),
        1,
        "and it is offered for its own extension"
    );
}

#[test]
fn psd_is_registered_and_offers_its_own_extension() {
    // Layered artwork rather than a rig format, but it fits the registry
    // because its options are parameters with defaults rather than a
    // conversation — so a script can import one without a UI.
    let importers = Importers::builtin();
    let psd = importers.get("import.psd").expect("registered");

    assert_eq!(psd.extensions(), ["psd"]);
    assert_eq!(
        importers.claiming(Path::new("art.psd")).count(),
        1,
        "and nothing else claims it"
    );
}

#[test]
fn an_importer_with_options_describes_them() {
    // What a plugin author reads instead of the source, and what an MCP client
    // lists a tool from. An importer with options and no schema is one nobody
    // can drive unattended.
    let importers = Importers::builtin();
    let schema = importers.get("import.psd").unwrap().options_schema();

    assert!(schema["properties"]["scale"].is_object(), "{schema}");
    assert!(schema["properties"]["skip_hidden"].is_object());
}

#[test]
fn an_importer_without_options_says_so() {
    // Spine takes none, and `Null` is how that reads — an empty object would
    // suggest options nobody can pass.
    let importers = Importers::builtin();
    assert!(
        importers
            .get("import.spine")
            .unwrap()
            .options_schema()
            .is_null(),
        "no options is not the same as an empty set of them"
    );
}

#[test]
fn a_psd_that_will_not_parse_is_not_claimed() {
    // Every `.psd` reaching `read_any` should be tried and moved past rather
    // than reported as a broken PSD — the reader cannot tell the two apart, and
    // "not that format" is the answer that lets a caller keep looking.
    use ankhimate_formats::ImportError;

    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "notes.psd", "this is not a psd at all");

    let importers = Importers::builtin();
    let Err(err) = importers.get("import.psd").unwrap().read(&path) else {
        panic!("a text file should not read as a PSD");
    };
    assert!(matches!(err, ImportError::NotThisFormat), "{err}");
}
