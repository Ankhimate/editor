//! Shipped community format plugins exercised as ordinary JavaScript files.

use ankhimate_formats::Importer;
use ankhimate_plugins::Host;

const SPINE: &str = include_str!("../../community-plugins/spine/plugin.js");
const DRAGONBONES: &str = include_str!("../../community-plugins/dragonbones/plugin.js");
const TWEEGEE_ITEM: &str = include_str!("../../community-plugins/tweegee-item/plugin.js");

fn spine_host() -> Host {
    Host::new().with_resources(std::collections::BTreeMap::from([(
        "spine_json.json".to_string(),
        include_bytes!("../../community-plugins/spine/spine_json.json").to_vec(),
    )]))
}

#[test]
fn community_packages_restore_the_external_format_registries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../community-plugins");
    let plugins = ankhimate_plugins::discovery::load(&root);
    assert_eq!(plugins.len(), 3, "{root:?}");
    assert!(plugins.iter().all(|plugin| plugin.is_loaded()));

    let importer_ids: Vec<&str> = plugins
        .iter()
        .flat_map(|plugin| plugin.importers.iter())
        .map(|importer| importer.id.as_str())
        .collect();
    let exporter_ids: Vec<&str> = plugins
        .iter()
        .flat_map(|plugin| plugin.exporters.iter())
        .map(|exporter| exporter.id.as_str())
        .collect();
    assert!(importer_ids.contains(&"import.spine"), "{importer_ids:?}");
    assert!(
        importer_ids.contains(&"import.dragonbones"),
        "{importer_ids:?}"
    );
    assert!(importer_ids.contains(&"import.twitem"), "{importer_ids:?}");
    assert!(exporter_ids.contains(&"export.spine"), "{exporter_ids:?}");
    let twitem = plugins
        .iter()
        .flat_map(|plugin| &plugin.importers)
        .find(|importer| importer.id == "import.twitem")
        .expect("Tweegee Item importer");
    assert_eq!(ankhimate_formats::Importer::extensions(twitem), ["twitem"]);
    assert!(twitem.binary);
}

#[test]
fn tweegee_item_plugin_reads_a_stored_package() {
    fn entry(name: &str, contents: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // versions and flags
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&[0; 8]); // time, date, crc
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(contents);
    }

    let manifest = br#"{"version":1,"itemId":"hat","assetScale":2,"targets":[{"name":"front","bone":"avatar_head.item_front","transform":{"a":-1,"d":1,"tx":0,"ty":0},"layers":[{"kind":"merged","file":"front.png","width":179,"height":11,"pivotX":0.25,"pivotY":0.75}]}]}"#;
    let mut package = Vec::new();
    entry("item.json", manifest, &mut package);
    entry("images/front.png", &[1, 2, 3], &mut package);
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("hat.twitem");
    std::fs::write(&path, package).expect("fixture");

    let importer = Host::new()
        .importers(TWEEGEE_ITEM)
        .expect("plugin loads")
        .remove(0);
    let loaded = Importer::read(&importer, &path).expect("item imports");

    assert_eq!(
        loaded.skeleton.bones.len(),
        2,
        "the dotted target becomes a chain"
    );
    assert_eq!(loaded.assets.images.len(), 1);
    let image = loaded.assets.images.values().next().expect("image asset");
    assert_eq!(
        (image.width, image.height),
        (179, 11),
        "pixels stay integral"
    );
    let attachment = loaded
        .skeleton
        .skins
        .values()
        .flat_map(|skin| skin.entries.values())
        .next()
        .expect("attachment");
    let ankhimate_core::attachment::Attachment::Region(region) = attachment else {
        panic!("region attachment expected");
    };
    assert_eq!(region.local_scale.x, 1.0);
    assert_eq!((region.width, region.height), (89.5, 5.5));
    assert_eq!(
        region.local_scale.y, -1.0,
        "reflection survives matrix decomposition"
    );
}

fn spine_import(json: &str) -> ankhimate_formats::Loaded {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("rig.json");
    std::fs::write(&path, json).expect("fixture");
    let importer = spine_host()
        .importers(SPINE)
        .expect("plugin loads")
        .into_iter()
        .find(|importer| importer.id == "import.spine")
        .expect("Spine importer registered");
    Importer::read(&importer, &path).expect("plugin imports")
}

