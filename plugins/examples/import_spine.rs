//! Import a Spine export into `.ankh`, headlessly.
//!
//! ```text
//! cargo run -p ankhimate-plugins --example import_spine -- <dir> <out.ankh>
//! ```
//!
//! `dir` holds the skeleton `.json`, and either an `.atlas` with its page images
//! or an `images/` directory of loose PNGs. Both layouts ship in the wild — a
//! rig exported for a runtime has an atlas, one exported for re-editing usually
//! does not.

use ankhimate_formats::convert::ProjectRef;
use ankhimate_plugins::bundled::spine::{self, Images};
use std::path::{Path, PathBuf};

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| usage()));
    let output = args.next().unwrap_or_else(|| usage());

    let find = |ext: &str| -> Option<PathBuf> {
        std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
    };

    let skeleton = find("json").unwrap_or_else(|| panic!("no .json in {}", dir.display()));
    let json = std::fs::read_to_string(&skeleton).expect("the skeleton file is readable");
    if let Some(v) = spine::declared_version(&json) {
        println!("Spine {v}");
    }

    // An atlas if there is one, loose images otherwise.
    let atlas_text = find("atlas").and_then(|p| std::fs::read_to_string(p).ok());
    let open_page = |file: &str| image::open(dir.join(file)).ok().map(|i| i.to_rgba8());
    let images_dir = dir.join("images");
    let open_loose = |name: &str| {
        // Spine writes an attachment path with `/` for a subdirectory and no
        // extension, which is exactly how the files sit on disk.
        image::open(images_dir.join(format!("{name}.png")))
            .ok()
            .map(|i| i.to_rgba8())
    };
    let images = match &atlas_text {
        Some(text) => Images::Atlas {
            text,
            pages: &open_page,
        },
        None if images_dir.is_dir() => Images::Loose(&open_loose),
        None => Images::None,
    };

    let stem = skeleton
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let loaded = match spine::read(&json, images, stem) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("could not read '{}': {e}", skeleton.display());
            std::process::exit(1);
        }
    };

    println!(
        "{} bones, {} slots, {} constraints, {} animations, {} images",
        loaded.skeleton.bones.len(),
        loaded.skeleton.slots.len(),
        loaded.skeleton.constraints.len(),
        loaded.animations.len(),
        loaded.assets.images.len(),
    );

    // What could not be carried across. An import that reports nothing on a
    // real rig is more likely not looking than lossless.
    if !loaded.report.dangling.is_empty() {
        println!("unresolved ({}):", loaded.report.dangling.len());
        for (what, name) in loaded.report.dangling.iter().take(10) {
            println!("  {what}: {name}");
        }
    }
    let summary = loaded.report.lossy_summary();
    if !summary.is_empty() {
        println!("approximated:");
        for (what, count) in &summary {
            println!("  {what}: {count}");
        }
        for l in loaded.report.lossy.iter().take(5) {
            println!("  e.g. {} — {}", l.where_, l.detail);
        }
    }

    ankhimate_formats::save(
        Path::new(&output),
        &ProjectRef {
            skeleton: &loaded.skeleton,
            animations: &loaded.animations,
            assets: &loaded.assets,
            name: &loaded.name,
            fps: loaded.fps,
            export_presets: &[],
            psd_layer_paths: &Default::default(),
        },
        // The bytes already live on each asset, which is where `save` reads
        // them from; this list is for images arriving beside a project rather
        // than inside it.
        &[],
    )
    .unwrap_or_else(|e| panic!("could not write '{output}': {e}"));
    println!("wrote {output}");
}

fn usage() -> ! {
    eprintln!("usage: import_spine <dir> <out.ankh>");
    std::process::exit(2)
}
