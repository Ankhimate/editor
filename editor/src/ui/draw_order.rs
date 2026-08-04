//! Draw-order panel (T-204, PLAN §2.3, §2.6).
//!
//! Lists the slots in **paint order at the current playhead**, front-most at the
//! top (the last-drawn slot sits on top of the stack, so top-of-list = front).
//! Reordering respects the mode contract (T-207):
//!
//! * **Setup** → edits `Skeleton.draw_order`, the setup stack;
//! * **Animate** → writes a `DrawOrder` key (offsets vs. the setup order), so the
//!   change is animated and the setup stack is left alone.
//!
//! Both paths go through [`AppState::commit_draw_order`], so this panel does not
//! know which command it produced.
//!
//! Rows reorder via up/down buttons; drag-and-drop is a later polish pass.

use crate::app_state::AppState;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    // The pose's draw order reflects any active DrawOrder key at the playhead; it
    // is what the viewport actually paints, so it is what we edit against.
    let order: Vec<_> = state.pose.draw_order.clone();

    if order.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 2.0 - 20.0);
            ui.label(egui::RichText::new("No slots yet").color(ui.visuals().weak_text_color()));
        });
        return;
    }

    let animating = state.session.is_animating();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Draw order").strong());
        ui.label(
            egui::RichText::new(if animating { "(animating)" } else { "(setup)" })
                .small()
                .color(ui.visuals().weak_text_color()),
        )
        .on_hover_text(if animating {
            "Reordering writes a draw-order key at the playhead"
        } else {
            "Reordering edits the setup stack"
        });
    });
    ui.separator();

    // Present front-to-back: draw_order is back→front, so iterate reversed.
    let front_to_back: Vec<_> = order.iter().copied().rev().collect();
    let mut swap: Option<(usize, usize)> = None; // indices in `front_to_back`

    for (i, slot_id) in front_to_back.iter().enumerate() {
        let name = state
            .doc
            .skeleton
            .slots
            .get(*slot_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "?".to_string());
        ui.horizontal(|ui| {
            // Up = toward front (earlier in this list).
            if ui
                .add_enabled(i > 0, egui::Button::new(crate::ui::icons::CARET_UP))
                .clicked()
            {
                swap = Some((i, i - 1));
            }
            if ui
                .add_enabled(
                    i + 1 < front_to_back.len(),
                    egui::Button::new(crate::ui::icons::CARET_DOWN),
                )
                .clicked()
            {
                swap = Some((i, i + 1));
            }
            ui.label(name);
        });
    }

    if let Some((a, b)) = swap {
        let mut new_front = front_to_back;
        new_front.swap(a, b);
        // Back to back→front storage order.
        let new_order: Vec<_> = new_front.into_iter().rev().collect();
        state.commit_draw_order(new_order);
    }
}