fn dragonbones_import(json: &str) -> Result<ankhimate_formats::Loaded, String> {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = dir.path().join("rig_ske.json");
    std::fs::write(&path, json).map_err(|error| error.to_string())?;
    let importer = Host::new()
        .importers(DRAGONBONES)
        .map_err(|error| error.to_string())?
        .remove(0);
    Importer::read(&importer, &path).map_err(|error| error.to_string())
}

#[test]
fn spine_plugin_preserves_constraint_channels_and_per_axis_curves() {
    let loaded = spine_import(
        r#"{
          "skeleton": { "spine": "4.3.23" },
          "bones": [{ "name": "root" }, { "name": "arm", "parent": "root" }],
          "constraints": [{
            "type": "transform", "name": "follow", "source": "root", "bones": ["arm"],
            "mixRotate": 0.1, "mixX": 0.2, "mixY": 0.3,
            "mixScaleX": 0.4, "mixScaleY": 0.5, "mixShearY": 0.6
          }],
          "animations": { "walk": { "bones": { "arm": { "translate": [
            { "x": 0, "y": 0, "curve": [0.1,0,0.2,0, 0.1,0,0.25,8] },
            { "time": 0.5, "x": 0, "y": 8 }
          ] } } } }
        }"#,
    );

    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
        psd_layer_paths: &loaded.psd_layer_paths,
    })
    .expect("serializes");
    let project: serde_json::Value = serde_json::from_str(&json).expect("json");
    let mix = &project["constraints"][0]["transform_mix"];
    assert_eq!(mix["translate_x"], 0.2);
    assert_eq!(mix["translate_y"], 0.3);
    assert_eq!(mix["scale_y"], 0.5);
    assert_eq!(mix["shear_y"], 0.6);

    let timelines = project["animations"][0]["timelines"]
        .as_array()
        .expect("timelines");
    let x = timelines
        .iter()
        .find(|timeline| timeline["kind"] == "bone_translate" && timeline["axis"] == "x")
        .expect("x track");
    let y = timelines
        .iter()
        .find(|timeline| timeline["kind"] == "bone_translate" && timeline["axis"] == "y")
        .expect("y track");
    assert_eq!(x["keys"][1]["handles"][3], 0.0);
    assert!(y["keys"][1]["handles"][3].as_f64().unwrap_or(0.0) > 0.5);
}

#[test]
fn spine_plugin_names_unsupported_constraints() {
    let loaded = spine_import(
        r#"{
          "skeleton": { "spine": "4.3.23" },
          "bones": [{ "name": "root" }],
          "constraints": [
            { "type": "path", "name": "rail", "bones": ["root"] },
            { "type": "physics", "name": "wobble", "bone": "root" }
          ]
        }"#,
    );
    let names: Vec<&str> = loaded
        .report
        .lossy
        .iter()
        .map(|loss| loss.where_.as_str())
        .collect();
    assert!(names.contains(&"rail"), "{names:?}");
    assert!(names.contains(&"wobble"), "{names:?}");
}

