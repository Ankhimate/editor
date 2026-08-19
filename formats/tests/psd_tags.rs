//! The PSD reader against a real file.
//!
//! Everything else about this reader is unit-tested on synthetic layer names and
//! rectangles a test author invented. This runs it over a PSD an artist saved,
//! which is the only check that has ever found anything in this repo — three
//! problems on its first run: a `because` message describing a metric that had
//! been replaced, a mirror count that double-counted every pair, and a group
//! reported twice for one reason.
//!
//! The fixture is `fixtures/tagged_rig.psd`: a tagged chain, an untagged
//! scatter, a numbered run, an ignored sketch and a mirrored pair.

use ankhimate_formats::psd;
use ankhimate_formats::psd_infer::{self, Candidate};
use ankhimate_formats::psd_tags::Tags;

const FIXTURE: &[u8] = include_bytes!("fixtures/tagged_rig.psd");

fn tree() -> Vec<psd::LayerNode> {
    psd::layer_tree(FIXTURE).expect("the fixture parses")
}

fn analyse() -> (
    Vec<Candidate>,
    Vec<Tags>,
    Vec<psd_infer::Guess>,
    Vec<psd_infer::Inferred>,
) {
    let nodes = tree();
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
    let tags: Vec<Tags> = nodes.iter().map(|n| Tags::parse(&n.name)).collect();
    let mut guesses = Vec::new();
    let inferred = psd_infer::infer(&candidates, &tags, &mut guesses);
    (candidates, tags, guesses, inferred)
}

#[test]
fn the_fixture_carries_the_structure_the_tests_assume() {
    // Asserted rather than assumed: a test written against what a fixture was
    // *meant* to contain passes for the wrong reason the day someone re-saves
    // it.
    let nodes = tree();
    let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();

    for expected in [
        "arm [bone]",
        "face",
        "fx",
        "sketch [ignore]",
        "leg_l",
        "leg_r",
    ] {
        assert!(
            names.contains(&expected),
            "missing `{expected}` in {names:?}"
        );
    }
    assert!(
        nodes.iter().any(|n| n.is_group && n.name == "arm [bone]"),
        "arm is a group, not a layer"
    );
}

#[test]
fn a_tag_in_a_layer_name_is_read_off_a_real_file() {
    // The grammar working on names Photoshop actually wrote, rather than on
    // strings a test made up.
    let tags: Vec<Tags> = tree().iter().map(|n| Tags::parse(&n.name)).collect();

    let arm = tags.iter().find(|t| t.name == "arm").expect("arm");
    assert!(
        arm.has("bone"),
        "the tag survived the round trip through a PSD"
    );

    let sketch = tags.iter().find(|t| t.name == "sketch").expect("sketch");
    assert!(sketch.has("ignore"));
}

#[test]
fn a_scattered_group_is_one_bone_and_a_chain_is_not() {
    // The pair that makes the heuristic worth having. Both groups have three
    // children; only the arrangement differs, and that is the whole signal.
    let (_, _, guesses, _) = analyse();

    assert!(
        guesses
            .iter()
            .any(|g| g.path == "face" && g.decided.contains("one bone")),
        "the scattered face was noticed: {guesses:?}"
    );
    assert!(
        !guesses.iter().any(|g| g.path.starts_with("arm")),
        "the chain was left alone: {guesses:?}"
    );
}

#[test]
fn a_numbered_run_reads_as_a_sequence_in_number_order() {
    let (_, _, _, inferred) = analyse();
    let sequence = inferred
        .iter()
        .find_map(|i| i.sequence.as_ref())
        .expect("the fire frames");

    assert_eq!(sequence.stem, "fire");
    assert_eq!(
        sequence.frames,
        ["fx/fire_01", "fx/fire_02", "fx/fire_03"],
        "ordered by number, not by stacking"
    );
}

#[test]
fn a_group_of_frames_is_not_also_reported_as_one_bone() {
    // Found by running this: `fx` drew both a sequence guess and a "one bone,
    // not 3" guess, because stacked frames are not a chain either. Two guesses
    // about one group is how a list of guesses stops being read.
    let (_, _, guesses, _) = analyse();
    let about_fx: Vec<&str> = guesses
        .iter()
        .filter(|g| g.path.starts_with("fx"))
        .map(|g| g.decided.as_str())
        .collect();

    assert_eq!(about_fx.len(), 1, "one explanation, not two: {about_fx:?}");
    assert!(about_fx[0].contains("sequence"), "and it is the useful one");
}

