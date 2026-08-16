//! Version migration (PLAN §6.1, ADR 0004).
//!
//! `.ankh` files carry a mandatory `version`. Migration runs before conversion to
//! the core model, stepping a parsed document forward one version at a time.
//!
//! Each step mutates a parsed document and bumps its version, so a v1 file walks
//! v1→v2→v3 rather than needing a reader per version. A field a newer version
//! removed arrives in  — the same catch-all that lets a v2 file survive a
//! round-trip through a v1 editor — so a step reads the old shape from there.

use crate::schema::{self, CURRENT_VERSION, Project};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MigrateError {
    /// The file was written by a newer version of the editor.
    #[error("project version {found} is newer than supported version {supported}")]
    TooNew { found: u32, supported: u32 },
    /// Version 0 does not exist, or no step knows how to leave this one.
    #[error("project version {found} is not valid")]
    InvalidVersion { found: u32 },
}

/// Bring a **raw** project tree up to [`CURRENT_VERSION`]'s *shape*.
///
/// Runs before deserialization, and exists because [`migrate`] cannot: a step
/// that changes a field's shape produces values the current types reject, so a
/// v1 file would fail to parse before its migration ever ran. Anything that
/// merely moves or renames a field belongs in [`migrate`], which is easier to
/// read against typed data; anything that changes a *type* belongs here.
///
/// Unknown versions pass through untouched — [`migrate`] does the validating.
pub fn migrate_json(mut raw: serde_json::Value) -> Result<serde_json::Value, MigrateError> {
    let version = raw
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(CURRENT_VERSION as u64);
    if version < 2 {
        v1_to_v2_json(&mut raw);
    }
    Ok(raw)
}