#[test]
fn spine_plugin_converts_weighted_meshes_and_keeps_deform_streams() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("rig.json");
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([255, 255, 255, 255]),
    ))
    .save(dir.path().join("mesh.png"))
    .expect("image");
    std::fs::write(
        &path,
        r#"{
          "skeleton": { "spine": "4.3.23" },
          "bones": [{ "name": "root" }, { "name": "child", "parent": "root", "x": 10 }],
          "slots": [{ "name": "shape", "bone": "root", "attachment": "mesh" }],
          "skins": [{ "name": "default", "attachments": { "shape": { "mesh": {
            "type": "mesh", "width": 1, "height": 1,
            "uvs": [0,0, 1,0, 0,1], "triangles": [0,1,2],
            "vertices": [1,0,0,0,1, 1,1,0,0,1, 1,0,0,10,1]
          }}}}],
          "animations": { "bend": { "attachments": { "default": { "shape": { "mesh": {
            "deform": [{ "time": 0.5, "vertices": [1,2,3,4,5,6] }]
          }}}}}}
        }"#,
    )
    .expect("fixture");
    let importer = spine_host()
        .importers(SPINE)
        .expect("plugin loads")
        .remove(0);
    let loaded = Importer::read(&importer, &path).expect("imports");
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
        psd_layer_paths: &loaded.psd_layer_paths,
    })
    .expect("serializes");
    let project: serde_json::Value = serde_json::from_str(&json).expect("json");
    let mesh = &project["skins"][0]["entries"][0]["attachment"];
    assert_eq!(
        mesh["vertices"],
        serde_json::json!([0.0, 0.0, 10.0, 0.0, 0.0, 10.0])
    );
    assert_eq!(mesh["weights"][1][0][0], "child");
    let deform = project["animations"][0]["timelines"]
        .as_array()
        .expect("timelines")
        .iter()
        .find(|timeline| timeline["kind"] == "deform")
        .expect("deform");
    assert_eq!(
        deform["keys"][0]["offsets"],
        serde_json::json!([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
    );
}

#[test]
fn dragonbones_plugin_converts_axes_duration_and_slot_switches() {
    let dir = tempfile::tempdir().expect("temp");
    let path = dir.path().join("hero_ske.json");
    std::fs::write(
        &path,
        r#"{
          "name": "hero", "frameRate": 10,
          "armature": [{ "name": "rig", "frameRate": 10,
            "bone": [
              { "name": "root" },
              { "name": "arm", "parent": "root", "transform": { "x": 3, "y": 4, "skX": 30, "skY": 20 } }
            ],
            "slot": [{ "name": "hand", "parent": "arm", "displayIndex": -1 }],
            "skin": [{ "name": "default", "slot": [{ "name": "hand", "display": [] }] }],
            "animation": [{ "name": "wave", "duration": 5, "bone": [{ "name": "arm",
              "translateFrame": [{ "duration": 2, "x": 0, "y": 0, "tweenEasing": 0 },
                                 { "duration": 3, "x": 5, "y": 7 }],
              "rotateFrame": [{ "duration": 2, "rotate": 0 }, { "duration": 3, "rotate": 30 }]
            }] }]
          }]
        }"#,
    )
    .expect("fixture");
    let importer = Host::new()
        .importers(DRAGONBONES)
        .expect("plugin loads")
        .remove(0);
    let loaded = Importer::read(&importer, &path).expect("imports");
    assert_eq!(loaded.name, "hero");
    assert_eq!(loaded.fps, 10);
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
        psd_layer_paths: &loaded.psd_layer_paths,
    })
    .expect("serializes");
    let project: serde_json::Value = serde_json::from_str(&json).expect("json");
    let arm = &project["bones"][1];
    assert_eq!(arm["ty"], -4.0);
    assert_eq!(arm["rotation"], -20.0);
    assert_eq!(arm["shear_y"], -10.0);
    assert_eq!(project["animations"][0]["duration"], 0.5);
    let timelines = project["animations"][0]["timelines"].as_array().unwrap();
    let y = timelines
        .iter()
        .find(|timeline| timeline["kind"] == "bone_translate" && timeline["axis"] == "y")
        .expect("y timeline");
    assert_eq!(y["keys"][1]["time"], 0.2);
    assert_eq!(y["keys"][1]["value"], -7.0);
}