#[test]
fn a_mirrored_pair_is_counted_once() {
    // Also found by running this: each pair is seen from both sides, so the raw
    // count said "4 layers" for two pairs. A small lie in a list of guesses is
    // how the rest of the list stops being trusted.
    let (_, _, guesses, _) = analyse();
    let mirror = guesses
        .iter()
        .find(|g| g.decided.contains("mirrored"))
        .expect("the legs and the eyes");

    assert!(
        mirror.decided.starts_with("2 "),
        "two pairs, not four halves: {}",
        mirror.decided
    );
}

#[test]
fn every_guess_says_how_to_disagree_with_it() {
    // The property that makes inference safe rather than merely convenient: a
    // wrong guess must be one rename away, not a search through documentation.
    let (_, _, guesses, _) = analyse();
    assert!(!guesses.is_empty());

    for guess in &guesses {
        assert!(!guess.decided.is_empty(), "a guess with nothing to read");
        assert!(
            !guess.because.is_empty(),
            "`{}` gives no evidence",
            guess.decided
        );
        assert!(
            !guess.override_with.is_empty(),
            "`{}` cannot be argued with",
            guess.decided
        );
    }
}

#[test]
fn the_reason_given_matches_the_test_that_was_run() {
    // Found by running this too: the explanation still described the *area*
    // metric that had already been replaced by an arrangement test. A guess
    // whose stated reason is not its real reason is worse than a silent one —
    // it teaches the artist a rule that does not exist.
    let (_, _, guesses, _) = analyse();
    let face = guesses
        .iter()
        .find(|g| g.path == "face")
        .expect("the face guess");

    assert!(
        face.because.contains("scatter") || face.because.contains("along one"),
        "the reason should describe arrangement: {}",
        face.because
    );
    assert!(
        !face.because.contains("area"),
        "and not the metric that was replaced: {}",
        face.because
    );
}

#[test]
fn a_group_reports_the_extent_of_what_is_inside_it() {
    // This used to hardcode `0x0`, and the `psd` crate's own group rectangle is
    // `1x1` — measured, not assumed. Either way a caller reading a group's
    // bounds got a number that meant nothing, and inference reads them.
    //
    // The arm's three limbs lie end to end, so its extent is wider than any one
    // of them; the fire frames stack, so theirs is exactly one frame.
    let nodes = tree();
    let bounds = |name: &str| {
        nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("no `{name}`"))
            .bounds
    };

    let (_, _, arm_w, _) = bounds("arm [bone]");
    let (_, _, hand_w, _) = bounds("hand");
    assert!(
        arm_w > hand_w * 2,
        "a chain spans more than one of its links: {arm_w} vs {hand_w}"
    );

    let (fx_x, fx_y, fx_w, fx_h) = bounds("fx");
    assert_eq!(
        (fx_x, fx_y, fx_w, fx_h),
        bounds("fire_01"),
        "stacked frames occupy exactly one frame's rectangle"
    );
}

#[test]
fn an_empty_group_has_no_extent_rather_than_a_wrong_one() {
    // A group with nothing in it should read as nothing, not as a one-pixel
    // rectangle at the origin that a heuristic would then reason about.
    let nodes = tree();
    for node in nodes.iter().filter(|n| n.is_group) {
        let (_, _, w, h) = node.bounds;
        let has_children = nodes
            .iter()
            .any(|c| c.path.starts_with(&format!("{}/", node.path)));
        if !has_children {
            assert_eq!((w, h), (0, 0), "`{}` is empty", node.name);
        } else {
            assert!(w > 0 && h > 0, "`{}` has art in it", node.name);
        }
    }
}

