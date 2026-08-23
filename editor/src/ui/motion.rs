//! Shared UI motion roles.
//!
//! Motion is session feedback, never document state. Keeping the timings here
//! prevents panels from growing their own unrelated collection of durations.

use eframe::egui;

fn reduced_motion_id() -> egui::Id {
    egui::Id::new("ankhimate_reduced_motion")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Instant,
    Quick,
    Shift,
    Enter,
}

impl Role {
    pub const fn seconds(self) -> f32 {
        match self {
            Self::Instant => 0.0,
            Self::Quick => 0.10,
            Self::Shift => 0.14,
            Self::Enter => 0.18,
        }
    }
}

/// Install the user's preference for this frame.
pub fn configure(ctx: &egui::Context, reduced: bool) {
    ctx.data_mut(|data| data.insert_temp(reduced_motion_id(), reduced));
}

pub fn reduced(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(reduced_motion_id()))
        .unwrap_or(false)
}

/// Animate a boolean state using the shared duration for its role.
pub fn factor(ctx: &egui::Context, id: egui::Id, value: bool, role: Role) -> f32 {
    let seconds = if reduced(ctx) {
        match role {
            Role::Instant => 0.0,
            _ => 0.06,
        }
    } else {
        role.seconds()
    };
    if seconds == 0.0 {
        f32::from(value)
    } else {
        ctx.animate_bool_with_time_and_easing(id, value, seconds, egui::emath::easing::cubic_out)
    }
}

/// Entrance progress that restarts after the surface has been absent for a
/// frame. `animate_bool` otherwise remembers its final `true` forever, making a
/// dialog animate only the first time it is opened.
pub fn entrance(ctx: &egui::Context, id: egui::Id, role: Role) -> f32 {
    let frame = ctx.cumulative_frame_nr();
    let seen_id = id.with("last_seen");
    let last_seen = ctx.data(|data| data.get_temp::<u64>(seen_id));
    if last_seen.is_none_or(|last| last.saturating_add(1) < frame) {
        factor(ctx, id, false, role);
    }
    ctx.data_mut(|data| data.insert_temp(seen_id, frame));
    factor(ctx, id, true, role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_manipulation_is_the_only_instant_role() {
        assert_eq!(Role::Instant.seconds(), 0.0);
        assert!(Role::Quick.seconds() > 0.0);
        assert!(Role::Shift.seconds() > Role::Quick.seconds());
        assert!(Role::Enter.seconds() > Role::Shift.seconds());
    }

    #[test]
    fn reduced_motion_is_context_local() {
        let ctx = egui::Context::default();
        assert!(!reduced(&ctx));
        configure(&ctx, true);
        assert!(reduced(&ctx));
    }
}
