//! Shared row model for the Spine-style timeline (tree + dopesheet + graph).
//!
//! One place derives the visible structure from the active animation so the name
//! tree, the dopesheet, and the graph editor all agree on rows, order, and key
//! addresses. Rows are grouped by **target** (a bone or a slot); each group has
//! child property rows, and a group can be folded.

use crate::app_state::AppState;
use crate::commands::key_cmds::{BoneProperty, TimelineAddr};
use ankhimate_core::animation::{Interp, Timeline};
use ankhimate_core::ids::{AnimationId, BoneId, SlotId};

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
    pub addr: TimelineAddr,
    pub keys: Vec<KeyInfo>,
    /// Scalar channels this property plots in the graph: `[(name, values)]`,
    /// values parallel to `keys`. Rotate has one, translate/scale/shear have two.
    pub channels: Vec<GraphChannel>,
    /// `true` for timelines the dopesheet shows read-only (draw order, ik, deform).
    pub read_only: bool,
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
    pub rows: Vec<PropertyRow>,
    /// A stable key for fold state in egui memory.
    pub fold_id: u64,
    /// Union of every child key time, for the group's summary dots.
    pub summary_times: Vec<f32>,
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
            let rows = buckets.remove(&key).unwrap_or_default();
            let mut summary: Vec<f32> = rows
                .iter()
                .flat_map(|r| r.keys.iter().map(|k| k.time))
                .collect();
            summary.sort_by(f32::total_cmp);
            summary.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

            groups.push(Group {
                label: key.label(&bone_name, &slot_name),
                rows,
                fold_id: key.fold_id(),
                summary_times: summary,
            });
        }

        Self { groups }
    }
}

/// One entry in the flattened, fold-aware visible row list.
pub enum VisibleRow<'a> {
    /// A group header row (bone/slot name).
    Group { data: &'a Group, folded: bool },
    /// A property row under a group.
    Property { data: &'a PropertyRow },
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
                    out.push(VisibleRow::Property { data: row });
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
    let vec2 = |keys: &[ankhimate_core::animation::Key<glam::Vec2>]| {
        let infos: Vec<KeyInfo> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| KeyInfo {
                index: i,
                time: k.time,
                interp: k.interp,
            })
            .collect();
        let xs = keys.iter().map(|k| k.value.x).collect();
        let ys = keys.iter().map(|k| k.value.y).collect();
        (
            infos,
            vec![
                GraphChannel {
                    axis: 0,
                    values: xs,
                },
                GraphChannel {
                    axis: 1,
                    values: ys,
                },
            ],
        )
    };

    match timeline {
        Timeline::BoneTranslate { bone, keys } => {
            let (k, ch) = vec2(keys);
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: "translate",
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Translate,
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                },
            )
        }
        Timeline::BoneRotate { bone, keys } => {
            let (k, ch) = scalar(keys, 2);
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: "rotate",
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Rotate,
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                },
            )
        }
        Timeline::BoneScale { bone, keys } => {
            let (k, ch) = vec2(keys);
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: "scale",
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Scale,
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                },
            )
        }
        Timeline::BoneShear { bone, keys } => {
            let (k, ch) = vec2(keys);
            (
                GroupKey::Bone(*bone),
                PropertyRow {
                    label: "shear",
                    addr: TimelineAddr::Bone {
                        bone: *bone,
                        property: BoneProperty::Shear,
                    },
                    keys: k,
                    channels: ch,
                    read_only: false,
                },
            )
        }
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
                    addr: TimelineAddr::SlotColor { slot: *slot },
                    keys: infos,
                    channels: vec![GraphChannel {
                        axis: 2,
                        values: alpha,
                    }],
                    read_only: false,
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
                    addr: TimelineAddr::SlotAttachment { slot: *slot },
                    keys: infos,
                    channels: Vec::new(),
                    read_only: false,
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
        Timeline::Deform { keys, .. } => (
            GroupKey::Global,
            read_only_row("deform", keys.iter().map(|k| (k.time, k.interp))),
        ),
    }
}

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
        // A harmless placeholder addr; read-only rows are not edited here.
        addr: TimelineAddr::SlotColor {
            slot: Default::default(),
        },
        keys: infos,
        channels: Vec::new(),
        read_only: true,
    }
}