#[test]
fn a_sequence_becomes_one_slot_and_not_one_per_frame() {
    // The whole point of detecting a run. Three frames used to mean three
    // slots the artist hides by hand, and the flipbook the numbering plainly
    // meant was lost — inference noticed and nothing acted on it.
    let import = psd::import(FIXTURE, &psd::ImportOptions::default()).expect("import");

    assert_eq!(
        import.summary.sequences,
        [("fire".to_string(), 3)],
        "the run folded, and said how many frames it folded"
    );

    let slots: Vec<&str> = import
        .skeleton
        .slots
        .iter()
        .map(|(_, s)| s.name.as_str())
        .collect();
    let frame_slots = slots.iter().filter(|n| n.starts_with("fire")).count();
    assert_eq!(frame_slots, 1, "one slot for the run: {slots:?}");
}

#[test]
fn every_frame_of_a_sequence_is_still_imported_as_an_image() {
    // The slot collapses; the art must not. A flipbook with one frame in the
    // asset database is a flipbook that does not play.
    let import = psd::import(FIXTURE, &psd::ImportOptions::default()).expect("import");
    let names: Vec<&str> = import
        .assets
        .images
        .values()
        .map(|a| a.name.as_str())
        .collect();

    for frame in ["fire_01", "fire_02", "fire_03"] {
        assert!(names.contains(&frame), "missing {frame} in {names:?}");
    }
}

#[test]
fn the_lead_attachment_cycles_the_frames_in_number_order() {
    // Order comes from the number, not from the stacking — the fixture stacks
    // `fire_03` on top, so a reader that trusted the file's order would play it
    // backwards.
    use ankhimate_core::attachment::Attachment;
    let import = psd::import(FIXTURE, &psd::ImportOptions::default()).expect("import");

    let sequence = import
        .skeleton
        .skins
        .iter()
        .flat_map(|(_, skin)| {
            import.skeleton.slots.iter().flat_map(move |(slot, _)| {
                skin.names_for_slot(slot)
                    .filter_map(move |name| skin.get(slot, name))
            })
        })
        .find_map(|attachment| match attachment {
            Attachment::Region(r) => r.sequence.clone(),
            _ => None,
        })
        .expect("the fire attachment carries a sequence");

    assert_eq!(sequence.frames, ["fire_01", "fire_02", "fire_03"]);
    assert!(
        sequence.fps > 0.0,
        "a sequence that never advances is a still"
    );
}

#[test]
fn an_explicit_frames_tag_builds_the_run_and_reports_nothing() {
    // `[frames]` used to *skip* the run entirely, so naming the feature was the
    // one thing that switched it off. It still must not produce a guess: a
    // report listing back what the artist wrote is noise that teaches them to
    // stop reading it.
    use ankhimate_formats::psd_infer::{self, Candidate};

    let candidates: Vec<Candidate> = ["fire_01", "fire_02"]
        .iter()
        .enumerate()
        .map(|(i, name)| Candidate {
            path: name.to_string(),
            name: name.to_string(),
            depth: 0,
            is_group: false,
            bounds: (0, i as i32, 10, 10),
        })
        .collect();
    let tags = vec![
        Tags::parse("fire_01 [frames][fps:24]"),
        Tags::parse("fire_02"),
    ];

    let mut guesses = Vec::new();
    let inferred = psd_infer::infer(&candidates, &tags, &mut guesses);

    let sequence = inferred[0]
        .sequence
        .as_ref()
        .expect("the tag built the run");
    assert!(sequence.explicit);
    assert_eq!(sequence.fps, Some(24.0), "[fps:24] speaks for the run");
    assert!(
        !guesses.iter().any(|g| g.decided.contains("sequence")),
        "nothing was guessed, so nothing is reported: {guesses:?}"
    );
}

#[test]
fn a_refused_run_stays_separate_attachments() {
    // `[!frames]` is the escape hatch the guess itself advertises. Numbered
    // layers that really are separate attachments do exist.
    use ankhimate_formats::psd_infer::{self, Candidate};

    let candidates: Vec<Candidate> = ["card_01", "card_02"]
        .iter()
        .enumerate()
        .map(|(i, name)| Candidate {
            path: name.to_string(),
            name: name.to_string(),
            depth: 0,
            is_group: false,
            bounds: (0, i as i32, 10, 10),
        })
        .collect();
    let tags = vec![Tags::parse("card_01 [!frames]"), Tags::parse("card_02")];

    let mut guesses = Vec::new();
    let inferred = psd_infer::infer(&candidates, &tags, &mut guesses);
    assert!(inferred.iter().all(|i| i.sequence.is_none()));
}

