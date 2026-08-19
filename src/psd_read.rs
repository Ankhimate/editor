//! Reading a layered document's *structure*, for anyone — including a plugin.
//!
//! Three things live behind the PSD importer that are not about PSD at all:
//!
//! * the **tag grammar** (`psd_tags`) — `[bone]`, `[frames]`, `[physics:cloth]`
//!   — a vocabulary for saying what a layer means, written in the one field
//!   every art tool round-trips;
//! * **inference** (`psd_infer`) — is this group a chain or a scatter, is this
//!   run of numbered layers a flipbook, which layers mirror which;
//! * and the **layer tree** itself.
//!
//! Only the third is Photoshop-specific. A plugin importing a layered TIFF, an
//! Aseprite file or a directory of numbered PNGs wants the same vocabulary and
//! the same questions answered, and without this it would reimplement both —
//! in JavaScript, and differently from us, so `[bones]` would mean one thing in
//! the built-in importer and another in an addon.
//!
//! Everything here is **JSON in, JSON out**. That is the shape the rest of the
//! plugin surface already has (`ops.invoke`, `rig()`, `bakeAtlas`), and it keeps
//! the contract readable in one place rather than spread across a binding.
//!
//! # A public contract
//!
//! These field names are a rename away from breaking every plugin that reads
//! them, with no compiler on that side — the same rule `docs/export-context.md`
//! states for the template context. Treat a change here as a breaking change.

use crate::psd_infer::{self, Candidate};
use crate::psd_tags::{self, Tags};
use serde::{Deserialize, Serialize};

/// One layer or group, with its tags read and its inference attached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Slash-joined path from the document root, e.g. `torso/arm/hand`.
    pub path: String,
    /// The name **with tags stripped** — what the bone or slot is called.
    ///
    /// An artist writing `arm [bone][slot:upper]` means a thing called `arm`,
    /// and a plugin should not have to know the grammar to find that out.
    pub name: String,
    /// The name exactly as the file spells it, tags and all.
    pub raw_name: String,
    pub depth: usize,
    pub is_group: bool,
    pub visible: bool,
    /// `[left, top, width, height]` in pixels.
    pub bounds: [i64; 4],
    /// Tags on this layer: `{"bone": null, "slot": "upper"}`. A bare tag is
    /// `null` rather than absent, so "present with no value" and "not present"
    /// stay distinguishable.
    pub tags: serde_json::Map<String, serde_json::Value>,
    /// Tags this build does not recognise. A plugin may still act on them — it
    /// is the one consumer that can define new ones.
    pub unknown_tags: Vec<String>,
    /// Should this layer get a bone of its own, once inference has spoken?
    pub bone: bool,
    /// The run of frames this layer heads, if any.
    pub sequence: Option<Sequence>,
    /// The path of the layer this one mirrors, if any.
    pub mirrors: Option<String>,
}

/// A run of layers that are frames of one drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    /// The name without its number — `fire_01` and `fire_02` share `fire`.
    pub stem: String,
    /// Layer paths, in frame order.
    pub frames: Vec<String>,
    /// From `[fps:n]`, or absent to take the document's rate.
    pub fps: Option<f32>,
    /// True when a `[frames]` tag said so rather than the numbering implying it.
    pub explicit: bool,
}

/// Something the reader decided that the file did not spell out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guess {
    pub path: String,
    pub decided: String,
    pub because: String,
    pub override_with: String,
}

/// What a layered document says about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Structure {
    pub layers: Vec<Layer>,
    /// Every guess, also reachable per-layer. Repeated here because a plugin
    /// showing the artist what was decided wants the list, not a walk.
    pub guesses: Vec<Guess>,
}

/// Read a PSD's structure: layers, tags and inference, without importing it.
pub fn read_psd(bytes: &[u8]) -> Result<Structure, crate::psd::PsdError> {
    let nodes = crate::psd::layer_tree(bytes)?;
    let tags: Vec<Tags> = nodes.iter().map(|n| Tags::parse(&n.name)).collect();
    let candidates: Vec<Candidate> = nodes
        .iter()
        .map(|n| Candidate {
            path: n.path.clone(),
            name: n.name.clone(),
            depth: n.depth,
            is_group: n.is_group,
            bounds: n.bounds,
        })
        .collect();

    let mut raw_guesses = Vec::new();
    let inferred = psd_infer::infer(&candidates, &tags, &mut raw_guesses);

    let layers = nodes
        .iter()
        .zip(&tags)
        .zip(&inferred)
        .map(|((node, tags), inferred)| Layer {
            path: node.path.clone(),
            name: tags.name.clone(),
            raw_name: node.name.clone(),
            depth: node.depth,
            is_group: node.is_group,
            visible: node.visible,
            bounds: [
                node.bounds.0 as i64,
                node.bounds.1 as i64,
                node.bounds.2 as i64,
                node.bounds.3 as i64,
            ],
            tags: tag_map(tags),
            unknown_tags: unknown_of(tags),
            bone: inferred.bone,
            sequence: inferred.sequence.as_ref().map(as_sequence),
            mirrors: inferred.mirrors.clone(),
        })
        .collect();

    Ok(Structure {
        layers,
        guesses: raw_guesses.into_iter().map(as_guess).collect(),
    })
}

