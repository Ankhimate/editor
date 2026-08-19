//! What a PSD implies, when it has not been told.
//!
//! Spine's PSD import is entirely explicit: `[bone]` says "make this a bone",
//! and a group with no tag becomes nothing. Nothing is ever wrong, and nothing
//! is ever free — the floor is reading the manual.
//!
//! Ours has been entirely implicit: every group is a bone, no exceptions. Free,
//! and wrong for a face grouped for tidiness.
//!
//! Both are ends of one dial. This is the middle: read the evidence the file
//! already carries, guess, and **say what was guessed** so it can be corrected
//! in one click — after which the correction is written back as a tag and
//! travels with the artwork.
//!
//! # The rule that makes inference safe
//!
//! **A tag always wins.** Inference fills silence and never overrides. So an
//! expert who tags everything gets Spine's determinism, and a beginner who tags
//! nothing gets a rig on the first try. That is strictly more than either end
//! of the dial, rather than a different trade.
//!
//! # The second rule
//!
//! **Every guess is reported.** Inference that is usually right is worse than
//! none if it is silent — the failure mode is a rig that is subtly wrong for a
//! reason nobody can see. Each [`Guess`] carries what was decided, why, and the
//! tag that would say otherwise.

use crate::psd_tags::Tags;

/// One decision the reader made that the file did not state.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    /// Layer path the guess is about.
    pub path: String,
    /// What was decided, for a human.
    pub decided: String,
    /// The evidence. An artist should be able to check the reasoning rather
    /// than take it on trust.
    pub because: String,
    /// The tag that would have said otherwise, so a correction is one click and
    /// a rename rather than a search through documentation.
    pub override_with: String,
}

/// A layer, as inference sees it.
///
/// Deliberately not `LayerNode`: inference needs only structure and geometry,
/// and taking less means it can be tested without a PSD.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub is_group: bool,
    /// `(left, top, width, height)` in PSD pixels.
    pub bounds: (i32, i32, u32, u32),
}

/// What inference concluded about one layer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inferred {
    /// This group should be a bone.
    pub bone: bool,
    /// These layers are frames of one sprite rather than separate attachments.
    pub sequence: Option<Sequence>,
    /// This layer mirrors another, so the pair can be rigged symmetrically.
    pub mirrors: Option<String>,
}

/// A run of layers that are frames of one drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    /// The name without its number — `fire_01` and `fire_02` share `fire`.
    pub stem: String,
    /// Layer paths, in frame order.
    pub frames: Vec<String>,
}

/// Read what the file implies, for layers the tags left silent.
pub fn infer(candidates: &[Candidate], tags: &[Tags], guesses: &mut Vec<Guess>) -> Vec<Inferred> {
    let mut out = vec![Inferred::default(); candidates.len()];

    infer_bones(candidates, tags, &mut out, guesses);
    infer_sequences(candidates, tags, &mut out, guesses);
    infer_mirrors(candidates, tags, &mut out, guesses);

    out
}

/// A group is a bone unless it looks like organisation rather than articulation.
///
/// The rule this replaces — *every* group is a bone — is wrong for the case it
/// is most often applied to: a face grouped so the artist can collapse it. Eleven
/// bones nobody will ever pose.
///
/// The evidence for "organisation" is that the children overlap heavily and sit
/// inside a small area: a face's eyes, brows and mouth occupy one head-sized
/// rectangle, while an arm's upper, fore and hand lie end to end.
fn infer_bones(
    candidates: &[Candidate],
    tags: &[Tags],
    out: &mut [Inferred],
    guesses: &mut Vec<Guess>,
) {
    for (i, candidate) in candidates.iter().enumerate() {
        if !candidate.is_group {
            continue;
        }
        // A tag settles it either way. `[bones]` counts too: it says every
        // child is a bone, which is exactly the disagreement this guess offers
        // — firing anyway would tell the artist to write what they wrote.
        if tags[i].has("bone") {
            out[i].bone = true;
            continue;
        }
        if tags[i].has("bones") {
            out[i].bone = true;
            for (j, child) in candidates.iter().enumerate() {
                if is_child_of(&child.path, &candidate.path) {
                    // A child may still refuse with `[!bone]`, which `inherit`
                    // honours — the exception is the thing worth writing down.
                    out[j].bone = !tags[j].blocks("bone");
                }
            }
            continue;
        }
        if tags[i].blocks("bone") || tags[i].has("ignore") {
            continue;
        }

        let children: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| is_child_of(&c.path, &candidate.path))
            .collect();

        // A group with one child articulates nothing on its own, and a group
        // with none is a folder.
        if children.len() < 2 {
            out[i].bone = true;
            continue;
        }

        // Limbs lie end to end; a face's features scatter in two dimensions.
        // 0.9 is deliberately strict: a false "one bone" costs the artist a
        // click, and a false "eleven bones" costs them a hierarchy to clean up.
        if linearity(&children) < 0.9 {
            guesses.push(Guess {
                path: candidate.path.clone(),
                decided: format!("`{}` is one bone, not {}", candidate.name, children.len()),
                because: format!(
                    "its {} layers overlap inside about one layer's area, which reads as \
                     a drawing grouped for tidiness rather than a chain that articulates",
                    children.len()
                ),
                override_with: format!("[bones] on `{}`", candidate.name),
            });
            out[i].bone = true;
            // The children stay attachments on this one bone.
            continue;
        }

        out[i].bone = true;
    }
}