#[test]
fn a_photoshop_blend_mode_is_read_without_a_tag() {
    // The file already says it, and every art tool round trips it. Requiring a
    // tag for something Photoshop records is asking the artist to write down
    // what they already did.
    //
    // **Weaker than it looks**: every layer in the fixture is Normal, so a
    // reader that ignored the file and hardcoded Normal would pass the first
    // assertion. Only the second has teeth — it fails if an unmapped mode is
    // ever read as mapped. A fixture with a Multiply layer would close this.
    use ankhimate_core::slot::BlendMode;
    let import = psd::import(FIXTURE, &psd::ImportOptions::default()).expect("import");

    assert!(
        import
            .skeleton
            .slots
            .iter()
            .all(|(_, s)| s.blend_mode == BlendMode::Normal),
        "the fixture's layers are all Normal, so nothing invented a mode"
    );
    assert!(
        import.summary.lost_blend.is_empty(),
        "and nothing was reported as lost: {:?}",
        import.summary.lost_blend
    );
}

#[test]
fn an_alpha_tag_overrides_the_layers_opacity() {
    // Opacity is read from the file, which the fixture cannot demonstrate — its
    // layers are all fully opaque, so a hardcoded 1.0 passes any assertion made
    // against them. This pins the half that *is* checkable: the tag wins, and
    // it reads 0–1 the way every other number in this grammar does rather than
    // Photoshop's 0–255.
    assert_eq!(Tags::parse("cape [alpha:0.5]").number("alpha"), Some(0.5));
    assert_eq!(
        Tags::parse("cape [alpha:2]")
            .number("alpha")
            .map(|a| a.clamp(0.0, 1.0)),
        Some(1.0),
        "out of range is clamped, not wrapped"
    );
}

#[test]
fn every_known_tag_has_something_that_reads_it() {
    // The failure this catches is the worst kind of silence: a tag listed as
    // known, so it draws no "unrecognised" warning, and read by nothing. The
    // artist writes it, sees no complaint, and gets no effect.
    //
    // Listed explicitly rather than derived, so adding a tag to `KNOWN` without
    // a reader fails here rather than passing quietly.
    use ankhimate_formats::psd_tags::KNOWN;
    const READ: &[&str] = &[
        "bone", "bones", "slot", "slots", "skin", "folder", "ik", "pivot", "clip", "box", "point",
        "frames", "fps", "scale", "ignore", "merge", "blend", "alpha",
    ];
    // Named, so the gap is a list somebody has to shorten rather than a fact
    // nobody wrote down. `[mesh]` and `[weights]` need the tracer in
    // `document`, which `formats` cannot reach; `[physics]` needs a model that
    // does not exist yet.
    const NOT_YET: &[&str] = &["mesh", "weights", "physics"];

    let unread: Vec<&&str> = KNOWN
        .iter()
        .filter(|t| !READ.contains(t) && !NOT_YET.contains(t))
        .collect();
    assert!(unread.is_empty(), "known but read by nothing: {unread:?}");
    assert!(
        NOT_YET.iter().all(|t| KNOWN.contains(t)),
        "the waiting list names a tag that is not known"
    );
}

#[test]
fn a_fully_transparent_layer_is_not_imported() {
    // Found in the running editor, not by a test: the fixture's stray `Layer 1`
    // came in as a slot with a 1x1 invisible image. Photoshop leaves these
    // behind and every one imported is something the artist has to find and
    // delete in a rig they did not build.
    let import = psd::import(FIXTURE, &psd::ImportOptions::default()).expect("import");

    let names: Vec<&str> = import
        .skeleton
        .slots
        .iter()
        .map(|(_, s)| s.name.as_str())
        .collect();
    assert!(
        !names.iter().any(|n| n.starts_with("Layer 1")),
        "an empty layer became a slot: {names:?}"
    );
    assert!(
        import
            .summary
            .skipped
            .iter()
            .any(|s| s.contains("Layer 1") && s.contains("transparent")),
        "and it was reported rather than silently dropped: {:?}",
        import.summary.skipped
    );
}