#[test]
fn dragonbones_50_combined_frames_become_real_timelines() {
    let loaded = dragonbones_import(
        r#"{
          "version":"5.0", "name":"legacy", "frameRate":10,
          "armature":[{
            "name":"rig",
            "bone":[{"name":"root"},{"name":"arm","parent":"root"}],
            "slot":[
              {"name":"body","parent":"root","displayIndex":-1},
              {"name":"glow","parent":"arm","displayIndex":-1}
            ],
            "skin":[{"name":"default","slot":[
              {"name":"body","display":[]},{"name":"glow","display":[]}
            ]}],
            "animation":[{
              "name":"cast", "duration":10, "playTimes":0,
              "bone":[{"name":"arm","frame":[
                {"duration":4,"tweenEasing":0,"transform":{}},
                {"duration":6,"transform":{"x":5,"y":7,"skX":30,"skY":30,"scX":2,"scY":0.5}}
              ]}],
              "slot":[{"name":"glow","frame":[
                {"duration":4,"displayIndex":-1,"color":{"aM":0}},
                {"duration":6,"displayIndex":-1,"color":{"aM":100}}
              ]}],
              "zOrder":{"frame":[
                {"duration":4,"zOrder":[]},
                {"duration":6,"zOrder":[1,-1]}
              ]}
            }]
          }]
        }"#,
    )
    .expect("DragonBones 5.0 imports");
    let project = project_value(&loaded);
    let animation = &project["animations"][0];
    assert_eq!(animation["duration"], 1.0);
    assert_eq!(animation["looping"], true);
    let timelines = animation["timelines"].as_array().expect("timelines");

    let find = |kind: &str, axis: Option<&str>| {
        timelines
            .iter()
            .find(|timeline| {
                timeline["kind"] == kind && axis.is_none_or(|axis| timeline["axis"] == axis)
            })
            .unwrap_or_else(|| panic!("missing {kind} {axis:?}: {timelines:?}"))
    };
    assert_eq!(find("bone_translate", Some("x"))["keys"][1]["value"], 5.0);
    assert_eq!(find("bone_translate", Some("y"))["keys"][1]["value"], -7.0);
    assert_eq!(find("bone_rotate", None)["keys"][1]["value"], -30.0);
    assert_eq!(find("bone_scale", Some("x"))["keys"][1]["value"], 2.0);
    assert_eq!(find("bone_scale", Some("y"))["keys"][1]["value"], 0.5);
    let color = find("slot_color", None);
    assert_eq!(color["keys"][0]["value"][3], 0.0);
    assert_eq!(color["keys"][1]["time"], 0.4);
    assert_eq!(color["keys"][1]["value"][3], 1.0);
    let draw_order = find("draw_order", None);
    assert_eq!(draw_order["keys"][0]["offsets"], serde_json::json!([]));
    assert_eq!(draw_order["keys"][1]["time"], 0.4);
    assert_eq!(
        draw_order["keys"][1]["offsets"],
        serde_json::json!([["glow", -1]])
    );
}

#[test]
fn dragonbones_plugin_folds_referenced_sprite_armatures_into_sequences() {
    let dir = tempfile::tempdir().expect("temp");
    for name in ["flash1", "flash2"] {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            3,
            image::Rgba([255, 255, 255, 255]),
        ))
        .save(dir.path().join(format!("{name}.png")))
        .expect("image");
    }
    let path = dir.path().join("effect_ske.json");
    std::fs::write(
        &path,
        r#"{ "name": "effect", "frameRate": 12, "armature": [
          { "name": "host", "bone": [{ "name": "root" }],
            "slot": [{ "name": "fx", "parent": "root", "displayIndex": 0 }],
            "skin": [{ "name": "default", "slot": [{ "name": "fx", "display": [
              { "name": "flash", "type": "armature" }
            ] }] }]
          },
          { "name": "flash", "frameRate": 12,
            "bone": [{ "name": "sprite", "transform": { "x": 100.5, "y": 7 } }],
            "skin": [{ "name": "default", "slot": [{ "name": "frames", "display": [
              { "name": "flash1", "type": "image" },
              { "name": "flash2", "type": "image" }
            ] }] }]
          }
        ] }"#,
    )
    .expect("fixture");
    let importer = Host::new()
        .importers(DRAGONBONES)
        .expect("plugin loads")
        .remove(0);
    let loaded = Importer::read(&importer, &path).expect("imports");
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
        psd_layer_paths: &loaded.psd_layer_paths,
    })
    .expect("serializes");
    let project: serde_json::Value = serde_json::from_str(&json).expect("json");
    let attachment = &project["skins"][0]["entries"][0]["attachment"];
    assert_eq!(
        attachment["sequence"]["frames"],
        serde_json::json!(["flash1", "flash2"])
    );
    assert_eq!(attachment["width"], 2.0);
    assert_eq!(attachment["height"], 3.0);
    assert_eq!(attachment["offset_x"], 100.5);
    assert_eq!(attachment["offset_y"], -7.0);
    assert!(
        !loaded
            .report
            .lossy
            .iter()
            .any(|loss| loss.what == "armature")
    );
}

