//! First-party importer registration and the open registry seam.

use ankhimate_formats::{ImportError, Importer, Importers};
use std::path::Path;

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

#[test]
fn only_first_party_formats_are_built_in() {
    let importers = Importers::builtin();
    let ids: Vec<&str> = importers.ids().collect();
    assert!(ids.contains(&"import.ankh"), "{ids:?}");
    assert!(ids.contains(&"import.psd"), "{ids:?}");
    assert!(!ids.contains(&"import.spine"), "{ids:?}");
    assert!(!ids.contains(&"import.dragonbones"), "{ids:?}");
}

#[test]
fn ids_are_dotted_labels_are_human_and_extensions_exist() {
    for importer in Importers::builtin().iter() {
        assert!(importer.id().contains('.'), "{}", importer.id());
        assert!(!importer.label().contains('.'), "{}", importer.label());
        assert!(!importer.extensions().is_empty());
    }
}

#[test]
fn an_extension_nobody_claims_narrows_to_nothing() {
    assert_eq!(
        Importers::builtin()
            .claiming(Path::new("rig.blend"))
            .count(),
        0
    );
}

#[test]
fn a_community_importer_registers_through_the_same_door() {
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
    assert_eq!(importers.claiming(Path::new("x.pretend")).count(), 1);
}

#[test]
fn psd_is_registered_with_options_and_its_own_extension() {
    let importers = Importers::builtin();
    let psd = importers.get("import.psd").expect("registered");
    assert_eq!(psd.extensions(), ["psd"]);
    assert_eq!(importers.claiming(Path::new("art.psd")).count(), 1);
    let schema = psd.options_schema();
    assert!(schema["properties"]["scale"].is_object(), "{schema}");
    assert!(schema["properties"]["skip_hidden"].is_object());
}

#[test]
fn malformed_psd_is_declined_for_the_next_importer() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "notes.psd", "this is not a psd at all");
    let importers = Importers::builtin();
    let Err(error) = importers.get("import.psd").unwrap().read(&path) else {
        panic!("text must not read as PSD");
    };
    assert!(matches!(error, ImportError::NotThisFormat), "{error}");
}

#[test]
fn unknown_json_is_not_mangled_by_a_first_party_importer() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "notes.json", r#"{"hello":"world"}"#);
    assert!(Importers::builtin().read_any(&path).is_err());
}

#[test]
fn psd_provenance_survives_a_save_and_reopen() {
    use ankhimate_core::{
        animation::Animation, assets::AssetDb, ids::AnimationId, skeleton::Skeleton,
        slotmap::SlotMap,
    };
    let skeleton = Skeleton::new();
    let animations: SlotMap<AnimationId, Animation> = SlotMap::with_key();
    let assets = AssetDb::new();
    let paths = std::collections::HashMap::from([("arm".to_string(), "torso/arm".to_string())]);
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
        Some("torso/arm")
    );
}

#[test]
fn a_rig_that_never_saw_a_psd_omits_provenance() {
    use ankhimate_core::{
        animation::Animation, assets::AssetDb, ids::AnimationId, skeleton::Skeleton,
        slotmap::SlotMap,
    };
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
    assert!(!json.contains("psd_layer_paths"));
}
