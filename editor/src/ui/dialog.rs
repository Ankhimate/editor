//! One dialog shell, used by every window the editor opens.
//!
//! The dialogs used to be eight hand-configured `egui::Window`s, which meant
//! eight slightly different answers to the same questions — some resizable and
//! some not, each with the default title bar and its own idea of padding, none
//! of them modal. A settings window you can click straight through while it is
//! open is not a dialog, it is a floating panel that happens to look like one.
//!
//! So: [`show`] is the only way a dialog gets on screen. It is modal (the
//! backdrop dims and swallows clicks), fixed-size, and wears the same card the
//! docked panels wear — same radius, same border, same header colour — so a
//! dialog reads as part of the program rather than as an OS window that wandered
//! in.
//!
//! Sizing is the caller's, through [`Dialog::width`] and friends, because it is
//! the one thing that genuinely differs: a rename prompt and a PSD import tree
//! do not want the same box. Everything else is fixed here on purpose.

use crate::theme::Theme;
use eframe::egui;

/// Padding inside the dialog body.
const BODY_PAD: i8 = 14;
/// Height of the dialog's own title strip.
const HEADER_H: f32 = 38.0;

/// What the user did with the dialog's own chrome this frame.
#[must_use]
pub struct DialogResponse<T> {
    /// Whatever the body returned.
    pub inner: T,
    /// The close button, the backdrop, or Escape. Callers that own an `open`
    /// flag should clear it on this.
    pub closed: bool,
}

/// A modal dialog, configured then shown.
pub struct Dialog<'a> {
    id: &'a str,
    title: &'a str,
    icon: Option<&'a str>,
    width: f32,
    max_height: Option<f32>,
    /// Can the backdrop or Escape dismiss it?
    dismissable: bool,
}

impl<'a> Dialog<'a> {
    /// A dialog titled `title`. `id` must be stable across frames.
    pub fn new(id: &'a str, title: &'a str) -> Self {
        Self {
            id,
            title,
            icon: None,
            width: 480.0,
            max_height: None,
            dismissable: true,
        }
    }

    /// A glyph shown before the title, from [`crate::ui::icons`].
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Cap the body height and scroll past it. Without this the dialog grows to
    /// fit its content, which is right for a prompt and wrong for a list that
    /// can run off the bottom of the screen.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Refuse the backdrop and Escape, leaving the body's own buttons as the way
    /// out. For dialogs where dismissing by accident loses work.
    pub fn dismissable(mut self, yes: bool) -> Self {
        self.dismissable = yes;
        self
    }

    pub fn show<T>(
        self,
        ctx: &egui::Context,
        theme: &Theme,
        body: impl FnOnce(&mut egui::Ui) -> T,
    ) -> DialogResponse<T> {
        let visuals = ctx.global_style().visuals.clone();
        let mut close = false;

        let modal = egui::Modal::new(egui::Id::new(self.id))
            // Darker than egui's default wash. The editor is already a dark UI,
            // and a faint scrim over a near-black canvas does not read as
            // "blocked" — it reads as a rendering glitch.
            .backdrop_color(egui::Color32::from_black_alpha(160))
            .frame(
                egui::Frame::NONE
                    .fill(visuals.panel_fill)
                    .stroke(egui::Stroke::new(1.0, theme.card_border()))
                    .corner_radius(super::CARD_RADIUS),
            )
            .show(ctx, |ui| {
                ui.set_width(self.width);
                ui.spacing_mut().item_spacing.y = 0.0;

                // ── Header ────────────────────────────────────────────────
                // The window background, matching the panels' tab strips, so a
                // dialog and a docked card have the same two-tone anatomy.
                let (header, _) =
                    ui.allocate_exact_size(egui::vec2(self.width, HEADER_H), egui::Sense::hover());
                ui.painter().rect_filled(
                    header,
                    // Only the top corners: the bottom two belong to the body.
                    egui::CornerRadius {
                        nw: super::CARD_RADIUS,
                        ne: super::CARD_RADIUS,
                        sw: 0,
                        se: 0,
                    },
                    theme.window_background(),
                );

                let text_pos = egui::pos2(header.min.x + 14.0, header.center().y);
                let title = match self.icon {
                    Some(icon) => format!("{icon}  {}", self.title),
                    None => self.title.to_string(),
                };
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    title,
                    egui::FontId::proportional(13.5),
                    visuals.text_color(),
                );

                // Close, in the corner the window controls use.
                let btn = egui::Rect::from_center_size(
                    egui::pos2(header.max.x - 22.0, header.center().y),
                    egui::vec2(26.0, 24.0),
                );
                let close_response = ui.interact(btn, ui.id().with("close"), egui::Sense::click());
                if close_response.hovered() {
                    ui.painter()
                        .rect_filled(btn, 5, visuals.error_fg_color.gamma_multiply(0.75));
                }
                ui.painter().text(
                    btn.center(),
                    egui::Align2::CENTER_CENTER,
                    super::icons::CLOSE,
                    egui::FontId::proportional(13.0),
                    if close_response.hovered() {
                        egui::Color32::WHITE
                    } else {
                        visuals.weak_text_color()
                    },
                );
                if close_response.clicked() {
                    close = true;
                }

                // A hairline under the header, for the same reason the cards
                // have one: without it the two fills read as a gradient.
                ui.painter().hline(
                    header.x_range(),
                    header.max.y,
                    egui::Stroke::new(1.0, theme.card_border()),
                );

                // ── Body ──────────────────────────────────────────────────
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(BODY_PAD))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        match self.max_height {
                            Some(height) => {
                                egui::ScrollArea::vertical()
                                    .max_height(height)
                                    .auto_shrink([false, true])
                                    .show(ui, body)
                                    .inner
                            }
                            None => body(ui),
                        }
                    })
                    .inner
            });

        // Backdrop and Escape, unless the caller opted out. `should_close`
        // folds both together.
        if self.dismissable && modal.should_close() {
            close = true;
        }

        DialogResponse {
            inner: modal.inner,
            closed: close,
        }
    }
}
