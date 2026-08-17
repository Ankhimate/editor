//! Crash recovery (T-701).
//!
//! An editor that loses an hour of work to a crash has taught you to save
//! compulsively, which is a tax it charges for the rest of its life. This writes
//! a spare copy on a timer so there is something to come back to.
//!
//! # What it is not
//!
//! Not a save. The autosave file sits **beside** the project under a different
//! extension and the project's own file is never touched — a background writer
//! that overwrites the thing you were editing is a way to lose two copies
//! instead of one. `current_path` does not move, the title bar does not change,
//! and the user is never told "saved" for something they did not ask for.
//!
//! # When it writes
//!
//! Only when the document has actually changed, tracked by
//! [`AppState::revision`], which every document mutation already bumps. A timer
//! alone would rewrite an untouched project every interval, and the one thing
//! worse than no autosave is one that burns a laptop's disk while you read.
//!
//! # Recovery
//!
//! On startup, an autosave newer than its project is offered. Offered, not
//! applied: the user knows whether the last session ended badly and we do not,
//! and silently opening a different file than the one that was double-clicked
//! is the kind of helpfulness people uninstall software over.
//!
//! An unsaved document autosaves into the temp directory, since there is no
//! project directory to sit beside yet.

use crate::app_state::AppState;
use std::path::{Path, PathBuf};

/// Extension appended to the project's own name: `walk.ankh.autosave`.
///
/// Appended rather than replacing `.ankh` so the file cannot be mistaken for a
/// project by a file dialog, and so a directory listing sorts it next to the
/// thing it belongs to.
const EXT: &str = "ankh.autosave";

/// How often to consider writing, in seconds.
///
/// Two minutes is the interval most editors settle on: short enough that what
/// you lose is an edit rather than an idea, long enough that a large rig's write
/// does not become the thing you notice about the program.
pub const DEFAULT_INTERVAL_SECS: u64 = 120;

/// The autosave path for a project, or for the unsaved document.
pub fn path_for(project: Option<&Path>) -> Option<PathBuf> {
    match project {
        Some(path) => {
            let name = path.file_name()?.to_str()?;
            Some(path.with_file_name(format!("{name}.autosave")))
        }
        // Nothing to sit beside yet. One fixed name rather than a unique one per
        // session: a temp directory quietly accumulating a file per crash is its
        // own bug, and the newest unsaved document is the only one anybody wants.
        None => Some(std::env::temp_dir().join(format!("untitled.{EXT}"))),
    }
}

/// Timer and dirty-tracking for the autosave.
pub struct Autosave {
    /// Seconds between attempts. Zero disables.
    pub interval_secs: u64,
    /// Seconds accumulated since the last attempt.
    elapsed: f32,
    /// `AppState::revision` as of the last successful write. `None` until one
    /// happens, so a document edited before the first tick still saves.
    saved_revision: Option<u64>,
    /// Where the last write went, for the status line and for cleanup.
    last_written: Option<PathBuf>,
}

impl Default for Autosave {
    fn default() -> Self {
        Self {
            interval_secs: DEFAULT_INTERVAL_SECS,
            elapsed: 0.0,
            saved_revision: None,
            last_written: None,
        }
    }
}

impl Autosave {
    /// Start with `interval_secs` between attempts; 0 disables.
    pub fn new(interval_secs: u64) -> Self {
        Self {
            interval_secs,
            ..Default::default()
        }
    }

    /// Advance the timer by a frame and report whether a write is due.
    ///
    /// `dt` comes from the frame clock rather than a wall-clock read, so this
    /// stays testable — see the module note in `core` about determinism. The
    /// same reasoning applies here for a different reason: a test that had to
    /// sleep two minutes would not be written.
    pub fn tick(&mut self, dt: f32, revision: u64) -> bool {
        if self.interval_secs == 0 {
            return false;
        }
        self.elapsed += dt;
        if self.elapsed < self.interval_secs as f32 {
            return false;
        }
        self.elapsed = 0.0;
        // Unchanged since the last write: nothing to do. Checked after the timer
        // resets so an idle document does not accumulate a backlog of attempts.
        self.saved_revision != Some(revision)
    }

    /// Write the autosave beside `project`, or to the temp directory.
    ///
    /// Failure is logged and swallowed. The user did not ask for this write, so
    /// interrupting them because it failed would be the autosave making their
    /// session worse — which is the opposite of the point.
    pub fn write(&mut self, state: &AppState, project: Option<&Path>) -> Option<PathBuf> {
        let path = path_for(project)?;
        match ankhimate_formats::save(&path, &state.doc.as_project_ref(), &[]) {
            Ok(()) => {
                self.saved_revision = Some(state.revision);
                self.last_written = Some(path.clone());
                Some(path)
            }
            Err(e) => {
                log::warn!("autosave to {} failed: {e}", path.display());
                None
            }
        }
    }

    /// Forget the current document's write history.
    ///
    /// Called when the document is replaced — a new or opened project shares
    /// nothing with the last one, and carrying its revision across would make
    /// the first tick think an untouched document was already saved.
    pub fn reset(&mut self) {
        self.elapsed = 0.0;
        self.saved_revision = None;
        self.last_written = None;
    }

    /// Delete the autosave for `project`, after a real save has superseded it.
    ///
    /// Best-effort and silent: a leftover autosave is harmless — recovery
    /// compares timestamps — while an error dialog about deleting a file the
    /// user never knew existed is not.
    pub fn discard(&mut self, project: Option<&Path>) {
        if let Some(path) = path_for(project)
            && path.exists()
            && let Err(e) = std::fs::remove_file(&path)
        {
            log::warn!("could not remove autosave {}: {e}", path.display());
        }
        self.last_written = None;
    }
}

