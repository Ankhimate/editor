//! Atlas bake (T-603a): trim, pack, extrude, page out.
//!
//! Runs on the CPU over `AssetDb` — no wgpu, no window. That is what lets it run
//! in CI and, later, in a headless CLI exporter.
//!
//! `core` stores image bytes verbatim as the encoded file and owns no decoder
//! (PLAN §3.1). Decoding therefore happens here, once per asset per bake.

use ankhimate_core::assets::AssetDb;
use image::{GenericImage, GenericImageView, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a bake is configured. Mirrors the `atlas` block of an export preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasSettings {
    /// Cut fully transparent borders away and record the offset.
    pub trim: bool,
    /// Empty pixels between neighbouring regions.
    pub padding: u32,
    /// Edge pixels duplicated outward from each region.
    ///
    /// Distinct from `padding`, and both are needed: padding separates regions,
    /// extrude fills the gap with a copy of the edge pixel. A renderer sampling
    /// at non-integer zoom reaches half a texel past the region; without
    /// extrude it finds background and every sprite gets a faint halo.
    pub extrude: u32,
    /// Maximum page edge, in pixels. Regions that do not fit open a new page.
    pub max_page: u32,
    /// Round page dimensions up to a power of two.
    pub power_of_two: bool,
    /// Allow a region to be stored rotated 90° when that packs tighter.
    ///
    /// Off by default: some runtimes cannot un-rotate a region, and a template
    /// author has no way to compensate for one that arrives sideways.
    pub allow_rotation: bool,
}

impl Default for AtlasSettings {
    fn default() -> Self {
        Self {
            trim: true,
            padding: 2,
            extrude: 1,
            max_page: 2048,
            power_of_two: false,
            allow_rotation: false,
        }
    }
}

/// Where one image ended up.
///
/// `offset_*` and `original_*` are what let a consumer place a trimmed sprite
/// where the untrimmed one sat. A format that emits only the packed rect shifts
/// every trimmed attachment, which reads as a rigging error rather than an
/// export bug — so these are not optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Asset name — the key an attachment's `texture` joins on.
    pub name: String,
    pub page: usize,
    pub x: u32,
    pub y: u32,
    /// Size as stored. With `rotated`, this is the *unrotated* size: a consumer
    /// reading a rotated region reads `height` across and `width` down.
    pub width: u32,
    pub height: u32,
    /// Transparent pixels cut from the left/top edge during trimming.
    pub offset_x: u32,
    pub offset_y: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub rotated: bool,
}

/// One baked page, ready to encode.
#[derive(Debug, Clone)]
pub struct Page {
    pub index: usize,
    pub width: u32,
    pub height: u32,
    pub pixels: RgbaImage,
}

#[derive(Debug, Clone)]
pub struct Atlas {
    pub pages: Vec<Page>,
    pub regions: Vec<Region>,
}

impl Atlas {
    /// Regions by name, for template context assembly and lookups.
    pub fn by_name(&self) -> BTreeMap<&str, &Region> {
        self.regions.iter().map(|r| (r.name.as_str(), r)).collect()
    }
}

#[derive(Debug)]
pub enum AtlasError {
    /// An asset's bytes did not decode. Carries the asset name and the reason.
    Decode { asset: String, reason: String },
    /// An image is larger than `max_page` even alone, so no page can hold it.
    TooLarge {
        asset: String,
        width: u32,
        height: u32,
        max_page: u32,
    },
}

impl std::fmt::Display for AtlasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AtlasError::Decode { asset, reason } => {
                write!(f, "image '{asset}' could not be decoded: {reason}")
            }
            AtlasError::TooLarge {
                asset,
                width,
                height,
                max_page,
            } => write!(
                f,
                "image '{asset}' is {width}x{height}, larger than the {max_page}px page limit; \
                 raise the page size or scale the source image"
            ),
        }
    }
}

impl std::error::Error for AtlasError {}

/// A decoded image plus the trim rect chosen for it.
struct Sprite {
    name: String,
    pixels: RgbaImage,
    /// Trimmed sub-rect within `pixels`.
    trim_x: u32,
    trim_y: u32,
    trim_w: u32,
    trim_h: u32,
    original_width: u32,
    original_height: u32,
}

impl Sprite {
    /// Footprint on a page: trimmed size widened by extrude on all sides.
    fn placed_size(&self, settings: &AtlasSettings) -> (u32, u32) {
        let e = settings.extrude * 2;
        (self.trim_w + e, self.trim_h + e)
    }
}

