//! Shared row model for the Spine-style timeline (tree + dopesheet + graph).
//!
//! One place derives the visible structure from the active animation so the name
//! tree, the dopesheet, and the graph editor all agree on rows, order, and key
//! addresses. Rows are grouped by **target** (a bone or a slot); each group has
//! child property rows, and a group can be folded.

use crate::app_state::AppState;
use ankhimate_core::animation::{Axis, Interp, Timeline};
use ankhimate_core::ids::{AnimationId, BoneId, SlotId};
use ankhimate_document::commands::key_cmds::{BoneProperty, TimelineAddr};

/// A single key as the timeline UI needs it: where it is and how it eases.
#[derive(Clone, Copy)]
pub struct KeyInfo {
    pub index: usize,
    pub time: f32,
    /// Value in the timeline's storage units — for the graph plot. Vec2 channels
    /// contribute one scalar per axis; see [`PropertyRow::channels`].
    pub interp: Interp,
}

/// A property row (e.g. `rotate`) under a target group.
pub struct PropertyRow {
    pub label: &'static str,
    /// Glyph for the property kind. In a sixty-row list the word is the slowest
    /// way to find "the rotate one".
    pub icon: &'static str,
    pub addr: TimelineAddr,
    pub keys: Vec<KeyInfo>,
    /// Scalar channels this property plots in the graph: `[(name, values)]`,
    /// values parallel to `keys`. Rotate has one, translate/scale/shear have two.
    pub channels: Vec<GraphChannel>,
    /// `true` for timelines the dopesheet shows read-only (draw order, ik, deform).
    pub read_only: bool,
    /// A key unique to this row, for widget ids and solo state.
    ///
    /// Not `addr.stable_id()`: read-only rows carry a placeholder address, so
    /// every one of them hashed to the same value — four `ik mix` rows under
    /// `scene` all claimed one widget id, and egui reported the clash across the
    /// whole panel. Assigned from the group and the row's position instead.
    pub row_id: u64,
}

/// A plottable scalar channel of a property row.
pub struct GraphChannel {
    /// The channel's colour index (0=x/red, 1=y/green, 2=single/blue).
    pub axis: usize,
    /// One value per key in `PropertyRow::keys`, same order.
    pub values: Vec<f32>,
}

/// A target group (one bone or one slot) with its property rows.
pub struct Group {
    pub label: String,
    /// Icon and tint for the row, so a bone row and a slot row are told apart
    /// without reading either name.
    pub icon: &'static str,
    pub tint: Option<[f32; 4]>,
    pub rows: Vec<PropertyRow>,
    /// A stable key for fold state in egui memory.
    pub fold_id: u64,
    /// Union of every child key time, for the group's summary dots.
    pub summary_times: Vec<f32>,
    /// The bone this group is for, when it is a bone group (T-905).
    ///
    /// Carried so the header can show and edit the group's sampling offset —
    /// an offset is invisible in the keys, so a track that is shifted has to say
    /// so where the track is, not only in a panel somewhere else.
    pub bone: Option<ankhimate_core::ids::BoneId>,
    /// Seconds this bone's timelines are shifted by. `0.0` when unshifted.
    pub offset: f32,
}

/// The whole visible tree for the active animation.
#[derive(Default)]
pub struct TimelineModel {
    pub groups: Vec<Group>,
}

