//! Export an `.ankh` through a shipped preset, headlessly.
//!
//! For checking a preset against a **real rig** rather than a test fixture: the
//! Spine preset shipped broken twice because the fixture had no track shape that
//! triggered the defect. A rig on disk does.
//!
//! ```text
//! cargo run -p ankhimate-export --example export_once -- samples/spineboy.ankh out/ "Spine JSON"
//! ```

use ankhimate_export::{presets, run};
use ankhimate_formats::convert::ProjectRef;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or_else(|| usage());
    let output = args.next().unwrap_or_else(|| usage());
    let wanted = args.next().unwrap_or_else(|| "Spine JSON".into());

    let (loaded, _) = ankhimate_formats::load(Path::new(&input))
        .unwrap_or_else(|e| panic!("could not read '{input}': {e}"));

    let project = ankhimate_formats::convert::to_schema(&ProjectRef {
        skeleton: &loaded.skeleton,
        animations: &loaded.animations,
        assets: &loaded.assets,
        name: &loaded.name,
        fps: loaded.fps,
        export_presets: &loaded.export_presets,
        psd_layer_paths: &Default::default(),
    });

    let preset = presets::builtin()
        .into_iter()
        .find(|p| p.name.starts_with(&wanted))
        .unwrap_or_else(|| panic!("no shipped preset whose name starts with '{wanted}'"));

    match run::export(&project, &loaded.assets, &preset, Path::new(&output)) {
        Ok(plan) => {
            println!(
                "wrote {} file(s) and {} image(s) to {output}",
                plan.files.len(),
                plan.binaries.len()
            );
            for path in plan.paths() {
                println!("  {path}");
            }
        }
        Err(e) => {
            eprintln!("export failed: {e}");
            std::process::exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!("usage: export_once <input.ankh> <output-dir> [preset name]");
    std::process::exit(2);
}
