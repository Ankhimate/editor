//! The program mark, rendered from vector art at whatever size is asked for.
//!
//! Kept as SVG and rasterised on demand rather than vendored as a PNG. A raster
//! has one native size, and the two places the mark appears want very different
//! ones — 18 points in the title bar against 256 pixels for the window icon.
//! Downscaling one source to the other is what made the first attempt look
//! chewed: GPU filtering samples 2x2 texels, so a 14x reduction drops most of
//! the glyph on the floor and aliases the stroke. Rendering at the target size
//! means every pixel is computed for the size it is shown at.
//!
//! Replacing it: any SVG with a `viewBox` works. Nothing here assumes the
//! aspect ratio or that the art is square.

use eframe::egui;
// Both re-exported by resvg, so the versions cannot drift apart.
use resvg::{tiny_skia, usvg};

/// The ankh, in the brand yellow.
pub const LOGO_SVG: &[u8] = include_bytes!("../../assets/logo.svg");

/// Height of the mark in the title bar, in points.
///
/// Smaller than the 26pt window buttons beside it: the mark is identity, not a
/// control, and matching their height would read as a fourth button.
pub const TITLE_BAR_HEIGHT: f32 = 18.0;

/// Edge of the window icon, in pixels. 256 is what Windows and the major Linux
/// desktops want as the largest entry.
const ICON_SIDE: u32 = 256;

/// A rasterised mark: straight RGBA, plus the size it came out at.
pub struct Raster {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Rasterise the mark to fit a box `height` pixels tall, keeping its aspect.
///
/// `None` if the vendored SVG will not parse, which only happens if it was
/// replaced with something that is not SVG. The mark is decoration; a bad asset
/// should cost the title bar its logo, not stop the editor from opening.
pub fn render(height: u32) -> Option<Raster> {
    let tree = usvg::Tree::from_data(LOGO_SVG, &usvg::Options::default()).ok()?;
    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return None;
    }

    let scale = height as f32 / size.height();
    let width = (size.width() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(width as f32 / size.width(), scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia works premultiplied; egui and the platform icon APIs both want
    // straight alpha. Un-premultiplying here rather than at the call sites keeps
    // the one place that knows about the difference in this file.
    Some(unpremultiply(Raster {
        rgba: pixmap.take(),
        width,
        height,
    }))
}

fn unpremultiply(mut raster: Raster) -> Raster {
    for px in raster.rgba.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255) / a as u32).min(255) as u8;
            }
        }
    }
    raster
}

/// The window icon, for the taskbar and Alt-Tab.
///
/// Padded to a square around the glyph. The platform scales whatever it is given
/// into a square slot, so handing it the tall 2:3 mark unchanged would squash
/// the ankh; padding lets it scale uniformly and keeps the proportions.
pub fn window_icon() -> Option<egui::IconData> {
    let art = render(ICON_SIDE)?;
    let side = ICON_SIDE.max(art.width);
    let mut square = vec![0u8; (side * side * 4) as usize];
    let (dx, dy) = ((side - art.width) / 2, (side - art.height) / 2);
    for y in 0..art.height {
        let src = (y * art.width * 4) as usize;
        let dst = (((y + dy) * side + dx) * 4) as usize;
        let len = (art.width * 4) as usize;
        square[dst..dst + len].copy_from_slice(&art.rgba[src..src + len]);
    }
    Some(egui::IconData {
        rgba: square,
        width: side,
        height: side,
    })
}

/// The mark as a texture, re-rendered whenever the size it is drawn at changes.
///
/// Holds the pixel height it was rendered for so a UI-scale change re-renders
/// instead of stretching what is already uploaded — the whole point of keeping
/// the art as vector.
#[derive(Default)]
pub struct Logo {
    texture: Option<egui::TextureHandle>,
    rendered_at: u32,
}

impl Logo {
    /// The texture at `height` *points*, rendered for the current pixel density.
    ///
    /// Returns `None` only if the asset will not parse.
    pub fn texture(&mut self, ctx: &egui::Context, height: f32) -> Option<&egui::TextureHandle> {
        let pixels = (height * ctx.pixels_per_point()).round().max(1.0) as u32;
        if self.texture.is_none() || self.rendered_at != pixels {
            let art = render(pixels)?;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [art.width as usize, art.height as usize],
                &art.rgba,
            );
            // Nearest, not linear: the raster is already exactly the size it
            // will be drawn at, so filtering would only soften edges that are
            // correct as they are.
            self.texture =
                Some(ctx.load_texture("ankhimate-logo", image, egui::TextureOptions::NEAREST));
            self.rendered_at = pixels;
        }
        self.texture.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_at_the_size_asked_for() {
        for height in [16, 18, 45, 256] {
            let art = render(height).expect("vendored logo must be parseable SVG");
            assert_eq!(art.height, height);
            assert_eq!(art.rgba.len(), (art.width * art.height * 4) as usize);
        }
    }

    #[test]
    fn the_loop_stays_hollow() {
        // The ankh's ring is a hole in the same path as its outline. A renderer
        // that took the winding rule as nonzero fills it in and the mark becomes
        // a lollipop — visible, but only if someone looks for it.
        //
        // Sampled near the top of the ring rather than its middle: the middle is
        // where the small triangle sits, which is *supposed* to be solid.
        let art = render(256).unwrap();
        let (x, y) = (art.width / 2, art.height / 10);
        let alpha = art.rgba[((y * art.width + x) * 4 + 3) as usize];
        assert_eq!(alpha, 0, "the ankh's loop filled in — check fill-rule");
    }

    #[test]
    fn ink_is_the_brand_yellow() {
        let art = render(256).unwrap();
        let opaque = art
            .rgba
            .chunks_exact(4)
            .find(|px| px[3] == 255)
            .expect("the mark must have solid pixels");
        assert_eq!(&opaque[..3], &[0xfa, 0xf7, 0x9f]);
    }

    #[test]
    fn window_icon_is_square_and_centred() {
        let icon = window_icon().unwrap();
        assert_eq!((icon.width, icon.height), (ICON_SIDE, ICON_SIDE));
        assert_eq!(icon.rgba.len(), (ICON_SIDE * ICON_SIDE * 4) as usize);
        // The padding is the point: the columns beside the glyph must be
        // untouched, or the copy landed at the wrong offset and it is cropped.
        assert!(icon.rgba[..4].iter().all(|&b| b == 0));
    }
}