/// v1 → v2, on the parts whose *type* changed.
///
/// **Bone tracks.** v1 paired the axes of translate, scale and shear into one
/// keyframe: one time, both values, one easing. v2 gives each axis its own
/// track, so one v1 timeline becomes two — x keeping the x values, y the y
/// ones, both inheriting the shared times and easing. That is exactly what the
/// pairing meant, so an existing rig poses identically; what it gains is the
/// freedom to move the two apart afterwards.
///
/// **Mix keys.** v1's `value: [rotate, translate, scale, shear]` becomes seven
/// named fields, each pair filled from the one number that covered both axes.
fn v1_to_v2_json(raw: &mut serde_json::Value) {
    use serde_json::{Value, json};

    let Some(animations) = raw.get_mut("animations").and_then(|a| a.as_array_mut()) else {
        return;
    };
    for animation in animations {
        let Some(timelines) = animation
            .get_mut("timelines")
            .and_then(|t| t.as_array_mut())
        else {
            continue;
        };
        let mut out: Vec<Value> = Vec::with_capacity(timelines.len());
        for timeline in timelines.drain(..) {
            let kind = timeline
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or_default()
                .to_string();
            match kind.as_str() {
                "bone_translate" | "bone_scale" | "bone_shear" => {
                    let keys = timeline
                        .get("keys")
                        .and_then(|k| k.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for axis in ["x", "y"] {
                        let split: Vec<Value> = keys
                            .iter()
                            .map(|k| {
                                // Keep every other field — the easing is
                                // flattened alongside `x`/`y`, so copying the
                                // key and swapping the value carries it.
                                let mut key = k.clone();
                                if let Some(map) = key.as_object_mut() {
                                    let value = map.get(axis).cloned().unwrap_or(json!(0.0));
                                    map.remove("x");
                                    map.remove("y");
                                    map.insert("value".into(), value);
                                }
                                key
                            })
                            .collect();
                        if split.is_empty() {
                            continue;
                        }
                        let mut t = timeline.clone();
                        if let Some(map) = t.as_object_mut() {
                            map.insert("axis".into(), json!(axis));
                            map.insert("keys".into(), Value::Array(split));
                        }
                        out.push(t);
                    }
                }
                "transform_constraint_mix" => {
                    let mut t = timeline.clone();
                    if let Some(keys) = t.get_mut("keys").and_then(|k| k.as_array_mut()) {
                        for key in keys.iter_mut() {
                            let old = key.get("value").and_then(|v| v.as_array()).cloned();
                            let Some(values) = old else { continue };
                            let at =
                                |i: usize| values.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let (rotate, translate, scale, shear) = (at(0), at(1), at(2), at(3));
                            if let Some(map) = key.as_object_mut() {
                                map.remove("value");
                                map.insert("rotate".into(), json!(rotate));
                                map.insert("translate_x".into(), json!(translate));
                                map.insert("translate_y".into(), json!(translate));
                                map.insert("scale_x".into(), json!(scale));
                                map.insert("scale_y".into(), json!(scale));
                                map.insert("shear_x".into(), json!(shear));
                                map.insert("shear_y".into(), json!(shear));
                            }
                        }
                    }
                    out.push(t);
                }
                _ => out.push(timeline),
            }
        }
        if let Some(slot) = animation.get_mut("timelines") {
            *slot = Value::Array(out);
        }
    }
}

/// Bring a parsed project up to [`CURRENT_VERSION`].
pub fn migrate(project: Project) -> Result<Project, MigrateError> {
    if project.version == 0 {
        return Err(MigrateError::InvalidVersion { found: 0 });
    }
    if project.version > CURRENT_VERSION {
        return Err(MigrateError::TooNew {
            found: project.version,
            supported: CURRENT_VERSION,
        });
    }

    let mut project = project;
    while project.version < CURRENT_VERSION {
        match project.version {
            1 => v1_to_v2(&mut project),
            // Unreachable while the guard above holds, but a silent infinite
            // loop is the worst way to find out otherwise.
            other => return Err(MigrateError::InvalidVersion { found: other }),
        }
    }
    debug_assert_eq!(project.version, CURRENT_VERSION);
    Ok(project)
}

/// v1 → v2: per-axis constraint mixes, and one track per axis.
///
/// Two changes, both "one number became two".
///
/// **Mixes.** v1 stored `mixes: [rotate, translate, scale, shear]` — one number
/// per channel, applied to both axes of the channels that have two. A v1 value
/// therefore means "this much on both axes".
///
/// **Bone tracks.** v1 paired the axes of translate, scale and shear into one
/// keyframe: one time, both values, one easing. v2 gives each axis its own
/// track, so a v1 timeline becomes two — the x track keeping the x values and
/// the y track the y ones, both inheriting the shared times and easing. That is
/// exactly what the pairing meant, so an existing rig poses identically; what it
/// gains is the ability to move the two apart afterwards.
///
/// Old fields are read out of `extra`: v2's types no longer name them, so serde
/// routes the unknown keys into the catch-all rather than failing the parse.
/// That is the same mechanism that lets a v2 file survive a round-trip through a
/// v1 editor, used here in the other direction.
fn v1_to_v2(project: &mut Project) {
    for constraint in &mut project.constraints {
        if let Some(old) = constraint.extra.remove("mixes")
            && let Some(values) = old.as_array()
        {
            let at = |i: usize| values.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let (rotate, translate, scale, shear) = (at(0), at(1), at(2), at(3));
            constraint.transform_mix = Some(schema::TransformMix {
                rotate,
                translate_x: translate,
                translate_y: translate,
                scale_x: scale,
                scale_y: scale,
                shear_x: shear,
                shear_y: shear,
            });
        }
    }

    project.version = 2;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(version: u32) -> Project {
        Project {
            version,
            name: "t".into(),
            fps: 30,
            assets: Vec::new(),
            bones: Vec::new(),
            slots: Vec::new(),
            draw_order: Vec::new(),
            skins: Vec::new(),
            default_skin: String::new(),
            constraints: Vec::new(),
            constraint_order: Vec::new(),
            animations: Vec::new(),
            export_presets: Vec::new(),
            groups: Vec::new(),
            extra: Default::default(),
        }
    }

    #[test]
    fn current_version_passes_through() {
        let p = migrate(project(CURRENT_VERSION)).expect("the current version is a no-op");
        assert_eq!(p.version, CURRENT_VERSION);
    }

    #[test]
    fn future_version_is_rejected_with_both_numbers() {
        let err = migrate(project(CURRENT_VERSION + 7)).unwrap_err();
        assert_eq!(
            err,
            MigrateError::TooNew {
                found: CURRENT_VERSION + 7,
                supported: CURRENT_VERSION
            }
        );
        // The message has to tell the user what to upgrade to.
        assert!(err.to_string().contains("newer"));
    }

    /// A v1 mix means "this much on both axes of every channel".
    ///
    /// v1 had one number per channel; v2 mixes each axis on its own. The old
    /// value applied to both axes, so that is what it migrates to — anything
    /// else silently changes how an existing rig poses.
    #[test]
    fn a_v1_mix_becomes_the_same_amount_on_both_axes() {
        let mut p = project(1);
        let mut c = constraint("shoulder");
        c.extra
            .insert("mixes".into(), serde_json::json!([0.25, 0.5, 0.75, 1.0]));
        p.constraints.push(c);

        let p = migrate(p).expect("v1 migrates");
        assert_eq!(p.version, CURRENT_VERSION);
        let mix = p.constraints[0].transform_mix.expect("the mix came across");
        assert_eq!(mix.rotate, 0.25);
        assert_eq!((mix.translate_x, mix.translate_y), (0.5, 0.5));
        assert_eq!((mix.scale_x, mix.scale_y), (0.75, 0.75));
        assert_eq!((mix.shear_x, mix.shear_y), (1.0, 1.0));
        // The v1 field is consumed, not left beside its replacement to be
        // written back out and read again by the next load.
        assert!(!p.constraints[0].extra.contains_key("mixes"));
    }

    /// A v2 file is not migrated twice.
    #[test]
    fn a_v2_mix_is_left_alone() {
        let mut p = project(CURRENT_VERSION);
        let mut c = constraint("shoulder");
        c.transform_mix = Some(schema::TransformMix {
            translate_x: -1.0,
            ..Default::default()
        });
        p.constraints.push(c);

        let p = migrate(p).expect("v2 passes through");
        let mix = p.constraints[0].transform_mix.expect("still there");
        // Untouched: a second pass would have filled y from x.
        assert_eq!((mix.translate_x, mix.translate_y), (-1.0, 0.0));
    }

    fn constraint(name: &str) -> schema::Constraint {
        schema::Constraint {
            name: name.into(),
            kind: "transform".into(),
            target: String::new(),
            bones: Vec::new(),
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
            stretch_limit: 1.1,
            stiffness: 0.0,
            transform_mix: None,
            offsets: None,
            local: false,
            relative: false,
            physics: None,
            forces: None,
            channels: None,
            slot: None,
            path: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn version_zero_is_rejected() {
        assert_eq!(
            migrate(project(0)).unwrap_err(),
            MigrateError::InvalidVersion { found: 0 }
        );
    }
}