impl TimelineModel {
    pub fn build(state: &AppState, anim: AnimationId) -> Self {
        let Some(animation) = state.doc.animations.get(anim) else {
            return Self::default();
        };

        // Bucket timelines by their target so each bone/slot becomes one group.
        // Preserve first-seen order for stability.
        let mut order: Vec<GroupKey> = Vec::new();
        let mut buckets: std::collections::HashMap<GroupKey, Vec<PropertyRow>> =
            std::collections::HashMap::new();

        for timeline in &animation.timelines {
            let (key, row) = describe(timeline);
            if !buckets.contains_key(&key) {
                order.push(key);
            }
            buckets.entry(key).or_default().push(row);
        }

        let bone_name = |b: BoneId| {
            state
                .doc
                .skeleton
                .bones
                .get(b)
                .map(|x| x.name.clone())
                .unwrap_or_else(|| "?".to_string())
        };
        let slot_name = |s: SlotId| {
            state
                .doc
                .skeleton
                .slots
                .get(s)
                .map(|x| x.name.clone())
                .unwrap_or_else(|| "?".to_string())
        };

        let mut groups = Vec::new();
        for key in order {
            let mut rows = buckets.remove(&key).unwrap_or_default();
            let mut summary: Vec<f32> = rows
                .iter()
                .flat_map(|r| r.keys.iter().map(|k| k.time))
                .collect();
            summary.sort_by(f32::total_cmp);
            summary.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

            let (icon, tint) = match key {
                GroupKey::Bone(b) => (
                    crate::ui::icons::BONE,
                    Some(crate::ui::canvas::renderer::group_color(
                        &state.doc.skeleton,
                        b,
                    )),
                ),
                GroupKey::Slot(_) => (crate::ui::icons::SLOT, None),
                GroupKey::Global => (crate::ui::icons::DRAW_ORDER, None),
            };
            // Stamped here rather than at construction: a row does not know its
            // own position until the group is assembled.
            for (index, row) in rows.iter_mut().enumerate() {
                row.row_id = key.fold_id().rotate_left(17).wrapping_add(index as u64 + 1);
            }
            let bone = match key {
                GroupKey::Bone(b) => Some(b),
                _ => None,
            };
            groups.push(Group {
                label: key.label(&bone_name, &slot_name),
                icon,
                tint,
                rows,
                fold_id: key.fold_id(),
                summary_times: summary,
                bone,
                offset: bone.map(|b| animation.bone_offset(b)).unwrap_or(0.0),
            });
        }

        Self { groups }
    }

    /// Every keyed time in the clip, once, for the animation-wide summary row.
    pub fn summary_times(&self) -> Vec<f32> {
        let mut times: Vec<_> = self
            .groups
            .iter()
            .flat_map(|group| group.summary_times.iter().copied())
            .collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
        times
    }
}

/// One entry in the flattened, fold-aware visible row list.
pub enum VisibleRow<'a> {
    /// A group header row (bone/slot name).
    Group { data: &'a Group, folded: bool },
    /// A property row under a group.
    Property {
        data: &'a PropertyRow,
        /// The group this row sits under, so soloing a group can carry to its
        /// properties without a second lookup.
        group_id: u64,
    },
}

impl VisibleRow<'_> {
    /// A stable key for solo state.
    ///
    /// Groups and properties share one namespace: `fold_id` comes from a slotmap
    /// key and `stable_id` from a key mixed with a property tag, so a collision
    /// would need a slotmap index to equal another one times sixteen plus a tag —
    /// possible in principle, which is why groups are tagged apart here.
    pub fn solo_id(&self) -> u64 {
        match self {
            VisibleRow::Group { data, .. } => data.fold_id ^ 0xA11_0000_0000_0000,
            VisibleRow::Property { data, .. } => data.row_id,
        }
    }

    /// Is this row shown, given the solo set? An empty set shows everything.
    pub fn is_soloed(&self, soloed: &std::collections::BTreeSet<u64>) -> bool {
        if soloed.is_empty() {
            return true;
        }
        match self {
            VisibleRow::Group { .. } => soloed.contains(&self.solo_id()),
            // A property is shown when it is soloed itself, or when its whole
            // group is: "show me this bone" has to mean all of its channels.
            VisibleRow::Property { data, group_id } => {
                soloed.contains(&data.row_id) || soloed.contains(&(group_id ^ 0xA11_0000_0000_0000))
            }
        }
    }
}

