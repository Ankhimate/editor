//! Version migration (PLAN §6.1, ADR 0004).
//!
//! `.ankh` files carry a mandatory `version`. Migration runs before conversion to
//! the core model, stepping a parsed document forward one version at a time.
//!
//! v1 is current, so every arm is identity today. The scaffold exists so adding v2
//! is a known extension point rather than a redesign — the mistake this avoids is
//! shipping a format with no migration story and discovering it at v2.

use crate::schema::{CURRENT_VERSION, Project};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MigrateError {
    /// The file was written by a newer version of the editor.
    #[error("project version {found} is newer than supported version {supported}")]
    TooNew { found: u32, supported: u32 },
    /// Version 0 does not exist; the field was missing or corrupt.
    #[error("project version 0 is not valid")]
    InvalidVersion,
}

/// Bring a parsed project up to [`CURRENT_VERSION`].
pub fn migrate(project: Project) -> Result<Project, MigrateError> {
    if project.version == 0 {
        return Err(MigrateError::InvalidVersion);
    }
    if project.version > CURRENT_VERSION {
        return Err(MigrateError::TooNew {
            found: project.version,
            supported: CURRENT_VERSION,
        });
    }

    // Only v1 exists, so there is nothing to step through yet. When v2 lands,
    // replace this with a `while project.version < CURRENT_VERSION` loop whose
    // arms mutate `project` and bump `project.version` (v1→v2→v3). Until then a
    // loop would be a clippy `while_immutable_condition` since no arm writes the
    // field.
    debug_assert_eq!(project.version, CURRENT_VERSION);
    Ok(project)
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
            groups: Vec::new(),
            extra: Default::default(),
        }
    }

    #[test]
    fn current_version_passes_through() {
        let p = migrate(project(CURRENT_VERSION)).expect("v1 is current");
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

    #[test]
    fn version_zero_is_rejected() {
        assert_eq!(
            migrate(project(0)).unwrap_err(),
            MigrateError::InvalidVersion
        );
    }
}
