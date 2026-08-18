//! The editor clipboard (T-209).
//!
//! Session state, never the document: copying is not an edit, so it must not
//! land on the undo stack or in a save file. Pasting *is* an edit and goes
//! through commands like everything else.
//!
//! Entities are captured **by value and by name**, not by id. Slotmap keys are
//! recycled and rewritten by undo (`IdRemap`), so an id-holding clipboard would
//! quietly paste onto the wrong bone after a delete-undo cycle.

use ankhimate_core::attachment::Attachment;
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::Bone;
use ankhimate_core::slot::Slot;

/// One bone in a copied subtree.
#[derive(Debug, Clone)]
pub struct ClipBone {
    pub bone: Bone,
    /// Index into [`BoneClip::bones`], or `None` for the subtree's own root.
    /// Positional rather than an id so the clip is self-contained.
    pub parent: Option<usize>,
}

/// A slot that hung off a copied bone, with its artwork.
#[derive(Debug, Clone)]
pub struct ClipSlot {
    pub slot: Slot,
    /// Index into [`BoneClip::bones`].
    pub bone: usize,
    /// `(skin name, attachment name, attachment)` — skins are matched by name on
    /// paste, so a subtree copied with a costume keeps it.
    pub entries: Vec<(String, String, Attachment)>,
}

/// A constraint that acted on a copied subtree (T-909).
///
/// Held by **index into the clip**, so a paste rebuilds it against the bones it
/// just created rather than against ids from the document it came from — which
/// may be a different document entirely.
#[derive(Debug, Clone)]
pub struct ClipConstraint {
    /// Name to give the rebuilt constraint. Uniquified on paste.
    pub name: String,
    /// The constraint with its bone ids left as they were; every one of them is
    /// re-pointed on paste using the index maps below.
    pub constraint: ankhimate_core::constraints::Constraint,
    /// Positions in [`BoneClip::bones`] for each id the constraint holds, in the
    /// order [`Constraint::affected_bones`] reports them.
    pub bones: Vec<usize>,
    /// Position of the target bone, when the constraint has one *and* it was
    /// inside the copied subtree.
    pub target: Option<usize>,
}

/// One animation's keys for a copied subtree (T-909).
#[derive(Debug, Clone)]
pub struct ClipAnimation {
    /// Matched by name on paste: pasting into a rig that already has a `walk`
    /// adds to that clip rather than making a second one.
    pub name: String,
    pub duration: f32,
    /// `(bone index in the clip, the timeline with its ids left as-is)`.
    pub timelines: Vec<(usize, ankhimate_core::animation::Timeline)>,
}

/// A copied bone subtree: the bones, plus everything hanging off them.
#[derive(Debug, Clone, Default)]
pub struct BoneClip {
    pub bones: Vec<ClipBone>,
    pub slots: Vec<ClipSlot>,
    /// Constraints wholly inside the subtree (T-909).
    ///
    /// Wholly, deliberately: one that reaches a bone outside the copy cannot be
    /// rebuilt without inventing that bone, so it is reported and dropped rather
    /// than pasted half-wired. Which ones were dropped is in
    /// [`BoneClip::dropped_constraints`].
    pub constraints: Vec<ClipConstraint>,
    /// Names of constraints left behind because they reached outside the copy.
    pub dropped_constraints: Vec<String>,
    /// Animation keys for the copied bones (T-909).
    ///
    /// The half of a rig transfer the reference tool loses: without these,
    /// moving a rigged and animated limb to another skeleton means re-animating
    /// it by hand.
    pub animations: Vec<ClipAnimation>,
}

/// A copied pose: bone **names** to local transforms.
///
/// Names, so a pose can be pasted onto the same rig after any amount of
/// undo/redo, and eventually onto a different rig with matching bone names.
#[derive(Debug, Clone, Default)]
pub struct PoseClip {
    pub entries: Vec<(String, Transform)>,
}

impl PoseClip {
    /// Mirror the pose across the rig's X axis.
    ///
    /// Negating X translation and rotation is what makes a left-half walk cycle
    /// into the right half; shear flips with it so a sheared pose stays
    /// consistent. Scale is left alone — mirroring is not a reflection of the
    /// art, it is the same pose facing the other way.
    pub fn mirrored(&self) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .map(|(name, t)| {
                    (
                        name.clone(),
                        Transform {
                            position: glam::vec2(-t.position.x, t.position.y),
                            rotation: -t.rotation,
                            scale: t.scale,
                            shear: glam::vec2(-t.shear.x, -t.shear.y),
                        },
                    )
                })
                .collect(),
        }
    }
}

/// What the clipboard is holding.
#[derive(Debug, Clone, Default)]
pub enum Clipboard {
    #[default]
    Empty,
    Bones(BoneClip),
    Pose(PoseClip),
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        matches!(self, Clipboard::Empty)
    }

    /// One line for the status bar, so a paste that does nothing can say why.
    pub fn describe(&self) -> String {
        match self {
            Clipboard::Empty => "nothing".into(),
            Clipboard::Bones(clip) => match clip.bones.len() {
                1 => "1 bone".into(),
                n => format!("{n} bones"),
            },
            Clipboard::Pose(clip) => match clip.entries.len() {
                1 => "a pose (1 bone)".into(),
                n => format!("a pose ({n} bones)"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirroring_flips_x_and_rotation_only() {
        let clip = PoseClip {
            entries: vec![(
                "arm".into(),
                Transform {
                    position: glam::vec2(10.0, 4.0),
                    rotation: 0.5,
                    scale: glam::vec2(2.0, 3.0),
                    shear: glam::vec2(0.2, -0.1),
                },
            )],
        };
        let (_, t) = &clip.mirrored().entries[0];
        assert_eq!(t.position, glam::vec2(-10.0, 4.0), "X flips, Y does not");
        assert_eq!(t.rotation, -0.5);
        assert_eq!(t.scale, glam::vec2(2.0, 3.0), "scale is not a reflection");
        assert_eq!(t.shear, glam::vec2(-0.2, 0.1));
    }

    #[test]
    fn mirroring_twice_is_the_original() {
        let clip = PoseClip {
            entries: vec![(
                "arm".into(),
                Transform {
                    position: glam::vec2(10.0, 4.0),
                    rotation: 0.5,
                    scale: glam::Vec2::ONE,
                    shear: glam::vec2(0.2, -0.1),
                },
            )],
        };
        let round_trip = clip.mirrored().mirrored();
        assert_eq!(round_trip.entries[0].1, clip.entries[0].1);
    }
}