/// Read the tag grammar out of one name, for a plugin importing something that
/// is not a PSD.
///
/// The vocabulary is the reusable part. An Aseprite importer wants `[bone]` to
/// mean what it means here, not what its author guessed it meant.
pub fn parse_tags(raw: &str) -> Layer {
    let tags = Tags::parse(raw);
    Layer {
        path: tags.name.clone(),
        name: tags.name.clone(),
        raw_name: raw.to_string(),
        depth: 0,
        is_group: false,
        visible: true,
        bounds: [0; 4],
        unknown_tags: unknown_of(&tags),
        tags: tag_map(&tags),
        // Nothing has been inferred about a lone name: this is the grammar
        // alone. Defaulting `bone` to true matches what inference starts from,
        // so a plugin that parses names and then calls `infer` sees one story.
        bone: true,
        sequence: None,
        mirrors: None,
    }
}

/// Run inference over a layer list a plugin built itself.
///
/// The second reusable half: "is this group a chain or a scatter", "is this a
/// flipbook", "which of these mirror each other" are questions about a layer
/// tree, not about Photoshop. A plugin reading a directory of numbered PNGs can
/// hand them here rather than answering them again, and differently.
///
/// Takes what [`read_psd`] returns, so a plugin can filter or synthesise layers
/// and ask again. `raw_name` is what the grammar is read from — a caller
/// building layers by hand puts the tags there, not in `name`.
pub fn infer(layers: &[Layer]) -> Structure {
    let tags: Vec<Tags> = layers.iter().map(|l| Tags::parse(&l.raw_name)).collect();
    let candidates: Vec<Candidate> = layers
        .iter()
        .map(|l| Candidate {
            path: l.path.clone(),
            name: l.raw_name.clone(),
            depth: l.depth,
            is_group: l.is_group,
            bounds: (
                l.bounds[0] as i32,
                l.bounds[1] as i32,
                l.bounds[2] as u32,
                l.bounds[3] as u32,
            ),
        })
        .collect();

    let mut raw_guesses = Vec::new();
    let inferred = psd_infer::infer(&candidates, &tags, &mut raw_guesses);

    let layers = layers
        .iter()
        .zip(&tags)
        .zip(&inferred)
        .map(|((layer, tags), inferred)| Layer {
            // The grammar is re-read too, so a caller that edited `raw_name`
            // between calls gets the tags it now says rather than the ones it
            // used to.
            name: tags.name.clone(),
            tags: tag_map(tags),
            unknown_tags: unknown_of(tags),
            bone: inferred.bone,
            sequence: inferred.sequence.as_ref().map(as_sequence),
            mirrors: inferred.mirrors.clone(),
            ..layer.clone()
        })
        .collect();

    Structure {
        layers,
        guesses: raw_guesses.into_iter().map(as_guess).collect(),
    }
}

fn tag_map(tags: &Tags) -> serde_json::Map<String, serde_json::Value> {
    tags.names()
        .map(|tag| {
            let value = match tags.value(tag) {
                Some(value) => serde_json::Value::String(value.to_string()),
                None => serde_json::Value::Null,
            };
            (tag.to_string(), value)
        })
        .collect()
}

fn unknown_of(tags: &Tags) -> Vec<String> {
    tags.names()
        .filter(|t| !psd_tags::KNOWN.contains(t))
        .map(str::to_string)
        .collect()
}

fn as_sequence(sequence: &psd_infer::Sequence) -> Sequence {
    Sequence {
        stem: sequence.stem.clone(),
        frames: sequence.frames.clone(),
        fps: sequence.fps,
        explicit: sequence.explicit,
    }
}

fn as_guess(guess: psd_infer::Guess) -> Guess {
    Guess {
        path: guess.path,
        decided: guess.decided,
        because: guess.because,
        override_with: guess.override_with,
    }
}
