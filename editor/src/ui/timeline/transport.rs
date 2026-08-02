//! Transport controls: play/pause, loop, FPS, frame step, key jump (T-202).
//!
//! Buttons only — the keyboard shortcuts (space, arrows) are handled globally in
//! `app.rs` so they work regardless of which panel has focus, matching every DCC
//! tool's muscle memory. Playhead advancement itself lives in
//! [`AppState::advance_playback`], driven by the eframe frame clock.

use crate::app_state::AppState;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    use egui_phosphor::regular as icon;

    // Go to start.
    if ui.button(icon::SKIP_BACK).on_hover_text("Start").clicked() {
        state.session.playing = false;
        state.set_playhead(0.0);
    }
    // Previous key.
    if ui
        .button(icon::CARET_LEFT)
        .on_hover_text("Previous key (Ctrl+←)")
        .clicked()
    {
        state.jump_key(false);
    }
    // Step back one frame.
    if ui
        .button(icon::REWIND)
        .on_hover_text("Step back (←)")
        .clicked()
    {
        state.step_frames(-1);
    }

    // Play / pause.
    let playing = state.session.playing;
    let play_icon = if playing { icon::PAUSE } else { icon::PLAY };
    if ui
        .button(play_icon)
        .on_hover_text("Play/Pause (Space)")
        .clicked()
    {
        toggle_play(state);
    }

    // Step forward one frame.
    if ui
        .button(icon::FAST_FORWARD)
        .on_hover_text("Step forward (→)")
        .clicked()
    {
        state.step_frames(1);
    }
    // Next key.
    if ui
        .button(icon::CARET_RIGHT)
        .on_hover_text("Next key (Ctrl+→)")
        .clicked()
    {
        state.jump_key(true);
    }
    // Go to end.
    if ui.button(icon::SKIP_FORWARD).on_hover_text("End").clicked()
        && let Some(dur) = state
            .session
            .active_animation
            .and_then(|id| state.doc.animations.get(id))
            .map(|a| a.duration)
    {
        state.session.playing = false;
        state.set_playhead(dur);
    }

    ui.separator();

    // Loop toggle.
    let mut looping = state.session.looping;
    if ui
        .selectable_label(looping, icon::REPEAT)
        .on_hover_text("Loop")
        .clicked()
    {
        looping = !looping;
        state.session.looping = looping;
    }

    // Auto-key toggle. On by default in Animate mode (T-207): turning it off
    // holds a pose as an unkeyed preview until `K` commits it, which is the
    // escape hatch for "let me look at this before I key it".
    let auto = state.session.auto_key;
    let auto_resp = ui
        .selectable_label(auto, icon::RECORD)
        .on_hover_text(if auto {
            "Auto-key on: posing writes keys at the playhead"
        } else {
            "Auto-key off: posing is held until you press K"
        });
    if auto_resp.clicked() {
        state.session.auto_key = !auto;
    }

    // Key the pending pose — the manual counterpart to auto-key.
    if ui
        .add_enabled(
            state.session.has_pending_pose(),
            egui::Button::new(icon::KEY),
        )
        .on_hover_text("Key the current pose (K)")
        .clicked()
    {
        state.key_pending_pose();
    }

    ui.separator();

    // FPS spinbox.
    ui.label("fps");
    let mut fps = state.doc.meta.fps as i32;
    if ui
        .add(egui::DragValue::new(&mut fps).range(1..=240).speed(1))
        .changed()
    {
        state.doc.meta.fps = fps.max(1) as u32;
    }

    ui.separator();
    // Current time readout in frames.
    let frame = (state.session.playhead * state.doc.meta.fps as f32).round() as i64;
    ui.label(
        egui::RichText::new(format!("f{frame}"))
            .monospace()
            .color(ui.visuals().weak_text_color()),
    );
}

/// Toggle playback; starting from the end rewinds so play always moves.
pub fn toggle_play(state: &mut AppState) {
    if !state.session.playing {
        let at_end = state
            .session
            .active_animation
            .and_then(|id| state.doc.animations.get(id))
            .map(|a| state.session.playhead >= a.duration - 1e-4)
            .unwrap_or(false);
        if at_end {
            state.session.playhead = 0.0;
        }
    }
    state.session.playing = !state.session.playing;
}