/// Is `path` an immediate child of `parent`?
fn is_child_of(path: &str, parent: &str) -> bool {
    path.strip_prefix(parent)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|rest| !rest.contains('/'))
}

/// How linear the children's arrangement is: 1.0 for a straight chain, 0 for a
/// scattered cluster.
///
/// Area was the obvious metric and is the wrong one — measured, not assumed. A
/// face's small features spread *nine* times their own mean area while an arm's
/// three abutting limbs spread only three, because a big rectangle next to a big
/// rectangle covers its own bounding box and a scatter of small ones does not.
///
/// Arrangement separates them cleanly. Limbs lie end to end along one axis;
/// eyes, brows and a mouth occupy two. So this is how much of the union's area
/// the children's centres explain along their dominant axis: a chain's centres
/// are collinear, a face's are not.
fn linearity(children: &[&Candidate]) -> f32 {
    if children.len() < 2 {
        return 1.0;
    }
    let centres: Vec<(f64, f64)> = children
        .iter()
        .map(|c| {
            let (x, y, w, h) = c.bounds;
            (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0)
        })
        .collect();

    let n = centres.len() as f64;
    let mean_x = centres.iter().map(|c| c.0).sum::<f64>() / n;
    let mean_y = centres.iter().map(|c| c.1).sum::<f64>() / n;

    // Spread along each axis, and how much they co-vary. A perfectly collinear
    // set has all its variance in one direction whatever that direction is, so
    // this compares the larger principal component against the smaller.
    let (mut var_x, mut var_y, mut cov) = (0.0, 0.0, 0.0);
    for (x, y) in &centres {
        let (dx, dy) = (x - mean_x, y - mean_y);
        var_x += dx * dx;
        var_y += dy * dy;
        cov += dx * dy;
    }
    var_x /= n;
    var_y /= n;
    cov /= n;

    let trace = var_x + var_y;
    if trace <= f64::EPSILON {
        // Every centre in the same place: stacked drawings, not a chain.
        return 0.0;
    }
    let diff = ((var_x - var_y).powi(2) + 4.0 * cov * cov).sqrt();
    let major = (trace + diff) / 2.0;
    let minor = (trace - diff) / 2.0;
    if major <= f64::EPSILON {
        return 0.0;
    }
    (1.0 - (minor / major)) as f32
}