impl TimelineModel {
    /// Flatten groups + their (unfolded) property rows into display order. The
    /// tree and the sheet both iterate this so their rows line up 1:1.
    pub fn visible_rows<'a>(&'a self, is_folded: &'a impl Fn(u64) -> bool) -> Vec<VisibleRow<'a>> {
        let mut out = Vec::new();
        for group in self.groups.iter() {
            let folded = is_folded(group.fold_id);
            out.push(VisibleRow::Group {
                data: group,
                folded,
            });
            if !folded {
                for row in &group.rows {
                    out.push(VisibleRow::Property {
                        data: row,
                        group_id: group.fold_id,
                    });
                }
            }
        }
        out
    }
}

/// Identity of a group — the target its rows drive.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GroupKey {
    Bone(BoneId),
    Slot(SlotId),
    /// Timelines with no per-target identity (draw order) share one group.
    Global,
}

impl GroupKey {
    fn label(
        &self,
        bone_name: &impl Fn(BoneId) -> String,
        slot_name: &impl Fn(SlotId) -> String,
    ) -> String {
        match self {
            GroupKey::Bone(b) => bone_name(*b),
            GroupKey::Slot(s) => slot_name(*s),
            GroupKey::Global => "scene".to_string(),
        }
    }

    fn fold_id(&self) -> u64 {
        use ankhimate_core::slotmap::Key;
        match self {
            GroupKey::Bone(b) => b.data().as_ffi(),
            GroupKey::Slot(s) => s.data().as_ffi() ^ 0x5555_5555_5555_5555,
            GroupKey::Global => 0xffff_ffff_ffff_ffff,
        }
    }
}

