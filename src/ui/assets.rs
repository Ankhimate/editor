//! The asset panel: the document's image library (T-301).
//!
//! Thumbnails are egui textures built lazily from the same bytes the wgpu
//! renderer uploads, cached by content hash in [`Session`](crate::session::Session)
//! so scrolling the panel does not re-decode PNGs every frame.
//!
//! Everything here is structural, so the panel is read-only in Animate mode
//! (T-207) — you can look at what the rig is made of while animating, but not
//! change it.

use crate::app_state::AppState;
use ankhimate_core::ids::AssetId;
use ankhimate_document::commands::asset_cmds::{
    DeleteAsset, ImportImage, RenameAsset, ReplaceAssetPixels,
};
use eframe::egui;

const THUMB: f32 = 64.0;

/// What the asset's original file looks like right now (T-306).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceState {
    /// Imported from a file that still matches what we hold.
    InSync,
    /// The file changed on disk since import — a reload would pick it up.
    Stale,
    /// The path is recorded but the file is gone (moved, or another machine).
    Missing,
    /// No path: the asset arrived inside an `.ankh`, which is not a problem.
    Embedded,
}

/// Compare an asset against its source file.
///
/// Bytes rather than timestamps: a mtime says a file was touched, not that its
/// contents differ, and an editor that keeps announcing false changes gets
/// ignored. Only called on demand — this reads every source from disk.
fn source_state(asset: &ankhimate_core::assets::ImageAsset) -> SourceState {
    let Some(path) = &asset.source_path else {
        return SourceState::Embedded;
    };
    match std::fs::read(path) {
        Ok(bytes) if bytes == asset.bytes => SourceState::InSync,
        Ok(_) => SourceState::Stale,
        Err(_) => SourceState::Missing,
    }
}

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    let setup = state.session.can_edit_structure();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Assets").strong());
        ui.label(
            egui::RichText::new(format!("({})", state.doc.assets.len()))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Icon only, outlined in the accent. The row is four actions of equal
            // weight — importing images, a sheet, a PSD, and re-reading sources —
            // and one of them spelled out as "＋ Import" made it look like the
            // primary action and the rest like its options.
            let hint = |enabled: bool, text: &'static str| {
                if enabled {
                    text
                } else {
                    "Switch to Setup mode (Tab)"
                }
            };
            if action_button(ui, crate::ui::icons::ADD, setup)
                .on_hover_text(hint(setup, "Import image files into the library"))
                .clicked()
            {
                import_dialog(state);
            }
            // Checking sources reads every file, so it is a button rather than
            // something that happens quietly every frame.
            let can_check = setup && !state.doc.assets.is_empty();
            if action_button(ui, crate::ui::icons::REFRESH, can_check)
                .on_hover_text(hint(can_check, "Check source files for changes"))
                .clicked()
            {
                check_sources(state);
            }
            if action_button(ui, crate::ui::icons::GRID, setup)
                .on_hover_text(hint(setup, "Import a spritesheet and slice it into cells"))
                .clicked()
            {
                open_sheet_dialog(state);
            }
            if action_button(ui, crate::ui::icons::DRAW_ORDER, setup)
                .on_hover_text(hint(setup, "Import a layered PSD as a rig"))
                .clicked()
            {
                open_psd_dialog(state);
            }
        });
    });
    ui.separator();

    if state.doc.assets.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(crate::ui::icons::ASSETS)
                    .size(28.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);
            ui.label(egui::RichText::new("No images yet").color(ui.visuals().weak_text_color()));
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Drop PNG/JPG/WebP onto the viewport, or use Import")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    }

    // Sorted by name so the grid does not reshuffle as assets are added.
    let mut ids: Vec<AssetId> = state.doc.assets.images.keys().collect();
    ids.sort_by_key(|&id| {
        state
            .doc
            .assets
            .get(id)
            .map(|a| a.name.clone())
            .unwrap_or_default()
    });

    let mut attach: Option<AssetId> = None;
    let mut delete: Option<AssetId> = None;
    let mut rename: Option<(AssetId, String)> = None;
    let mut reload: Option<AssetId> = None;
    let mut relink: Option<AssetId> = None;

    for id in ids {
        let Some(asset) = state.doc.assets.get(id) else {
            continue;
        };
        let name = asset.name.clone();
        let dims = format!("{}×{}", asset.width, asset.height);
        let missing = asset.bytes.is_empty();
        let texture = thumbnail(ui.ctx(), state, id);

        ui.horizontal(|ui| {
            match texture {
                Some(handle) => {
                    ui.add(
                        egui::Image::new(&handle)
                            .fit_to_exact_size(egui::vec2(THUMB, THUMB))
                            .maintain_aspect_ratio(true),
                    );
                }
                None => {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(THUMB, THUMB), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        egui::epaint::CornerRadius::same(3),
                        ui.visuals().extreme_bg_color,
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if missing { "?" } else { "…" },
                        egui::FontId::proportional(18.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }

            ui.vertical(|ui| {
                let mut edited = name.clone();
                let field = ui.add_enabled(
                    setup,
                    egui::TextEdit::singleline(&mut edited).desired_width(140.0),
                );
                if field.lost_focus() && edited != name && !edited.trim().is_empty() {
                    rename = Some((id, edited.trim().to_string()));
                }
                ui.label(
                    egui::RichText::new(if missing {
                        format!("{dims} · pixels missing")
                    } else {
                        dims.clone()
                    })
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
                // Source status (T-306), only once the user has asked for a
                // check — an unchecked asset says nothing rather than guessing.
                if let Some(flag) = state.session.stale_assets.get(id).copied() {
                    let (text, color) = match flag {
                        true => (
                            "source changed — reload to pick it up",
                            egui::Color32::from_rgb(235, 170, 60),
                        ),
                        false => ("source missing", egui::Color32::from_rgb(230, 90, 90)),
                    };
                    ui.label(egui::RichText::new(text).small().color(color));
                }

                let action_width = ((ui.available_width() - 6.0) / 2.0).max(74.0);
                ui.columns(2, |columns| {
                    let can_attach = setup && state.session.active_bone().is_some();
                    let btn = columns[0].add_enabled(
                        can_attach,
                        egui::Button::new(format!("{}  Attach", crate::ui::icons::ATTACHMENT))
                            .min_size(egui::vec2(action_width, crate::ui::CONTROL_HEIGHT)),
                    );
                    let btn = if !setup {
                        btn.on_hover_text("Switch to Setup mode to attach (Tab)")
                    } else if !can_attach {
                        btn.on_hover_text("Select a bone to attach this to")
                    } else {
                        btn.on_hover_text("Add a slot on the selected bone showing this image")
                    };
                    if btn.clicked() {
                        attach = Some(id);
                    }
                    if columns[1]
                        .add_enabled(
                            setup,
                            egui::Button::new(format!("{}  Delete", crate::ui::icons::DELETE))
                                .min_size(egui::vec2(action_width, crate::ui::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        delete = Some(id);
                    }
                });
                ui.columns(2, |columns| {
                    let has_source = state
                        .doc
                        .assets
                        .get(id)
                        .is_some_and(|a| a.source_path.is_some());
                    if columns[0]
                        .add_enabled(
                            setup && has_source,
                            egui::Button::new(format!("{}  Reload", crate::ui::icons::REFRESH))
                                .min_size(egui::vec2(action_width, crate::ui::CONTROL_HEIGHT)),
                        )
                        .on_hover_text("Re-read the file this was imported from")
                        .clicked()
                    {
                        reload = Some(id);
                    }
                    if columns[1]
                        .add_enabled(
                            setup,
                            egui::Button::new(format!("{}  Relink…", crate::ui::icons::RELINK))
                                .min_size(egui::vec2(action_width, crate::ui::CONTROL_HEIGHT)),
                        )
                        .on_hover_text("Point this asset at a different file, keeping its name")
                        .clicked()
                    {
                        relink = Some(id);
                    }
                });
            });
        });
        ui.add_space(4.0);
    }

    if let Some((id, new_name)) = rename {
        state.dispatch(Box::new(RenameAsset::new(id, new_name)));
    }
    if let Some(id) = attach
        && let Some(bone) = state.session.active_bone()
        && let Some(asset) = state.doc.assets.get(id).cloned()
    {
        // Attaching re-imports the same pixels under a uniquified name rather
        // than sharing the asset: sharing needs an attachment that references an
        // asset id, which arrives with the mesh work (T-401).
        if state.dispatch(Box::new(ImportImage::new(asset, bone, glam::Vec2::ZERO)))
            && let Some(&slot) = state.doc.skeleton.draw_order.last()
        {
            state.session.select_slot(Some(slot));
        }
    }
    if let Some(id) = reload {
        let path = state
            .doc
            .assets
            .get(id)
            .and_then(|a| a.source_path.clone())
            .map(std::path::PathBuf::from);
        if let Some(path) = path {
            replace_pixels_from(state, id, &path, false);
        }
    }
    if let Some(id) = relink
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
    {
        replace_pixels_from(state, id, &path, true);
    }
    if let Some(id) = delete {
        let uses = attachment_uses(state, id);
        if uses > 0 {
            state.session.set_status(format!(
                "Deleted an image still used by {uses} attachment(s) — they will not draw"
            ));
        }
        state.dispatch(Box::new(DeleteAsset::new(id)));
    }
}

/// Read `path` and swap it into the asset, as a reload or a relink (T-306).
fn replace_pixels_from(state: &mut AppState, id: AssetId, path: &std::path::Path, relink: bool) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            state
                .session
                .set_status(format!("Could not read {}: {e}", path.display()));
            return;
        }
    };
    let (width, height) = match image::load_from_memory(&bytes) {
        Ok(img) => (img.width(), img.height()),
        Err(e) => {
            state
                .session
                .set_status(format!("{} is not a supported image: {e}", path.display()));
            return;
        }
    };

    let cmd = if relink {
        ReplaceAssetPixels::relink(
            id,
            bytes,
            width,
            height,
            path.to_string_lossy().into_owned(),
        )
    } else {
        ReplaceAssetPixels::reload(id, bytes, width, height)
    };
    if state.dispatch(Box::new(cmd)) {
        // The GPU and thumbnail caches are content-keyed, so new pixels get a
        // new key on their own — but this asset's memoized key is now stale.
        state.session.texture_keys.remove(id);
        state.session.stale_assets.remove(id);
        state.session.thumbnails.clear();
        state
            .session
            .set_status(if relink { "Relinked" } else { "Reloaded" });
    }
}

/// Pick a spritesheet and stage it for slicing (T-305).
fn open_sheet_dialog(state: &mut AppState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
        .pick_file()
    else {
        return;
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            state
                .session
                .set_status(format!("Could not read {}: {e}", path.display()));
            return;
        }
    };
    let (width, height) = match image::load_from_memory(&bytes) {
        Ok(img) => (img.width(), img.height()),
        Err(e) => {
            state
                .session
                .set_status(format!("{} is not a supported image: {e}", path.display()));
            return;
        }
    };

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sheet")
        .to_string();
    let mut pending = crate::ui::atlas::PendingAtlas::new(name, bytes, width, height);
    pending.source_path = Some(path.to_string_lossy().into_owned());
    // A stale preview from a previous sheet would show the wrong picture.
    state.session.thumbnails.remove("atlas_preview");
    state.session.pending_atlas = Some(pending);
}