#[test]
fn dragonbones_plugin_keeps_mesh_geometry_whole_and_reports_lost_weights() {
    let dir = tempfile::tempdir().expect("temp");
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([255, 255, 255, 255]),
    ))
    .save(dir.path().join("face.png"))
    .expect("image");
    let path = dir.path().join("mesh_ske.json");
    std::fs::write(
        &path,
        r#"{ "name":"mesh", "armature":[{"name":"rig",
          "bone":[{"name":"root"}], "slot":[{"name":"face","parent":"root"}],
          "skin":[{"slot":[{"name":"face","display":[{
            "type":"mesh", "name":"face", "vertices":[0,0,10,0,10,-20,0,-20],
            "uvs":[0,0,1,0,1,1,0,1], "triangles":[0,1,2,0,2,3], "weights":[1]
          }]}]}]
        }] }"#,
    )
    .unwrap();
    let importer = Host::new().importers(DRAGONBONES).unwrap().remove(0);
    let loaded = Importer::read(&importer, &path).expect("imports");
    let project = project_value(&loaded);
    let mesh = &project["skins"][0]["entries"][0]["attachment"];
    assert_eq!(
        mesh["vertices"],
        serde_json::json!([0.0, 0.0, 10.0, 0.0, 10.0, 20.0, 0.0, 20.0])
    );
    assert_eq!(
        mesh["uvs"],
        serde_json::json!([0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0])
    );
    assert_eq!(mesh["triangles"].as_array().unwrap().len(), 6);
    assert!(
        loaded
            .report
            .lossy
            .iter()
            .any(|loss| loss.detail.contains("without its weights"))
    );
}

#[test]
fn dragonbones_plugin_pins_defaults_timing_and_visibility() {
    assert!(dragonbones_import(r#"{"name":"not a rig"}"#).is_err());
    let loaded = dragonbones_import(
        r#"{ "name":"defaults", "frameRate":20, "armature":[{
          "name":"rig", "bone":[{"name":"root"},{"name":"plain","parent":"root"}],
          "slot":[{"name":"shown","parent":"root","displayIndex":1},
                  {"name":"hidden","parent":"root","displayIndex":-1}],
          "skin":[{"slot":[
            {"name":"shown","display":[{"name":"a"},{"name":"b"}]},
            {"name":"hidden","display":[{"name":"gone"}]}
          ]}],
          "animation":[{"name":"timing","duration":4,"bone":[{"name":"plain",
            "translateFrame":[{"x":0,"y":0,"tweenEasing":0},{"duration":0,"x":2,"y":3}]
          }]}]
        }] }"#,
    )
    .expect("imports");
    let project = project_value(&loaded);
    let plain = project["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bone| bone["name"] == "plain")
        .unwrap();
    assert_eq!(plain["tx"], 0.0);
    assert_eq!(plain["ty"], 0.0);
    assert_eq!(project["animations"][0]["duration"], 0.2);
    let slots = project["slots"].as_array().unwrap();
    assert_eq!(slots[0]["attachment"], "b");
    assert!(slots[1]["attachment"].is_null());
}

#[test]
fn dragonbones_plugin_reports_unread_data_and_builds_ik_root_first() {
    let loaded = dragonbones_import(
        r#"{ "name":"reported", "armature":[
          {"name":"host", "bone":[{"name":"root"},{"name":"upper","parent":"root"},
             {"name":"lower","parent":"upper"},{"name":"target"}],
           "ik":[{"name":"leg","bone":"lower","target":"target","chain":1,"weight":50}],
           "animation":[{"name":"odd","timeline":[{"type":99}]}]},
          {"name":"unused", "bone":[{"name":"other"}]}
        ] }"#,
    )
    .expect("imports");
    let project = project_value(&loaded);
    assert_eq!(
        project["constraints"][0]["bones"],
        serde_json::json!(["upper", "lower"])
    );
    assert_eq!(project["constraints"][0]["mix"], 0.5);
    let kinds: Vec<&str> = loaded.report.lossy.iter().map(|loss| loss.what).collect();
    assert!(kinds.contains(&"timeline"), "{kinds:?}");
    assert!(kinds.contains(&"armature"), "{kinds:?}");
}

#[test]
fn two_json_community_plugins_decline_each_others_files() {
    let dir = tempfile::tempdir().expect("temp");
    let dragon_path = dir.path().join("dragon_ske.json");
    std::fs::write(
        &dragon_path,
        r#"{ "armature": [{ "name": "rig", "bone": [{ "name": "root" }] }] }"#,
    )
    .expect("fixture");
    let spine_path = dir.path().join("spine.json");
    std::fs::write(
        &spine_path,
        r#"{ "skeleton": { "spine": "4.3" }, "bones": [{ "name": "root" }] }"#,
    )
    .expect("fixture");

    let mut registry = ankhimate_formats::Importers::new();
    for importer in Host::new().importers(DRAGONBONES).expect("dragon plugin") {
        registry.register(Box::new(importer));
    }
    for importer in spine_host().importers(SPINE).expect("spine plugin") {
        registry.register(Box::new(importer));
    }
    assert_eq!(
        registry.read_any(&dragon_path).expect("dragon").0,
        "import.dragonbones"
    );
    assert_eq!(
        registry.read_any(&spine_path).expect("spine").0,
        "import.spine"
    );
}

