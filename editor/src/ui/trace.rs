//! The mesh tracing window (T-402).
//!
//! Detail and interior spacing are meaningless as bare numbers — "2.0" says
//! nothing about whether the outline will hug the art or cut its corners. The
//! window shows the traced outline over the image and the resulting point count,
//! so the settings are chosen by looking rather than by guessing and undoing.

use crate::app_state::AppState;
use ankhimate_core::ids::{SkinId, SlotId};
use ankhimate_document::meshgen;
use eframe::egui;

/// A trace being set up. Session state — cancelling leaves nothing behind.
#[derive(Clone)]
pub struct PendingTrace {
    pub skin: SkinId,
    pub slot: SlotId,
    pub name: String,
}

impl PendingTrace {
    pub fn new(skin: SkinId, slot: SlotId, name: String) -> Self {
        Self { skin, slot, name }
    }
}

const PREVIEW_KEY: &str = "trace_preview";

pub fn ui(ctx: &egui::Context, state: &mut AppState, theme: &crate::theme::Theme) {
    let Some(pending) = state.session.pending_trace.clone() else {
        return;
    };

    // Everything the preview needs, resolved once.
    let Some(bytes) = mesh_texture_bytes(state, &pending) else {
        state
            .session
            .set_status("No image to trace for this attachment");
        state.session.pending_trace = None;
        return;
    };
    let Some(image) = image::load_from_memory(&bytes).ok().map(|i| i.to_rgba8()) else {
        state.session.set_status("Could not decode the image");
        state.session.pending_trace = None;
        return;
    };

    let mut options = state.session.trace_options;
    // Retrace as the dials move; refine only when asked, so interior points do
    // not silently reappear after the outline is re-cut.
    let outline = meshgen::trace(&image, options);
    let traced = match (&outline, state.session.trace_refined) {
        (Some(traced), true) => Some(meshgen::refine(traced, options)),
        (Some(traced), false) => Some(meshgen::Traced {
            contours: traced.contours.clone(),
            interior: Vec::new(),
        }),
        (None, _) => None,
    };
    let mut apply = false;
    let mut close = false;

    let dialog = crate::ui::dialog::Dialog::new("trace_mesh", "Trace mesh")
        .icon(crate::ui::icons::MESH)
        .width(620.0)
        .show(ctx, theme, |ui| {
            ui.horizontal_top(|ui| {
                preview(ui, state, theme, &image, traced.as_ref());

                ui.vertical(|ui| {
                    ui.set_min_width(210.0);
                    ui.label(egui::RichText::new(&pending.name).strong());
                    ui.label(
                        egui::RichText::new(format!("{}×{} px", image.width(), image.height()))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(8.0);

                    // Sliders, not spinboxes: these are feel dials with no
                    // meaningful unit, and a slider says "sweep me and watch".
                    slider(
                        ui,
                        "Detail",
                        &mut options.detail,
                        0.0..=100.0,
                        "How many vertices surround the shape. More detail follows the \
                         silhouette more closely.",
                    );
                    slider(
                        ui,
                        "Concavity",
                        &mut options.concavity,
                        0.0..=100.0,
                        "Prioritises placing vertices into concave areas — the notches and gaps \
                         a plain simplification flattens first",
                    );
                    slider(
                        ui,
                        "Refinement",
                        &mut options.refinement,
                        0.0..=100.0,
                        "The time and effort to spend finding an optimal solution.\n\
                         Slides the vertices along the silhouette to where they describe it \
                         best. Does not change how many there are — that is Detail.",
                    );
                    slider(
                        ui,
                        "Uniform",
                        &mut options.uniform,
                        0.0..=1.0,
                        "Sub-divides long edges for more uniform spacing. Even spacing deforms \
                         more predictably; a long edge is a hinge that cannot bend.",
                    );
                    slider(
                        ui,
                        "Interior",
                        &mut options.interior,
                        0.0..=100.0,
                        "Density of the interior vertices added by Refine.\n\
                         Interior points are what let a mesh bend in the middle rather than \
                         only at its edges.",
                    );

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Alpha threshold");
                        let mut threshold = options.alpha_threshold as f32;
                        if ui
                            .add(egui::DragValue::new(&mut threshold).range(1.0..=255.0))
                            .on_hover_text("Pixels at or above this alpha count as solid")
                            .changed()
                        {
                            options.alpha_threshold = threshold as u8;
                        }
                        ui.label("Padding");
                        ui.add(
                            egui::DragValue::new(&mut options.padding)
                                .range(0.0..=20.0)
                                .speed(0.1),
                        )
                        .on_hover_text("Push the outline outward so edge pixels are not clipped");
                    });

                    ui.add_space(8.0);
                    match &traced {
                        Some(traced) => {
                            let outline: usize = traced.contours.iter().map(|c| c.len()).sum();
                            let total = outline + traced.interior.len();
                            ui.label(
                                egui::RichText::new(format!(
                                    "Vertices: {total}   ({outline} outline, {} interior, \
                                     {} contour(s))",
                                    traced.interior.len(),
                                    traced.contours.len()
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        }
                        None => {
                            ui.label(
                                egui::RichText::new(
                                    "Nothing to trace at these settings — try a lower alpha \
                                     threshold, or less detail on a very large image",
                                )
                                .small()
                                .color(egui::Color32::from_rgb(230, 150, 60)),
                            );
                        }
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        // Trace re-cuts the outline; Refine fills it. Two steps
                        // because changing interior density should not throw the
                        // silhouette away and start over.
                        if ui
                            .add_enabled(outline.is_some(), egui::Button::new("Trace"))
                            .on_hover_text("Re-cut the outline from the artwork")
                            .clicked()
                        {
                            state.session.trace_refined = false;
                        }
                        if ui
                            .add_enabled(outline.is_some(), egui::Button::new("Refine"))
                            .on_hover_text("Add interior vertices at the Refinement density")
                            .clicked()
                        {
                            state.session.trace_refined = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(traced.is_some(), egui::Button::new("OK"))
                                .on_hover_text(
                                    "Replace the mesh. Weights are cleared — the vertices they \
                                     referred to are gone.",
                                )
                                .clicked()
                            {
                                apply = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
                });
            });
        });

    state.session.trace_options = options;

    if apply && let Some(traced) = traced {
        apply_trace(state, &pending, &traced);
        close = true;
    }
    close |= dialog.closed;
    if close {
        state.session.pending_trace = None;
        state.session.thumbnails.remove(PREVIEW_KEY);
    }
}

/// One labelled slider row, with the value shown numerically beside it.
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    hover: &str,
) {
    ui.horizontal(|ui| {
        ui.add_sized([90.0, 18.0], egui::Label::new(label));
        let decimals = if *range.end() <= 1.0 { 2 } else { 0 };
        ui.add(
            egui::Slider::new(value, range)
                .max_decimals(decimals)
                .clamping(egui::SliderClamping::Always),
        )
        .on_hover_text(hover);
    });
}

/// The image with the traced outline drawn over it.
fn preview(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &crate::theme::Theme,
    image: &image::RgbaImage,
    traced: Option<&meshgen::Traced>,
) {
    const MAX: f32 = 300.0;

    let handle = match state.session.thumbnails.get(PREVIEW_KEY) {
        Some(handle) => Some(handle.clone()),
        None => {
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            );
            let handle = ui
                .ctx()
                .load_texture(PREVIEW_KEY, color, egui::TextureOptions::LINEAR);
            state
                .session
                .thumbnails
                .insert(PREVIEW_KEY.to_string(), handle.clone());
            Some(handle)
        }
    };

    let scale = (MAX / image.width().max(1) as f32).min(MAX / image.height().max(1) as f32);
    let size = egui::vec2(image.width() as f32 * scale, image.height() as f32 * scale);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if let Some(handle) = handle {
        egui::Image::new(&handle).paint_at(ui, rect);
    }

    let Some(traced) = traced else {
        return;
    };
    let painter = ui.painter_at(rect);
    // Contours are normalized, so the same numbers scale to any preview size.
    let to_pos = |p: glam::Vec2| {
        egui::pos2(
            rect.min.x + p.x * rect.width(),
            rect.min.y + p.y * rect.height(),
        )
    };
    for contour in &traced.contours {
        for i in 0..contour.len() {
            let a = to_pos(contour[i]);
            let b = to_pos(contour[(i + 1) % contour.len()]);
            painter.line_segment([a, b], egui::Stroke::new(1.5, theme.mesh_edge()));
        }
    }
    for point in &traced.interior {
        painter.circle_filled(to_pos(*point), 1.5, theme.mesh_vertex_selected());
    }
}

fn mesh_texture_bytes(state: &AppState, pending: &PendingTrace) -> Option<Vec<u8>> {
    use ankhimate_core::attachment::Attachment;
    let Attachment::Mesh(mesh) = state
        .doc
        .skeleton
        .skins
        .get(pending.skin)?
        .get(pending.slot, &pending.name)?
    else {
        return None;
    };
    let id = state.doc.assets.by_name(&mesh.texture)?;
    state.doc.assets.get(id).map(|a| a.bytes.clone())
}

fn apply_trace(state: &mut AppState, pending: &PendingTrace, traced: &meshgen::Traced) {
    use ankhimate_core::attachment::Attachment;

    let Some(Attachment::Mesh(mesh)) = state
        .doc
        .skeleton
        .skins
        .get(pending.skin)
        .and_then(|s| s.get(pending.slot, &pending.name))
    else {
        return;
    };
    let had_weights = !mesh.weights.is_empty();

    // Preserve the mesh's UV-to-local frame. Using its axis-aligned bounds here
    // discarded rotation from a converted region and spread the texture across
    // the larger box.
    let (vertices, uvs, triangles) = meshgen::mesh_from_trace_on_mesh(traced, mesh);
    if triangles.is_empty() {
        state
            .session
            .set_status("Tracing produced no triangles — try a coarser detail value");
        return;
    }
    let (vertex_count, triangle_count) = (vertices.len(), triangles.len());

    if state.dispatch(Box::new(
        ankhimate_document::commands::mesh_cmds::TraceMesh::new(
            pending.skin,
            pending.slot,
            pending.name.clone(),
            vertices,
            uvs,
            triangles,
        ),
    )) {
        state.session.selected_vertices.clear();
        state.session.set_status(if had_weights {
            format!(
                "Traced {vertex_count} vertices, {triangle_count} triangles — weights were cleared"
            )
        } else {
            format!("Traced {vertex_count} vertices, {triangle_count} triangles")
        });
    }
}