/// Pick a PSD and stage it for import (T-302).
fn open_psd_dialog(state: &mut AppState) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Photoshop", &["psd", "psb"])
        .pick_file()
    else {
        return;
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            state
                .session
                .set_status(format!("Could not read {}: {e}", path.display()));
            return;
        }
    };
    // The tree is read up front so the modal can show the document before
    // committing to anything — and so an unreadable file fails here, with the
    // file name still in hand, rather than inside the import.
    let nodes = match ankhimate_formats::psd::layer_tree(&bytes) {
        Ok(nodes) => nodes,
        Err(e) => {
            state.session.set_status(format!("{}: {e}", path.display()));
            return;
        }
    };
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("psd")
        .to_string();
    let mut pending = crate::ui::psd_import::PendingPsd::new(name, bytes, nodes);
    pending.source_path = Some(path.to_string_lossy().into_owned());
    state.session.pending_psd = Some(pending);
}

/// Compare every asset against its source file and record what differs.
fn check_sources(state: &mut AppState) {
    state.session.stale_assets.clear();
    let ids: Vec<AssetId> = state.doc.assets.images.keys().collect();
    let (mut stale, mut missing) = (0, 0);
    for id in ids {
        let Some(asset) = state.doc.assets.get(id) else {
            continue;
        };
        match source_state(asset) {
            SourceState::Stale => {
                state.session.stale_assets.insert(id, true);
                stale += 1;
            }
            SourceState::Missing => {
                state.session.stale_assets.insert(id, false);
                missing += 1;
            }
            SourceState::InSync | SourceState::Embedded => {}
        }
    }
    state.session.set_status(match (stale, missing) {
        (0, 0) => "All sources are up to date".to_string(),
        (s, 0) => format!("{s} source(s) changed on disk"),
        (0, m) => format!("{m} source file(s) missing"),
        (s, m) => format!("{s} changed, {m} missing"),
    });
}

