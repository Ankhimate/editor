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

/// v1 → v2: constraint mixes gain a per-axis form.
///
/// v1 stored `mixes: [rotate, translate, scale, shear]` — one number per
/// channel, applied to both axes of the channels that have two. v2 mixes each
/// axis on its own, so a v1 value means "this much on both axes", which is
/// exactly [`schema::TransformMix`] with the pair filled from one number.
///
/// The v1 field is read out of `extra`: v2's `Constraint` no longer names
/// `mixes`, so serde routes the unknown key into the catch-all rather than
/// failing the parse. That is the same mechanism that lets a v2 file survive a
/// round-trip through a v1 editor, used here in the other direction.
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

    // Mix timelines keyed four values per key; each becomes the same pair-filled
    // form. The v1 keys parsed as `MixKey` would read every field as absent, so
    // they arrive here as zeros and have to be rebuilt from `extra` too.
    for animation in &mut project.animations {
        for timeline in &mut animation.timelines {
            if let schema::Timeline::TransformConstraintMix { keys, .. } = timeline {
                for key in keys.iter_mut() {
                    let Some(old) = key.extra.remove("value") else {
                        continue;
                    };
                    let Some(values) = old.as_array() else {
                        continue;
                    };
                    let at =
                        |i: usize| values.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                    let (rotate, translate, scale, shear) = (at(0), at(1), at(2), at(3));
                    key.value = schema::TransformMix {
                        rotate,
                        translate_x: translate,
                        translate_y: translate,
                        scale_x: scale,
                        scale_y: scale,
                        shear_x: shear,
                        shear_y: shear,
                    };
                }
            }
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