/// An autosave worth offering at startup.
#[derive(Debug, Clone)]
pub struct Recovery {
    /// The autosave file.
    pub autosave: PathBuf,
    /// The project it belongs to, if it had one.
    pub project: Option<PathBuf>,
}

/// Is there an autosave newer than the project it shadows?
///
/// Newer, not merely present: an autosave older than the last real save has
/// already been superseded, and offering it would invite the user to restore
/// stale work over good work.
///
/// A project whose own mtime cannot be read is treated as *not* superseding the
/// autosave — the recovery is offered rather than hidden. Erring the other way
/// silently discards the only copy of a crashed session.
pub fn check(project: Option<&Path>) -> Option<Recovery> {
    let autosave = path_for(project)?;
    let auto_time = std::fs::metadata(&autosave).ok()?.modified().ok()?;

    if let Some(project) = project {
        let project_time = std::fs::metadata(project)
            .ok()
            .and_then(|m| m.modified().ok());
        if let Some(project_time) = project_time
            && project_time >= auto_time
        {
            return None;
        }
    }

    Some(Recovery {
        autosave,
        project: project.map(Path::to_path_buf),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_autosave_sits_beside_the_project_without_replacing_it() {
        let project = Path::new("/rigs/walk.ankh");
        let auto = path_for(Some(project)).unwrap();

        assert_eq!(auto.file_name().unwrap(), "walk.ankh.autosave");
        assert_eq!(auto.parent(), project.parent());
        assert_ne!(auto, project, "must never target the project itself");
    }

    #[test]
    fn an_unsaved_document_autosaves_to_the_temp_directory() {
        let auto = path_for(None).unwrap();
        assert_eq!(auto.parent().unwrap(), std::env::temp_dir());
    }

    #[test]
    fn nothing_is_due_before_the_interval_elapses() {
        let mut autosave = Autosave::new(10);
        assert!(!autosave.tick(9.0, 1));
        assert!(autosave.tick(1.5, 1), "crossing the interval is due");
    }

    #[test]
    fn an_untouched_document_is_not_rewritten() {
        // The reason the revision is consulted at all: a timer alone rewrites an
        // idle project every two minutes forever.
        let mut autosave = Autosave::new(10);
        autosave.saved_revision = Some(7);
        assert!(!autosave.tick(11.0, 7), "revision unchanged, nothing to do");
        assert!(autosave.tick(11.0, 8), "edited since, now due");
    }

    #[test]
    fn an_edit_before_the_first_write_still_saves() {
        // `saved_revision` starts as None rather than 0, so a document edited in
        // the first two minutes is not mistaken for one already written.
        let mut autosave = Autosave::new(10);
        assert!(autosave.tick(11.0, 0), "never written, so due");
    }

    #[test]
    fn a_zero_interval_disables_it() {
        let mut autosave = Autosave::new(0);
        assert!(!autosave.tick(10_000.0, 99));
    }

    #[test]
    fn resetting_forgets_the_previous_documents_revision() {
        // A new document starts at revision 0 too. Without the reset, the first
        // tick would compare against the *old* document's saved revision and
        // could decide there was nothing to write.
        let mut autosave = Autosave::new(10);
        autosave.saved_revision = Some(0);
        assert!(!autosave.tick(11.0, 0));

        autosave.reset();
        assert!(autosave.tick(11.0, 0), "a fresh document is unwritten");
    }

    #[test]
    fn a_missing_autosave_offers_no_recovery() {
        let dir = std::env::temp_dir().join("ankh_autosave_test_missing");
        let _ = std::fs::create_dir_all(&dir);
        let project = dir.join("nothing.ankh");
        assert!(check(Some(&project)).is_none());
    }

    #[test]
    fn an_autosave_older_than_the_project_is_not_offered() {
        // It has already been superseded by a real save; offering it invites the
        // user to restore stale work over good work.
        let dir = std::env::temp_dir().join("ankh_autosave_test_stale");
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("rig.ankh");
        let auto = dir.join("rig.ankh.autosave");

        std::fs::write(&auto, b"old").unwrap();
        // Sleep rather than set mtimes by hand: filesystem timestamp resolution
        // is coarse enough on Windows that two writes in the same instant
        // compare equal, which would make this pass for the wrong reason.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&project, b"newer").unwrap();

        assert!(check(Some(&project)).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_autosave_newer_than_the_project_is_offered() {
        let dir = std::env::temp_dir().join("ankh_autosave_test_fresh");
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("rig.ankh");
        let auto = dir.join("rig.ankh.autosave");

        std::fs::write(&project, b"saved").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&auto, b"crashed after this").unwrap();

        let recovery = check(Some(&project)).expect("newer autosave is offered");
        assert_eq!(recovery.autosave, auto);
        assert_eq!(recovery.project.as_deref(), Some(project.as_path()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_autosave_for_a_project_that_no_longer_exists_is_still_offered() {
        // The project file being gone is exactly the case where the autosave is
        // the only copy left. Hiding it here would discard the crashed session.
        let dir = std::env::temp_dir().join("ankh_autosave_test_orphan");
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("vanished.ankh");
        let auto = dir.join("vanished.ankh.autosave");
        std::fs::write(&auto, b"all that is left").unwrap();

        assert!(check(Some(&project)).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
