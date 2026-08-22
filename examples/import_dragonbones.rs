//! Read a DragonBones rig and report what came across.
//!
//! The check a unit test cannot make: real files, with the shapes their authors
//! actually wrote rather than the ones a test author imagined. Every importer
//! bug found in this repo so far was found this way.
//!
//! ```text
//! cargo run -p ankhimate-formats --example import_dragonbones -- <dir> [out.ankh]
//! ```
//!
//! `<dir>` holds `<name>_ske.json` and optionally `<name>_tex.json` +
//! `<name>_tex.png`.

use ankhimate_formats::dragonbones::{self, Images};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: import_dragonbones <dir> [out.ankh]");
        std::process::exit(2);
    };
    let out = args.next().map(PathBuf::from);

    let Some(ske) = find(&dir, "_ske.json") else {
        eprintln!("no *_ske.json in {}", dir.display());
        std::process::exit(1);
    };
    let stem = ske
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix("_ske.json"))
        .unwrap_or("rig")
        .to_string();

    let json = std::fs::read_to_string(&ske).expect("read skeleton");
    println!("{}", ske.display());
    if let Some(v) = dragonbones::declared_version(&json) {
        println!("  declares version {v}");
    }

    let tex_json = dir.join(format!("{stem}_tex.json"));
    let atlas_text = std::fs::read_to_string(&tex_json).ok();
    let dir_for_pages = dir.clone();
    let pages = move |file: &str| -> Option<image::RgbaImage> {
        image::open(dir_for_pages.join(file))
            .ok()
            .map(|i| i.to_rgba8())
    };

    let images = match &atlas_text {
        Some(text) => {
            println!("  atlas {}", tex_json.display());
            Images::Atlas {
                text,
                pages: &pages,
            }
        }
        None => {
            println!("  no atlas — geometry only");
            Images::None
        }
    };

    let loaded = match dragonbones::read(&json, images, &stem) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("  failed: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "\n  {} bones, {} slots, {} assets, {} animations, {} fps",
        loaded.skeleton.bones.len(),
        loaded.skeleton.slots.len(),
        loaded.assets.len(),
        loaded.animations.len(),
        loaded.fps,
    );

    let timelines: usize = loaded.animations.values().map(|a| a.timelines.len()).sum();
    let keys: usize = loaded
        .animations
        .values()
        .flat_map(|a| &a.timelines)
        .map(count_keys)
        .sum();
    println!("  {timelines} timelines, {keys} keys");

    // The longest clips, since a rig with 489 of them is unreadable in full.
    let mut by_length: Vec<_> = loaded
        .animations
        .values()
        .map(|a| (a.timelines.len(), a.name.as_str(), a.duration))
        .collect();
    by_length.sort_by_key(|(timelines, ..)| std::cmp::Reverse(*timelines));
    for (tl, name, duration) in by_length.iter().take(5) {
        println!("    {name:24} {duration:6.2}s  {tl} timelines");
    }

    if !loaded.report.dangling.is_empty() {
        println!("\n  unresolved:");
        let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        for (kind, _) in &loaded.report.dangling {
            *by_kind.entry(kind).or_default() += 1;
        }
        for (kind, n) in by_kind {
            println!("    {kind}: {n}");
        }
    }

    if !loaded.report.lossy.is_empty() {
        println!("\n  approximated:");
        let mut by_kind: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for l in &loaded.report.lossy {
            by_kind.entry(l.what).or_default().push(&l.where_);
        }
        for (kind, mut wheres) in by_kind {
            wheres.sort_unstable();
            wheres.dedup();
            println!("    {kind}: {}", wheres.len());
            for w in wheres.iter().take(4) {
                println!("      {w}");
            }
            if wheres.len() > 4 {
                println!("      … and {} more", wheres.len() - 4);
            }
        }
    }

    if let Some(out) = out {
        // `save` builds the image blobs from the asset db itself, so there is
        // nothing extra to hand it.
        let project = ankhimate_formats::convert::ProjectRef {
            skeleton: &loaded.skeleton,
            animations: &loaded.animations,
            assets: &loaded.assets,
            name: &loaded.name,
            fps: loaded.fps,
            export_presets: &loaded.export_presets,
            psd_layer_paths: &Default::default(),
        };
        match ankhimate_formats::save(&out, &project, &[]) {
            Ok(()) => println!("\n  wrote {}", out.display()),
            Err(e) => eprintln!("\n  save failed: {e}"),
        }
    }
}

fn count_keys(t: &ankhimate_core::animation::Timeline) -> usize {
    use ankhimate_core::animation::Timeline::*;
    match t {
        BoneTranslate { keys, .. } | BoneScale { keys, .. } | BoneShear { keys, .. } => keys.len(),
        BoneRotate { keys, .. } => keys.len(),
        _ => 0,
    }
}

fn find(dir: &Path, suffix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(suffix))
        })
}