/// How many attachments reference this asset by name.
fn attachment_uses(state: &AppState, id: AssetId) -> usize {
    use ankhimate_core::attachment::Attachment;
    let Some(name) = state.doc.assets.get(id).map(|a| a.name.clone()) else {
        return 0;
    };
    state
        .doc
        .skeleton
        .skins
        .iter()
        .flat_map(|(_, skin)| skin.entries.values())
        .filter(|att| match att {
            Attachment::Region(r) => r.texture == name,
            Attachment::Mesh(m) => m.texture == name,
            // The rest are geometry or markers, not artwork — they reference no
            // asset.
            _ => false,
        })
        .count()
}

/// A square icon button: an accent glyph, and nothing else until you touch it.
///
/// No border. The glyphs are already the only accent-coloured thing in the
/// header, so an outline around each one adds a second mark saying what the
/// colour has said — and four boxed icons in a row read as a toolbar bolted on
/// rather than as part of the header.
fn action_button(ui: &mut egui::Ui, icon: &str, enabled: bool) -> egui::Response {
    const SIZE: f32 = 26.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(SIZE, SIZE),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let accent = ui.visuals().selection.bg_fill;
    let color = if enabled {
        accent
    } else {
        accent.gamma_multiply(0.35)
    };
    // Hover fills faintly rather than brightening the outline: a brighter border
    // on an already-accent button is a change nobody notices.
    if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, 6, accent.gamma_multiply(0.15));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        color,
    );
    response
}