/// Bake every image in `assets` into one or more pages.
///
/// Deterministic: assets are processed in name order, so the same library packs
/// to identical bytes every run. A packer that walked a hash map would reorder
/// between runs and make every export a spurious diff in version control.
pub fn bake(assets: &AssetDb, settings: &AtlasSettings) -> Result<Atlas, AtlasError> {
    let mut sprites = Vec::new();
    let mut names: Vec<&str> = assets.images.iter().map(|(_, a)| a.name.as_str()).collect();
    names.sort_unstable();

    for name in names {
        let Some(id) = assets.by_name(name) else {
            continue;
        };
        let asset = &assets.images[id];
        let decoded = image::load_from_memory(&asset.bytes).map_err(|e| AtlasError::Decode {
            asset: asset.name.clone(),
            reason: e.to_string(),
        })?;
        let mut pixels = decoded.to_rgba8();
        // A zero-dimension image cannot be placed at all: it would produce a
        // zero-area region, which divides by zero in a consumer's UV maths.
        // Promote it to a single transparent pixel so the export completes and
        // the artefact is visibly empty rather than subtly broken.
        if pixels.width() == 0 || pixels.height() == 0 {
            pixels = RgbaImage::new(1, 1);
        }
        let (w, h) = (pixels.width(), pixels.height());
        let (tx, ty, tw, th) = if settings.trim {
            trim_bounds(&pixels)
        } else {
            (0, 0, w, h)
        };
        sprites.push(Sprite {
            name: asset.name.clone(),
            pixels,
            trim_x: tx,
            trim_y: ty,
            trim_w: tw,
            trim_h: th,
            original_width: w,
            original_height: h,
        });
    }

    for s in &sprites {
        let (pw, ph) = s.placed_size(settings);
        if pw > settings.max_page || ph > settings.max_page {
            return Err(AtlasError::TooLarge {
                asset: s.name.clone(),
                width: s.original_width,
                height: s.original_height,
                max_page: settings.max_page,
            });
        }
    }

    // Tallest first. A shelf packer fed in arbitrary order leaves a tall sprite
    // opening a deep shelf that short ones then waste; height-descending keeps
    // each shelf close to uniform.
    let mut order: Vec<usize> = (0..sprites.len()).collect();
    order.sort_by(|&a, &b| {
        let (_, ah) = sprites[a].placed_size(settings);
        let (_, bh) = sprites[b].placed_size(settings);
        bh.cmp(&ah)
            .then_with(|| sprites[a].name.cmp(&sprites[b].name))
    });

    let mut shelves = ShelfPacker::new(settings);
    let mut placements = Vec::new();
    for &i in &order {
        let (pw, ph) = sprites[i].placed_size(settings);
        let spot = shelves.place(pw, ph);
        placements.push((i, spot));
    }

    let mut pages = build_pages(&sprites, &placements, &shelves, settings);
    let mut regions: Vec<Region> = placements
        .iter()
        .map(|(i, spot)| {
            let s = &sprites[*i];
            Region {
                name: s.name.clone(),
                page: spot.page,
                x: spot.x + settings.extrude,
                y: spot.y + settings.extrude,
                width: s.trim_w,
                height: s.trim_h,
                offset_x: s.trim_x,
                offset_y: s.trim_y,
                original_width: s.original_width,
                original_height: s.original_height,
                rotated: false,
            }
        })
        .collect();

    // Emit in name order regardless of packing order, so the region table is
    // stable and diffable even when the packer's choices change.
    regions.sort_by(|a, b| a.name.cmp(&b.name));
    pages.sort_by_key(|p| p.index);

    Ok(Atlas { pages, regions })
}

