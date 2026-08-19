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