/// Decode (once) and cache an egui thumbnail for an asset.
fn thumbnail(
    ctx: &egui::Context,
    state: &mut AppState,
    id: AssetId,
) -> Option<egui::TextureHandle> {
    scaled_thumbnail(ctx, state, id, THUMB as u32 * 2)
}

/// Decode (once) and cache an asset scaled to fit `max_px` on its long edge.
///
/// The panel and the hierarchy's hover preview want different sizes, so the size
/// is part of the cache key: asking for a bigger one must not hand back the
/// small one already in the map and draw it upscaled and soft.
pub fn scaled_thumbnail(
    ctx: &egui::Context,
    state: &mut AppState,
    id: AssetId,
    max_px: u32,
) -> Option<egui::TextureHandle> {
    let asset = state.doc.assets.get(id)?;
    if asset.bytes.is_empty() {
        return None;
    }
    // Keyed by name+size rather than the slotmap id, for the same reason the GPU
    // cache is content-keyed: ids are recycled between documents.
    let cache_key = format!(
        "thumb:{}:{}x{}@{max_px}",
        asset.name, asset.width, asset.height
    );
    if let Some(handle) = state.session.thumbnails.get(&cache_key) {
        return Some(handle.clone());
    }

    let image = image::load_from_memory(&asset.bytes).ok()?;
    let rgba = image.thumbnail(max_px, max_px).to_rgba8();
    let (w, h) = rgba.dimensions();
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
    let handle = ctx.load_texture(&cache_key, color_image, egui::TextureOptions::LINEAR);
    state.session.thumbnails.insert(cache_key, handle.clone());
    Some(handle)
}

/// File dialog import — the keyboard-and-menu path to the same place a drop goes.
fn import_dialog(state: &mut AppState) {
    let Some(paths) = rfd::FileDialog::new()
        .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
        .pick_files()
    else {
        return;
    };
    let bone = state.session.active_bone().or_else(|| {
        state.doc.skeleton.update_order.iter().copied().find(|&id| {
            state
                .doc
                .skeleton
                .bones
                .get(id)
                .is_some_and(|b| b.parent.is_none())
        })
    });
    let Some(bone) = bone else {
        state
            .session
            .set_status("Create a bone first, then import an image onto it");
        return;
    };
    for path in paths {
        crate::ui::canvas::import_image_file(state, &path, bone, glam::Vec2::ZERO);
    }
}