#[test]
fn spine_community_exporter_uses_its_packaged_preset() {
    let exporter = spine_host()
        .exporters(SPINE)
        .expect("plugin loads")
        .into_iter()
        .find(|exporter| exporter.id == "export.spine")
        .expect("exporter");
    let document = ankhimate_document::Document::new();
    let (plan, _) = exporter.plan(document).expect("preset renders");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, "untitled.json");
    assert!(plan.files[0].contents.contains("\"skeleton\""));
}

fn project_value(loaded: &ankhimate_formats::Loaded) -> serde_json::Value {
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
        psd_layer_paths: &loaded.psd_layer_paths,
    })
    .expect("serializes");
    let mut value = serde_json::from_str(&json).expect("json");
    normalize_signed_zero(&mut value);
    value
}

fn normalize_signed_zero(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number) if number.as_f64() == Some(0.0) => {
            *number = serde_json::Number::from_f64(0.0).unwrap();
        }
        serde_json::Value::Array(values) => {
            for value in values {
                normalize_signed_zero(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                normalize_signed_zero(value);
            }
        }
        _ => {}
    }
}

#[test]
fn spine_plugin_preserves_a_representative_whole_project() {
    let json = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }, { "name": "arm", "parent": "root", "x": 10, "rotation": 20 }],
      "slots": [{ "name": "hand", "bone": "arm" }],
      "constraints": [
        { "type": "ik", "name": "reach", "target": "root", "bones": ["arm"], "mix": 0.7 },
        { "type": "transform", "name": "follow", "source": "root", "bones": ["arm"], "mixX": 0.4 }
      ],
      "animations": { "move": {
        "bones": { "arm": {
          "rotate": [{ "value": 0, "curve": [0.1,-5,0.4,25] }, { "time": 0.5, "value": 30 }],
          "translate": [{ "x": 0, "y": 0 }, { "time": 0.5, "x": 4, "y": 8 }]
        }},
        "ik": { "reach": [{ "mix": 0.7 }, { "time": 0.5, "mix": 1 }] }
      }}
    }"#;
    let plugin = spine_import(json);
    let project = project_value(&plugin);
    assert_eq!(project["bones"].as_array().unwrap().len(), 2);
    assert_eq!(project["slots"].as_array().unwrap().len(), 1);
    assert_eq!(project["constraints"].as_array().unwrap().len(), 2);
    assert_eq!(project["animations"][0]["duration"], 0.5);
    assert_eq!(
        project["animations"][0]["timelines"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn dragonbones_plugin_preserves_a_representative_whole_project() {
    let json = r#"{
      "name": "hero", "frameRate": 10,
      "armature": [{ "name": "rig", "frameRate": 10,
        "bone": [{ "name": "root" }, { "name": "arm", "parent": "root",
          "transform": { "x": 3, "y": 4, "skX": 30, "skY": 20 } }],
        "slot": [{ "name": "hand", "parent": "arm", "displayIndex": -1 }],
        "skin": [{ "name": "default", "slot": [{ "name": "hand", "display": [] }] }],
        "ik": [{ "name": "reach", "bone": "arm", "target": "root", "chain": 0, "weight": 75 }],
        "animation": [{ "name": "wave", "duration": 5,
          "bone": [{ "name": "arm",
            "translateFrame": [{ "duration": 2, "x": 0, "y": 0, "tweenEasing": 0 },
                               { "duration": 3, "x": 5, "y": 7 }],
            "rotateFrame": [{ "duration": 2, "rotate": 0 }, { "duration": 3, "rotate": 30 }]
          }]
        }]
      }]
    }"#;
    let plugin = dragonbones_import(json).expect("plugin");
    let project = project_value(&plugin);
    assert_eq!(project["name"], "hero");
    assert_eq!(project["bones"].as_array().unwrap().len(), 2);
    assert_eq!(project["slots"].as_array().unwrap().len(), 1);
    assert_eq!(project["constraints"].as_array().unwrap().len(), 1);
    assert_eq!(project["animations"][0]["duration"], 0.5);
    assert_eq!(
        project["animations"][0]["timelines"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