/// The smallest rect containing every non-transparent pixel.
///
/// A fully transparent image has no such rect. Rather than a zero-area region —
/// which divides by zero in UV math downstream — it degrades to 1x1 at the
/// origin, so the export completes and the artefact is visibly empty.
fn trim_bounds(img: &RgbaImage) -> (u32, u32, u32, u32) {
    let (w, h) = (img.width(), img.height());
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for y in 0..h {
        for x in 0..w {
            if img.get_pixel(x, y).0[3] != 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        return (0, 0, 1, 1);
    }
    (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

#[derive(Debug, Clone, Copy)]
struct Spot {
    page: usize,
    x: u32,
    y: u32,
}

/// Shelf (skyline-lite) packer.
///
/// Chosen over MaxRects deliberately: it is a few dozen lines, its output is
/// trivially reproducible, and atlas density is not this project's bottleneck.
/// MaxRects packs perhaps 10% tighter for considerably more code to keep
/// deterministic.
struct ShelfPacker {
    max_page: u32,
    padding: u32,
    pages: Vec<PageLayout>,
}

#[derive(Default)]
struct PageLayout {
    /// Bottom edge of the open shelf.
    shelf_y: u32,
    /// Left edge of free space on the open shelf.
    cursor_x: u32,
    /// Height of the open shelf.
    shelf_h: u32,
    /// Extent actually used, for page sizing.
    used_w: u32,
    used_h: u32,
}

impl ShelfPacker {
    fn new(settings: &AtlasSettings) -> Self {
        Self {
            max_page: settings.max_page,
            padding: settings.padding,
            pages: vec![PageLayout::default()],
        }
    }

    fn place(&mut self, w: u32, h: u32) -> Spot {
        let pad = self.padding;
        for (index, page) in self.pages.iter_mut().enumerate() {
            // Fits on the open shelf?
            let x = if page.cursor_x == 0 {
                0
            } else {
                page.cursor_x + pad
            };
            if x + w <= self.max_page && page.shelf_y + h.max(page.shelf_h) <= self.max_page {
                let spot = Spot {
                    page: index,
                    x,
                    y: page.shelf_y,
                };
                page.cursor_x = x + w;
                page.shelf_h = page.shelf_h.max(h);
                page.used_w = page.used_w.max(page.cursor_x);
                page.used_h = page.used_h.max(page.shelf_y + page.shelf_h);
                return spot;
            }
            // Open a new shelf above the current one.
            let next_y = page.shelf_y + page.shelf_h + pad;
            if w <= self.max_page && next_y + h <= self.max_page {
                let spot = Spot {
                    page: index,
                    x: 0,
                    y: next_y,
                };
                page.shelf_y = next_y;
                page.shelf_h = h;
                page.cursor_x = w;
                page.used_w = page.used_w.max(w);
                page.used_h = page.used_h.max(next_y + h);
                return spot;
            }
        }
        // Every existing page is full.
        let index = self.pages.len();
        self.pages.push(PageLayout {
            shelf_y: 0,
            cursor_x: w,
            shelf_h: h,
            used_w: w,
            used_h: h,
        });
        Spot {
            page: index,
            x: 0,
            y: 0,
        }
    }
}

fn build_pages(
    sprites: &[Sprite],
    placements: &[(usize, Spot)],
    packer: &ShelfPacker,
    settings: &AtlasSettings,
) -> Vec<Page> {
    let mut pages: Vec<Page> = packer
        .pages
        .iter()
        .enumerate()
        .map(|(index, layout)| {
            let (mut w, mut h) = (layout.used_w.max(1), layout.used_h.max(1));
            if settings.power_of_two {
                w = w.next_power_of_two();
                h = h.next_power_of_two();
            }
            Page {
                index,
                width: w,
                height: h,
                pixels: RgbaImage::new(w, h),
            }
        })
        .collect();

    for (i, spot) in placements {
        let s = &sprites[*i];
        let page = &mut pages[spot.page];
        let dx = spot.x + settings.extrude;
        let dy = spot.y + settings.extrude;
        let view = s.pixels.view(s.trim_x, s.trim_y, s.trim_w, s.trim_h);
        page.pixels
            .copy_from(&*view, dx, dy)
            .expect("region placed within its page");
        if settings.extrude > 0 {
            extrude_edges(
                &mut page.pixels,
                dx,
                dy,
                s.trim_w,
                s.trim_h,
                settings.extrude,
            );
        }
    }

    pages
}

/// Duplicate the region's border pixels outward by `amount`.
///
/// Corners take the nearest corner pixel, which is what a clamped sampler would
/// have returned had the texture ended there.
fn extrude_edges(page: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, amount: u32) {
    let (pw, ph) = (page.width(), page.height());
    for step in 1..=amount {
        for col in 0..w {
            let px = x + col;
            if px >= pw {
                continue;
            }
            let top = *page.get_pixel(px, y);
            let bottom = *page.get_pixel(px, y + h - 1);
            if y >= step {
                page.put_pixel(px, y - step, top);
            }
            if y + h - 1 + step < ph {
                page.put_pixel(px, y + h - 1 + step, bottom);
            }
        }
        for row in 0..h {
            let py = y + row;
            if py >= ph {
                continue;
            }
            let left = *page.get_pixel(x, py);
            let right = *page.get_pixel(x + w - 1, py);
            if x >= step {
                page.put_pixel(x - step, py, left);
            }
            if x + w - 1 + step < pw {
                page.put_pixel(x + w - 1 + step, py, right);
            }
        }
    }
    // Corners.
    for sy in 1..=amount {
        for sx in 1..=amount {
            let corners = [
                (x.checked_sub(sx), y.checked_sub(sy), (x, y)),
                (Some(x + w - 1 + sx), y.checked_sub(sy), (x + w - 1, y)),
                (x.checked_sub(sx), Some(y + h - 1 + sy), (x, y + h - 1)),
                (
                    Some(x + w - 1 + sx),
                    Some(y + h - 1 + sy),
                    (x + w - 1, y + h - 1),
                ),
            ];
            for (tx, ty, (srcx, srcy)) in corners {
                if let (Some(tx), Some(ty)) = (tx, ty)
                    && tx < pw
                    && ty < ph
                {
                    let p = *page.get_pixel(srcx, srcy);
                    page.put_pixel(tx, ty, p);
                }
            }
        }
    }
}

/// Encode a page to PNG bytes.
pub fn encode_png(page: &Page) -> Result<Vec<u8>, image::ImageError> {
    let mut out = Vec::new();
    page.pixels
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// The filename convention for a page: `atlas.png`, `atlas_2.png`, …
pub fn page_filename(stem: &str, index: usize) -> String {
    if index == 0 {
        format!("{stem}.png")
    } else {
        format!("{stem}_{}.png", index + 1)
    }
}