/// Turn one timeline into a `(group, property row)` pair.
fn describe(timeline: &Timeline) -> (GroupKey, PropertyRow) {
    // Helpers to collect keys + channels.
    let scalar = |keys: &[ankhimate_core::animation::Key<f32>], axis: usize| {
        let infos = keys
            .iter()
            .enumerate()
            .map(|(i, k)| KeyInfo {
                index: i,
                time: k.time,
                interp: k.interp,
            })
            .collect();
        let values = keys.iter().map(|k| k.value).collect();
        (infos, vec![GraphChannel { axis, values }])
    };
    match timeline {
        Timeline::BoneTranslate { bone, axis, keys } => {
            // One row per track. The axes are independent now — their own key
            // times, their own curves — so one row showing both would have to
            // lie about one of them.
            let (k, ch) = scalar(keys, axis.index());
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: match axis {
                        Axis::X => "translate x",
                        Axis::Y => "translate y",
                    },
                    icon: crate::ui::icons::TRANSLATE,
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Translate,
                        axis: Some(*axis),
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                    row_id: 0,
                },
            )
        }
        Timeline::BoneRotate { bone, keys } => {
            let (k, ch) = scalar(keys, 2);
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: "rotate",
                    icon: crate::ui::icons::ROTATE,
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Rotate,
                        axis: None,
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                    row_id: 0,
                },
            )
        }
        Timeline::BoneScale { bone, axis, keys } => {
            // One row per track. The axes are independent now — their own key
            // times, their own curves — so one row showing both would have to
            // lie about one of them.
            let (k, ch) = scalar(keys, axis.index());
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: match axis {
                        Axis::X => "scale x",
                        Axis::Y => "scale y",
                    },
                    icon: crate::ui::icons::SCALE,
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Scale,
                        axis: Some(*axis),
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                    row_id: 0,
                },
            )
        }
        Timeline::BoneShear { bone, axis, keys } => {
            // One row per track. The axes are independent now — their own key
            // times, their own curves — so one row showing both would have to
            // lie about one of them.
            let (k, ch) = scalar(keys, axis.index());
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: match axis {
                        Axis::X => "shear x",
                        Axis::Y => "shear y",
                    },
                    icon: crate::ui::icons::SHEAR,
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Shear,
                        axis: Some(*axis),
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                    row_id: 0,
                },
            )
        }
        Timeline::SlotVisible { slot, keys } => (
            GroupKey::Slot(*slot),
            read_only_row("visible", keys.iter().map(|k| (k.time, k.interp))),
        ),
        Timeline::SlotColor { slot, keys } => {
            let infos = keys
                .iter()
                .enumerate()
                .map(|(i, k)| KeyInfo {
                    index: i,
                    time: k.time,
                    interp: k.interp,
                })
                .collect();
            let alpha = keys.iter().map(|k| k.value[3]).collect();
            (
                GroupKey::Slot(*slot),
                PropertyRow {
                    label: "color",
                    icon: crate::ui::icons::PALETTE,
                    addr: TimelineAddr::SlotColor { slot: *slot },
                    keys: infos,
                    channels: vec![GraphChannel {
                        axis: 2,
                        values: alpha,
                    }],
                    read_only: false,
                    row_id: 0,
                },
            )
        }
        Timeline::SlotAttachment { slot, keys } => {
            let infos = keys
                .iter()
                .enumerate()
                .map(|(i, k)| KeyInfo {
                    index: i,
                    time: k.time,
                    interp: k.interp,
                })
                .collect();
            (
                GroupKey::Slot(*slot),
                PropertyRow {
                    label: "attachment",
                    icon: crate::ui::icons::IMAGE,
                    addr: TimelineAddr::SlotAttachment { slot: *slot },
                    keys: infos,
                    channels: Vec::new(),
                    read_only: false,
                    row_id: 0,
                },
            )
        }
        // Read-only rows (edited through their own panels).
        Timeline::DrawOrder { keys } => (
            GroupKey::Global,
            read_only_row("draw order", keys.iter().map(|k| (k.time, k.interp))),
        ),
        Timeline::IkMix { keys, .. } => (
            GroupKey::Global,
            read_only_row("ik mix", keys.iter().map(|k| (k.time, k.interp))),
        ),
        Timeline::IkBendDirection { keys, .. } => (
            GroupKey::Global,
            read_only_row("ik bend", keys.iter().map(|k| (k.time, k.interp))),
        ),
        Timeline::IkSoftness { keys, .. } => (
            GroupKey::Global,
            read_only_row("ik softness", keys.iter().map(|k| (k.time, k.interp))),
        ),
        Timeline::TransformConstraintMix { keys, .. } => (
            GroupKey::Global,
            read_only_row("constraint mix", keys.iter().map(|k| (k.time, k.interp))),
        ),
        Timeline::Deform { keys, .. } => (
            GroupKey::Global,
            read_only_row("deform", keys.iter().map(|k| (k.time, k.interp))),
        ),
    }
}

/// A row the timeline can show but not edit — constraint mixes, deform, draw
/// order. They share one glyph because what matters about them here is that they
/// are *not* editable, not which kind they are.
fn read_only_row(label: &'static str, keys: impl Iterator<Item = (f32, Interp)>) -> PropertyRow {
    let infos = keys
        .enumerate()
        .map(|(i, (time, interp))| KeyInfo {
            index: i,
            time,
            interp,
        })
        .collect();
    PropertyRow {
        label,
        icon: crate::ui::icons::READ_ONLY,
        // A harmless placeholder addr; read-only rows are not edited here.
        addr: TimelineAddr::SlotColor {
            slot: Default::default(),
        },
        keys: infos,
        channels: Vec::new(),
        read_only: true,
        row_id: 0,
    }
}

#[cfg(test)]
mod solo_tests {
    use super::*;
    use std::collections::BTreeSet;

    fn property(addr: TimelineAddr, row_id: u64) -> PropertyRow {
        PropertyRow {
            label: "rotate",
            icon: "",
            addr,
            keys: Vec::new(),
            channels: Vec::new(),
            read_only: false,
            row_id,
        }
    }

    fn slot_addr(n: u64) -> TimelineAddr {
        use ankhimate_core::slotmap::KeyData;
        TimelineAddr::SlotColor {
            slot: SlotId::from(KeyData::from_ffi(n)),
        }
    }

