//! Print the world position of named bones at one frame of one animation.
//!
//! For comparing an import against the editor it came from: a screenshot shows
//! that two rigs differ, and this says by how much and where.

use ankhimate_formats::dragonbones::{self, Images};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: pose_at <dir> <anim> <frame>"));
    let anim_name = args.next().unwrap_or_else(|| "walk".into());
    let frame: f32 = args.next().unwrap_or_else(|| "10".into()).parse().unwrap();

    let ske = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.to_str().is_some_and(|s| s.ends_with("_ske.json")))
        .expect("a _ske.json");
    let json = std::fs::read_to_string(&ske).unwrap();
    let loaded = dragonbones::read(&json, Images::None, "rig").expect("reads");

    let anim = loaded
        .animations
        .values()
        .find(|a| a.name == anim_name)
        .expect("named animation");
    let t = frame / loaded.fps as f32;

    let mut pose = ankhimate_core::pose::Pose::new();
    ankhimate_core::pose::evaluate(&loaded.skeleton, &[(anim, t, 1.0)], &mut pose);

    println!("{anim_name} at frame {frame} ({t:.3}s), fps {}", loaded.fps);
    for (id, bone) in loaded.skeleton.bones.iter() {
        let w = pose.world(id);
        println!(
            "  {:16} world ({:9.2}, {:9.2})  rot {:8.2}",
            bone.name,
            w.tx,
            w.ty,
            w.decompose().rotation.to_degrees()
        );
    }
}
