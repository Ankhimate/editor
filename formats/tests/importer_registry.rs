//! The importer registry, against files rather than assumptions.
//!
//! Native importer registry behavior. Foreign-format readers are bundled by
//! `ankhimate-plugins` and tested there.

use ankhimate_formats::Importers;
use std::path::Path;

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

#[test]
fn built_ins_register_and_are_listed() {
    let importers = Importers::builtin();
    let ids: Vec<&str> = importers.ids().collect();
    assert!(ids.contains(&"import.ankh"), "{ids:?}");
    assert!(ids.contains(&"import.psd"), "{ids:?}");
    assert!(!ids.contains(&"import.spine"), "foreign reader leaked in");
    assert!(
        !ids.contains(&"import.dragonbones"),
        "foreign reader leaked in"
    );
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
fn an_extension_nobody_claims_narrows_to_nothing() {
    let importers = Importers::builtin();
    let path = Path::new("rig.blend");
    assert_eq!(importers.claiming(path).count(), 0);
}

#[test]
fn a_plugin_registers_through_the_same_door_a_built_in_does() {
    // The property the whole registry exists for. If this needs anything a
    // built-in does not, the door is not the same one.
    use ankhimate_formats::{ImportError, Importer};

    struct Pretend;
    impl Importer for Pretend {
        fn id(&self) -> &str {
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
    // Ankhimate takes none, and `Null` is how that reads — an empty object would
    // suggest options nobody can pass.
    let importers = Importers::builtin();
    assert!(
        importers
            .get("import.ankh")
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

#[test]
fn psd_provenance_survives_a_save_and_reopen() {
    // Without this the link is lost on save, every layer looks new on
    // re-import, and `psd::diff` cannot do the one job it exists for. It was
    // dropped by both `save` and `load` before being written to the schema.
    use ankhimate_core::animation::Animation;
    use ankhimate_core::assets::AssetDb;
    use ankhimate_core::ids::AnimationId;
    use ankhimate_core::skeleton::Skeleton;
    use ankhimate_core::slotmap::SlotMap;

    let skeleton = Skeleton::new();
    let animations: SlotMap<AnimationId, Animation> = SlotMap::with_key();
    let assets = AssetDb::new();
    let mut paths = std::collections::HashMap::new();
    paths.insert("arm".to_string(), "torso/arm".to_string());

    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &skeleton,
        animations: &animations,
        assets: &assets,
        name: "rig",
        fps: 30,
        export_presets: &[],
        psd_layer_paths: &paths,
    })
    .expect("writes");

    let reloaded = ankhimate_formats::from_json(&json).expect("reads");
    assert_eq!(
        reloaded.psd_layer_paths.get("arm").map(String::as_str),
        Some("torso/arm"),
        "the layer a redrawn asset came from"
    );
}

#[test]
fn a_rig_that_never_saw_a_psd_serialises_as_it_did_before() {
    // The field is skipped when empty, so adding it does not change the bytes
    // of every existing project.
    use ankhimate_core::animation::Animation;
    use ankhimate_core::assets::AssetDb;
    use ankhimate_core::ids::AnimationId;
    use ankhimate_core::skeleton::Skeleton;
    use ankhimate_core::slotmap::SlotMap;

    let skeleton = Skeleton::new();
    let animations: SlotMap<AnimationId, Animation> = SlotMap::with_key();
    let assets = AssetDb::new();
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &skeleton,
        animations: &animations,
        assets: &assets,
        name: "rig",
        fps: 30,
        export_presets: &[],
        psd_layer_paths: &Default::default(),
    })
    .expect("writes");

    assert!(
        !json.contains("psd_layer_paths"),
        "an empty map writes nothing"
    );
}
