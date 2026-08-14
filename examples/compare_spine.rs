//! Compare a Spine export's setup pose against `core`'s own evaluation.
//!
//! Diffing an export field-by-field against a reference file only works when the
//! reference describes the same rig state. When it does not — a richer authoring
//! session, a different Spine version — every section can match and the render
//! can still be wrong, with nothing to point at.
//!
//! `core::evaluate()` has no such problem: it is what the editor draws, so it is
//! the ground truth for what the rig *means*. This walks the exported skeleton
//! the way a consumer does and reports the bones whose world placement disagrees.
//!
//! ```text
//! cargo run -p ankhimate-export --example compare_spine -- samples/spineboy.ankh out/skeleton.json
//! ```

use ankhimate_core::math::Transform;
use ankhimate_core::pose::{self, Pose};
use ankhimate_core::transforms::Affine2;
use ankhimate_formats::convert::ProjectRef;
use glam::Vec2;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let rig = args.next().unwrap_or_else(|| usage());
    let exported = args.next().unwrap_or_else(|| usage());

    let (loaded, _) = ankhimate_formats::load(Path::new(&rig))
        .unwrap_or_else(|e| panic!("could not read '{rig}': {e}"));
    let _ = ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
    };

    // Setup pose: no animation applied, which is what an importer shows first.
    let mut posed = Pose::default();
    pose::evaluate(&loaded.skeleton, &[], &mut posed);

    let text = std::fs::read_to_string(&exported)
        .unwrap_or_else(|e| panic!("could not read '{exported}': {e}"));
    let doc: Value = serde_json::from_str(&text).expect("the export is valid JSON");

    // Rebuild each bone's world transform from the exported file, exactly as a
    // consumer does: local transform composed onto the parent's world.
    let bones = doc["bones"].as_array().expect("bones is an array");
    let mut worlds: HashMap<String, Affine2> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for b in bones {
        let name = b["name"].as_str().expect("a bone has a name").to_string();
        let local = Transform {
            position: Vec2::new(num(b, "x", 0.0), num(b, "y", 0.0)),
            rotation: num(b, "rotation", 0.0).to_radians(),
            scale: Vec2::new(num(b, "scaleX", 1.0), num(b, "scaleY", 1.0)),
            shear: Vec2::new(
                num(b, "shearX", 0.0).to_radians(),
                num(b, "shearY", 0.0).to_radians(),
            ),
        };
        let world = match b["parent"].as_str() {
            Some(parent) => {
                let p = worlds
                    .get(parent)
                    .unwrap_or_else(|| panic!("'{name}' is declared before its parent '{parent}'"));
                p.mul(&local.to_affine())
            }
            None => local.to_affine(),
        };
        worlds.insert(name.clone(), world);
        order.push(name);
    }

    let mut worst: Vec<(f32, String, [f32; 2], [f32; 2])> = Vec::new();
    for (id, bone) in loaded.skeleton.bones.iter() {
        let Some(theirs) = worlds.get(&bone.name) else {
            println!("MISSING from the export: {}", bone.name);
            continue;
        };
        let Some(ours) = posed.worlds.get(id) else {
            continue;
        };
        let a = [ours.tx, ours.ty];
        let b = [theirs.tx, theirs.ty];
        let drift = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        worst.push((drift, bone.name.clone(), a, b));
    }

    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("a distance is never NaN"));
    let off = worst.iter().filter(|(d, ..)| *d > 0.5).count();
    println!(
        "{off} of {} bones land somewhere else than core puts them\n",
        worst.len()
    );
    for (drift, name, ours, theirs) in worst.iter().take(15) {
        println!(
            "{name:24} drift {drift:9.2}   core ({:8.2},{:8.2})   export ({:8.2},{:8.2})",
            ours[0], ours[1], theirs[0], theirs[1]
        );
    }
}

fn num(v: &Value, key: &str, fallback: f32) -> f32 {
    v[key].as_f64().map(|n| n as f32).unwrap_or(fallback)
}

fn usage() -> ! {
    eprintln!("usage: compare_spine <rig.ankh> <exported-skeleton.json>");
    std::process::exit(2)
}
