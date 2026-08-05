//! The program mark, decoded once and shared by everything that draws it.
//!
//! One PNG serves both the title bar and the window icon. It is vendored as a
//! raster rather than as the SVG it was drawn from because egui cannot rasterise
//! SVG without `egui_extras`' `svg` feature, which pulls resvg and usvg in — a
//! large tree for one glyph that never changes.
//!
//! Replacing it: export at 256px wide or more, RGBA with a transparent
//! background, and keep the glyph's own aspect ratio — nothing here assumes a
//! particular size or squareness, both call sites read the dimensions back.

use eframe::egui;

/// The ankh, in the brand yellow on transparency.
pub const LOGO_PNG: &[u8] = include_bytes!("../../assets/logo.png");

/// Height of the mark in the title bar, in points.
///
/// Smaller than the 26px window buttons beside it: the mark is identity, not a
/// control, and matching their height would read as a fourth button.
pub const TITLE_BAR_HEIGHT: f32 = 18.0;

/// Decode the mark to raw RGBA. `None` if the vendored file will not decode,
/// which only happens if it was replaced with something that is not a PNG.
fn decode() -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(LOGO_PNG).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

/// The window icon, for the taskbar and Alt-Tab.
///
/// Padded to a square around the glyph. Windows scales whatever it is given to a
/// square slot, so handing it the tall 2:3 mark unchanged would squash the ankh;
/// padding lets the platform scale uniformly and keeps the proportions.
pub fn window_icon() -> Option<egui::IconData> {
    let (rgba, w, h) = decode()?;
    let side = w.max(h);
    let mut square = vec![0u8; (side * side * 4) as usize];
    let (dx, dy) = ((side - w) / 2, (side - h) / 2);
    for y in 0..h {
        let src = (y * w * 4) as usize;
        let dst = (((y + dy) * side + dx) * 4) as usize;
        let len = (w * 4) as usize;
        square[dst..dst + len].copy_from_slice(&rgba[src..src + len]);
    }
    Some(egui::IconData {
        rgba: square,
        width: side,
        height: side,
    })
}

/// Upload the mark as a texture, once per context.
///
/// Linear filtering because the mark is drawn at roughly a fifteenth of its
/// stored size and at whatever fractional scale the display asks for; nearest
/// sampling there is what made the earlier icons look chewed.
pub fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let (rgba, w, h) = decode()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    Some(ctx.load_texture("ankhimate-logo", image, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_decodes_with_transparency() {
        let (rgba, w, h) = decode().expect("vendored logo must be a decodable PNG");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // A mark with no transparent pixel is a rectangle, which means someone
        // exported it flattened onto a background and it will show as a block
        // sitting on the title bar.
        assert!(
            rgba.chunks_exact(4).any(|px| px[3] == 0),
            "logo has no transparent pixels — was it exported flattened?"
        );
    }

    #[test]
    fn window_icon_is_square_and_centred() {
        let (_, w, h) = decode().unwrap();
        let icon = window_icon().unwrap();
        let side = w.max(h);
        assert_eq!((icon.width, icon.height), (side, side));
        assert_eq!(icon.rgba.len(), (side * side * 4) as usize);

        // The padding is the point: a row above the glyph must be untouched, or
        // the copy landed at the wrong offset and the ankh is cropped.
        let dy = (side - h) / 2;
        if dy > 0 {
            assert!(icon.rgba[..(side * 4) as usize].iter().all(|&b| b == 0));
        }
    }
}