/// Layers named `fire_01`, `fire_02`, … are frames of one drawing.
///
/// Five separate attachments is the wrong answer twice over: the artist gets
/// five slots to hide by hand, and the flipbook the numbering plainly means is
/// lost. `Sequence` is the model we already have for this.
fn infer_sequences(
    candidates: &[Candidate],
    tags: &[Tags],
    out: &mut [Inferred],
    guesses: &mut Vec<Guess>,
) {
    let mut runs: std::collections::BTreeMap<(usize, String), Vec<(usize, u32)>> =
        Default::default();

    for (i, candidate) in candidates.iter().enumerate() {
        if candidate.is_group || tags[i].has("ignore") {
            continue;
        }
        let Some((stem, number)) = split_trailing_number(&tags[i].name) else {
            continue;
        };
        // Keyed by depth as well as stem, so `arm/fire_01` and `fx/fire_01` do
        // not merge into one impossible sequence.
        runs.entry((candidate.depth, stem))
            .or_default()
            .push((i, number));
    }

    for ((_, stem), mut members) in runs {
        if members.len() < 2 {
            continue;
        }
        // An explicit `[frames]` anywhere in the run means the artist already
        // said so, and inference stays quiet.
        if members.iter().any(|(i, _)| tags[*i].has("frames")) {
            continue;
        }
        members.sort_by_key(|(_, number)| *number);

        let frames: Vec<String> = members
            .iter()
            .map(|(i, _)| candidates[*i].path.clone())
            .collect();
        let first = members[0].0;

        guesses.push(Guess {
            path: candidates[first].path.clone(),
            decided: format!("`{stem}` is a {}-frame sequence", frames.len()),
            because: "consecutively numbered layers at the same level read as frames of \
                      one drawing rather than separate attachments"
                .to_string(),
            override_with: format!("[!frames] on `{stem}`"),
        });

        out[first].sequence = Some(Sequence { stem, frames });
    }
}

/// `arm_01` → `("arm", 1)`. Returns `None` when there is no trailing number.
fn split_trailing_number(name: &str) -> Option<(String, u32)> {
    let digits_start = name.len()
        - name
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .count();
    if digits_start == name.len() {
        return None;
    }
    let number: u32 = name[digits_start..].parse().ok()?;
    let stem = name[..digits_start].trim_end_matches(['_', '-', ' ', '.']);
    if stem.is_empty() {
        return None;
    }
    Some((stem.to_string(), number))
}

/// A layer ending `_l` with an `_r` sibling is half of a mirrored pair.
///
/// Worth knowing because it is what a "mirror the pose" or "copy keys across"
/// operation needs, and the artist has already stated it in the naming.
fn infer_mirrors(
    candidates: &[Candidate],
    tags: &[Tags],
    out: &mut [Inferred],
    guesses: &mut Vec<Guess>,
) {
    let by_name: std::collections::HashMap<&str, usize> = tags
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), i))
        .collect();

    let mut paired = 0usize;
    for (i, tag) in tags.iter().enumerate() {
        let Some((stem, side)) = split_side(&tag.name) else {
            continue;
        };
        let other = format!("{stem}{}", if side == 'l' { 'r' } else { 'l' });
        if let Some(&j) = by_name.get(other.as_str()) {
            out[i].mirrors = Some(candidates[j].path.clone());
            paired += 1;
        }
    }

    if paired >= 2 {
        guesses.push(Guess {
            path: String::new(),
            decided: format!("{} layers pair with a mirrored sibling", paired),
            because: "names ending `_l` and `_r` read as left and right of one part, \
                      which is what mirroring a pose acts on"
                .to_string(),
            override_with: "rename either side".to_string(),
        });
    }
}

