//! Print what our reader sees in a PSD.
//!
//! For checking a fixture against what a test is about to assert, rather than
//! writing assertions from what the file was *meant* to contain. Every importer
//! bug in this repo was found by looking at real output.
//!
//! ```text
//! cargo run -p ankhimate-formats --example inspect_psd -- <file.psd>
//! ```

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: inspect_psd <file.psd>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&path).expect("read the psd");

    let nodes = match ankhimate_formats::psd::layer_tree(&bytes) {
        Ok(nodes) => nodes,
        Err(e) => {
            eprintln!("could not read it: {e}");
            std::process::exit(1);
        }
    };

    println!("{} layers and groups\n", nodes.len());
    for node in &nodes {
        let tags = ankhimate_formats::psd_tags::Tags::parse(&node.name);
        let kind = if node.is_group { "group" } else { "layer" };
        let (x, y, w, h) = node.bounds;
        let listed: Vec<&str> = tags.names().collect();
        println!(
            "{:indent$}{kind} {:20} {:>4},{:<4} {:>4}x{:<4} {}{}",
            "",
            tags.name,
            x,
            y,
            w,
            h,
            if listed.is_empty() {
                String::new()
            } else {
                format!("tags {listed:?}")
            },
            if node.visible { "" } else { "  (hidden)" },
            indent = node.depth * 2,
        );
    }

    // What inference makes of it, which is the half a test cannot read off the
    // layer names.
    let candidates: Vec<ankhimate_formats::psd_infer::Candidate> = nodes
        .iter()
        .map(|n| ankhimate_formats::psd_infer::Candidate {
            path: n.path.clone(),
            name: n.name.clone(),
            depth: n.depth,
            is_group: n.is_group,
            bounds: n.bounds,
        })
        .collect();
    let tags: Vec<ankhimate_formats::psd_tags::Tags> = nodes
        .iter()
        .map(|n| ankhimate_formats::psd_tags::Tags::parse(&n.name))
        .collect();

    let mut guesses = Vec::new();
    let inferred = ankhimate_formats::psd_infer::infer(&candidates, &tags, &mut guesses);

    println!("\n{} guesses", guesses.len());
    for guess in &guesses {
        println!("  {}", guess.decided);
        println!("    because {}", guess.because);
        println!("    say otherwise with {}", guess.override_with);
    }

    let sequences: Vec<_> = inferred
        .iter()
        .filter_map(|i| i.sequence.as_ref())
        .collect();
    println!("\n{} sequences", sequences.len());
    for sequence in sequences {
        println!("  {} — {:?}", sequence.stem, sequence.frames);
    }

    let mirrors: Vec<_> = inferred
        .iter()
        .enumerate()
        .filter_map(|(i, inf)| inf.mirrors.as_ref().map(|m| (&candidates[i].path, m)))
        .collect();
    println!("\n{} mirror pairs", mirrors.len());
    for (a, b) in mirrors {
        println!("  {a} ↔ {b}");
    }

    // What the importer makes of all that, which is the half the guesses above
    // cannot show: a guess nothing acts on is a comment.
    let options = ankhimate_formats::psd::ImportOptions::default();
    match ankhimate_formats::psd::import(&bytes, &options) {
        Ok(import) => {
            let s = &import.summary;
            println!(
                "
imported {} bones, {} slots, {} images, {} skins",
                s.bones, s.slots, s.images, s.skins
            );
            for (stem, frames) in &s.sequences {
                println!("  sequence {stem} — {frames} frames in one slot");
            }
            for (path, tag) in &s.unknown_tags {
                println!("  unknown tag [{tag}] on {path}");
            }
            for skipped in &s.skipped {
                println!("  skipped {skipped}");
            }
            for (_, bone) in import.skeleton.bones.iter() {
                println!("  bone {}", bone.name);
            }
        }
        Err(e) => println!(
            "
import failed: {e}"
        ),
    }
}