    #[test]
    fn clip_summary_has_one_marker_per_keyed_time() {
        let group = |times: Vec<f32>, id| Group {
            label: String::new(),
            icon: "",
            tint: None,
            rows: Vec::new(),
            fold_id: id,
            summary_times: times,
            bone: None,
            offset: 0.0,
        };
        let model = TimelineModel {
            groups: vec![group(vec![0.0, 0.5], 1), group(vec![0.5, 1.0], 2)],
        };
        assert_eq!(model.summary_times(), vec![0.0, 0.5, 1.0]);
    }

    /// The empty set is "show everything", not "show nothing". Un-soloing the
    /// last row is the commonest way to empty it, and a blank sheet there would
    /// read as the clip having lost its keys.
    #[test]
    fn an_empty_solo_set_shows_every_row() {
        let row = property(slot_addr(1), 1);
        let visible = VisibleRow::Property {
            data: &row,
            group_id: 7,
        };
        assert!(visible.is_soloed(&BTreeSet::new()));
    }

    #[test]
    fn soloing_one_row_hides_its_siblings() {
        let (a, b) = (property(slot_addr(1), 1), property(slot_addr(2), 2));
        let (va, vb) = (
            VisibleRow::Property {
                data: &a,
                group_id: 7,
            },
            VisibleRow::Property {
                data: &b,
                group_id: 7,
            },
        );
        let mut soloed = BTreeSet::new();
        soloed.insert(va.solo_id());
        assert!(va.is_soloed(&soloed));
        assert!(!vb.is_soloed(&soloed), "a sibling stayed visible");
    }

    /// "Show me this bone" has to mean all of its channels, or soloing a group
    /// would show a header with nothing under it.
    #[test]
    fn soloing_a_group_carries_to_its_properties() {
        let row = property(slot_addr(1), 1);
        let group = Group {
            label: "arm".into(),
            icon: "",
            tint: None,
            rows: Vec::new(),
            fold_id: 7,
            summary_times: Vec::new(),
            bone: None,
            offset: 0.0,
        };
        let header = VisibleRow::Group {
            data: &group,
            folded: false,
        };
        let child = VisibleRow::Property {
            data: &row,
            group_id: 7,
        };
        let mut soloed = BTreeSet::new();
        soloed.insert(header.solo_id());
        assert!(header.is_soloed(&soloed));
        assert!(child.is_soloed(&soloed));
    }

    /// Groups and properties share one id space, so a group's key is tagged
    /// apart from any property's.
    /// Read-only rows share one placeholder address, so identity has to come
    /// from `row_id` — four `ik mix` rows under `scene` hashing alike is exactly
    /// how egui ended up reporting a widget clash across the whole panel.
    #[test]
    fn rows_sharing_an_address_still_have_distinct_ids() {
        let (a, b) = (property(slot_addr(0), 11), property(slot_addr(0), 12));
        let (va, vb) = (
            VisibleRow::Property {
                data: &a,
                group_id: 3,
            },
            VisibleRow::Property {
                data: &b,
                group_id: 3,
            },
        );
        assert_eq!(a.addr.stable_id(), b.addr.stable_id(), "same address");
        assert_ne!(va.solo_id(), vb.solo_id(), "but distinct rows");
    }

    #[test]
    fn group_and_property_ids_do_not_collide() {
        let row = property(slot_addr(7), 7);
        let group = Group {
            label: "arm".into(),
            icon: "",
            tint: None,
            rows: Vec::new(),
            fold_id: 7,
            summary_times: Vec::new(),
            bone: None,
            offset: 0.0,
        };
        let header = VisibleRow::Group {
            data: &group,
            folded: false,
        };
        let child = VisibleRow::Property {
            data: &row,
            group_id: 7,
        };
        assert_ne!(header.solo_id(), child.solo_id());
    }
}