/// `arm_l` → `("arm_", 'l')`.
fn split_side(name: &str) -> Option<(&str, char)> {
    let lower = name.as_bytes();
    let last = *lower.last()? as char;
    if !matches!(last.to_ascii_lowercase(), 'l' | 'r') {
        return None;
    }
    let before = *lower.get(lower.len().checked_sub(2)?)? as char;
    if !matches!(before, '_' | '-' | '.') {
        return None;
    }
    Some((&name[..name.len() - 1], last.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(path: &str, is_group: bool, bounds: (i32, i32, u32, u32)) -> Candidate {
        Candidate {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            depth: path.matches('/').count(),
            is_group,
            bounds,
        }
    }

    fn run(candidates: &[Candidate]) -> (Vec<Inferred>, Vec<Guess>) {
        let tags: Vec<Tags> = candidates.iter().map(|c| Tags::parse(&c.name)).collect();
        let mut guesses = Vec::new();
        let inferred = infer(candidates, &tags, &mut guesses);
        (inferred, guesses)
    }

    #[test]
    fn a_face_grouped_for_tidiness_is_one_bone() {
        // The case the old "every group is a bone" rule got wrong, and the case
        // Spine makes you tag your way out of.
        let candidates = vec![
            layer("face", true, (0, 0, 100, 100)),
            layer("face/eye_left", false, (10, 20, 20, 15)),
            layer("face/eye_right", false, (60, 20, 20, 15)),
            layer("face/mouth", false, (30, 60, 40, 20)),
        ];
        let (_, guesses) = run(&candidates);

        let guess = guesses
            .iter()
            .find(|g| g.path == "face")
            .expect("the face was noticed");
        assert!(guess.decided.contains("one bone"), "{}", guess.decided);
        assert!(
            guess.override_with.contains("[bones]"),
            "and says how to disagree: {}",
            guess.override_with
        );
    }

    #[test]
    fn a_limb_chain_is_left_alone() {
        // Layers laid end to end are a chain that articulates, and inference
        // should have nothing to say about it.
        let candidates = vec![
            layer("arm", true, (0, 0, 300, 40)),
            layer("arm/upper", false, (0, 0, 100, 40)),
            layer("arm/fore", false, (100, 0, 100, 40)),
            layer("arm/hand", false, (200, 0, 100, 40)),
        ];
        let (_, guesses) = run(&candidates);
        assert!(
            !guesses.iter().any(|g| g.path == "arm"),
            "a chain needs no guess: {guesses:?}"
        );
    }

    #[test]
    fn a_tag_beats_inference() {
        // The rule that makes inference safe. An artist who said `[bones]`
        // gets bones, whatever the geometry looks like.
        let candidates = vec![
            layer("face [bones]", true, (0, 0, 100, 100)),
            layer("face [bones]/eye_l", false, (10, 20, 20, 15)),
            layer("face [bones]/eye_r", false, (60, 20, 20, 15)),
            layer("face [bones]/mouth", false, (30, 60, 40, 20)),
        ];
        let tags: Vec<Tags> = candidates.iter().map(|c| Tags::parse(&c.name)).collect();
        let mut guesses = Vec::new();
        infer(&candidates, &tags, &mut guesses);

        assert!(
            !guesses.iter().any(|g| g.decided.contains("one bone")),
            "inference stayed quiet where the file spoke: {guesses:?}"
        );
    }

    #[test]
    fn a_plural_tag_makes_every_child_a_bone_except_the_one_that_refuses() {
        // The composition the grammar exists for, and the thing the three
        // markers it replaced could not express: say it once for the group,
        // and write down only the exception.
        let candidates = vec![
            layer("face [bones]", true, (0, 0, 100, 100)),
            layer("face [bones]/brow", false, (10, 10, 20, 10)),
            layer("face [bones]/eye", false, (10, 30, 20, 15)),
            layer("face [bones]/mouth [!bone]", false, (30, 60, 40, 20)),
        ];
        let tags: Vec<Tags> = candidates.iter().map(|c| Tags::parse(&c.name)).collect();
        let mut guesses = Vec::new();
        let inferred = infer(&candidates, &tags, &mut guesses);

        assert!(inferred[1].bone, "brow inherited");
        assert!(inferred[2].bone, "eye inherited");
        assert!(!inferred[3].bone, "mouth refused");
    }

    #[test]
    fn a_face_and_an_arm_are_told_apart_by_arrangement_not_by_area() {
        // Recorded because the obvious metric is the wrong one, and it was
        // wrong in the direction that looks right: the face's small features
        // spread nine times their own mean area, the arm's abutting limbs only
        // three. Area would have called the arm the tidier group.
        let face = vec![
            layer("f/eye_l", false, (10, 20, 20, 15)),
            layer("f/eye_r", false, (60, 20, 20, 15)),
            layer("f/mouth", false, (30, 60, 40, 20)),
        ];
        let arm = vec![
            layer("a/upper", false, (0, 0, 100, 40)),
            layer("a/fore", false, (100, 0, 100, 40)),
            layer("a/hand", false, (200, 0, 100, 40)),
        ];
        let arm_refs: Vec<&Candidate> = arm.iter().collect();
        let face_refs: Vec<&Candidate> = face.iter().collect();

        assert!(
            linearity(&arm_refs) > 0.9,
            "limbs end to end are collinear: {}",
            linearity(&arm_refs)
        );
        assert!(
            linearity(&face_refs) < 0.9,
            "features scatter in two dimensions: {}",
            linearity(&face_refs)
        );
    }

    #[test]
    fn numbered_layers_read_as_a_sequence() {
        // Five attachments is wrong twice: five slots to hide by hand, and the
        // flipbook the numbering plainly means is lost.
        let candidates = vec![
            layer("fx", true, (0, 0, 100, 100)),
            layer("fx/fire_01", false, (0, 0, 50, 50)),
            layer("fx/fire_02", false, (0, 0, 50, 50)),
            layer("fx/fire_03", false, (0, 0, 50, 50)),
        ];
        let (inferred, guesses) = run(&candidates);

        let sequence = inferred
            .iter()
            .find_map(|i| i.sequence.as_ref())
            .expect("a sequence was found");
        assert_eq!(sequence.stem, "fire");
        assert_eq!(sequence.frames.len(), 3);
        assert!(guesses.iter().any(|g| g.decided.contains("3-frame")));
    }

    #[test]
    fn a_sequence_orders_its_frames_by_number_not_by_stacking() {
        // A PSD's layer order is bottom-to-top and an artist may reorder them;
        // the number is what says which frame is which.
        let candidates = vec![
            layer("fx", true, (0, 0, 100, 100)),
            layer("fx/fire_03", false, (0, 0, 50, 50)),
            layer("fx/fire_01", false, (0, 0, 50, 50)),
            layer("fx/fire_02", false, (0, 0, 50, 50)),
        ];
        let (inferred, _) = run(&candidates);
        let sequence = inferred.iter().find_map(|i| i.sequence.as_ref()).unwrap();
        assert_eq!(sequence.frames, ["fx/fire_01", "fx/fire_02", "fx/fire_03"]);
    }

    #[test]
    fn one_numbered_layer_is_not_a_sequence() {
        let candidates = vec![
            layer("fx", true, (0, 0, 100, 100)),
            layer("fx/fire_01", false, (0, 0, 50, 50)),
        ];
        let (inferred, _) = run(&candidates);
        assert!(inferred.iter().all(|i| i.sequence.is_none()));
    }

    #[test]
    fn identically_named_layers_at_different_depths_do_not_merge() {
        // `arm/fire_01` and `fx/fire_01` are two drawings that happen to share a
        // name, not one impossible sequence.
        let candidates = vec![
            layer("arm/fire_01", false, (0, 0, 50, 50)),
            layer("fx/deep/fire_01", false, (0, 0, 50, 50)),
        ];
        let (inferred, _) = run(&candidates);
        assert!(inferred.iter().all(|i| i.sequence.is_none()));
    }

    #[test]
    fn mirrored_names_pair_up() {
        let candidates = vec![
            layer("arm_l", false, (0, 0, 50, 50)),
            layer("arm_r", false, (60, 0, 50, 50)),
            layer("leg_l", false, (0, 60, 50, 50)),
            layer("leg_r", false, (60, 60, 50, 50)),
        ];
        let (inferred, guesses) = run(&candidates);

        assert_eq!(inferred[0].mirrors.as_deref(), Some("arm_r"));
        assert_eq!(inferred[1].mirrors.as_deref(), Some("arm_l"));
        assert!(guesses.iter().any(|g| g.decided.contains("pair")));
    }

    #[test]
    fn a_name_merely_ending_in_l_is_not_a_side() {
        // `barrel` should not pair with `barrer`. The separator is what makes
        // `_l` a side rather than a letter.
        let candidates = vec![
            layer("barrel", false, (0, 0, 50, 50)),
            layer("barrer", false, (0, 0, 50, 50)),
        ];
        let (inferred, _) = run(&candidates);
        assert!(inferred.iter().all(|i| i.mirrors.is_none()));
    }

    #[test]
    fn a_number_splits_off_its_stem() {
        assert_eq!(split_trailing_number("fire_01"), Some(("fire".into(), 1)));
        assert_eq!(split_trailing_number("walk-12"), Some(("walk".into(), 12)));
        assert_eq!(split_trailing_number("torso"), None);
        assert_eq!(
            split_trailing_number("01"),
            None,
            "a bare number has no stem"
        );
    }
}
