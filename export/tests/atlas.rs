//! Atlas bake behaviour (T-603a).

use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_export::atlas::{AtlasError, AtlasSettings, bake, encode_png, page_filename};
use image::{ImageFormat, Rgba, RgbaImage};

/// A `size`x`size` image whose opaque content is a `w`x`h` block at (`x`,`y`).
fn png(size: u32, x: u32, y: u32, w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let mut img = RgbaImage::new(size, size);
    for py in y..(y + h) {
        for px in x..(x + w) {
            img.put_pixel(px, py, Rgba(color));
        }
    }
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

fn solid(size: u32, color: [u8; 4]) -> Vec<u8> {
    png(size, 0, 0, size, size, color)
}

fn db(entries: &[(&str, Vec<u8>, u32)]) -> AssetDb {
    let mut db = AssetDb::new();
    for (name, bytes, size) in entries {
        db.add(ImageAsset::new(*name, bytes.clone(), *size, *size));
    }
    db
}

#[test]
fn a_trimmed_region_records_what_it_cut() {
    // 32x32 image, opaque only in a 10x6 block at (5, 7).
    let assets = db(&[("arm", png(32, 5, 7, 10, 6, [255, 0, 0, 255]), 32)]);
    let atlas = bake(&assets, &AtlasSettings::default()).unwrap();

    let r = &atlas.regions[0];
    assert_eq!(
        (r.width, r.height),
        (10, 6),
        "stored size is the trimmed size"
    );
    assert_eq!(
        (r.offset_x, r.offset_y),
        (5, 7),
        "offset is what was cut from the left/top"
    );
    assert_eq!(
        (r.original_width, r.original_height),
        (32, 32),
        "original size survives trimming"
    );

    // The round trip that matters: offset + trimmed size must reconstruct the
    // source placement, or every trimmed attachment renders shifted.
    assert_eq!(r.offset_x + r.width, 15);
    assert_eq!(r.offset_y + r.height, 13);
}

#[test]
fn trimming_off_keeps_the_original_bounds() {
    let assets = db(&[("arm", png(32, 5, 7, 10, 6, [255, 0, 0, 255]), 32)]);
    let settings = AtlasSettings {
        trim: false,
        ..Default::default()
    };
    let atlas = bake(&assets, &settings).unwrap();
    let r = &atlas.regions[0];
    assert_eq!((r.width, r.height), (32, 32));
    assert_eq!((r.offset_x, r.offset_y), (0, 0));
}

/// Packing is the step most likely to become order-dependent, and an atlas that
/// differs run to run makes every export a spurious diff in version control.
#[test]
fn the_same_assets_pack_to_identical_bytes() {
    let entries: Vec<(&str, Vec<u8>, u32)> = vec![
        ("head", solid(16, [255, 0, 0, 255]), 16),
        ("torso", solid(24, [0, 255, 0, 255]), 24),
        ("arm", solid(8, [0, 0, 255, 255]), 8),
        ("leg", solid(12, [255, 255, 0, 255]), 12),
    ];
    let first = bake(&db(&entries), &AtlasSettings::default()).unwrap();

    // Insert in a different order: a packer keyed on insertion or on a hash map
    // would place these differently.
    let mut shuffled = entries.clone();
    shuffled.reverse();
    let second = bake(&db(&shuffled), &AtlasSettings::default()).unwrap();

    assert_eq!(first.regions, second.regions, "region tables must match");
    assert_eq!(first.pages.len(), second.pages.len());
    for (a, b) in first.pages.iter().zip(second.pages.iter()) {
        assert_eq!(
            encode_png(a).unwrap(),
            encode_png(b).unwrap(),
            "page {} differs between bakes",
            a.index
        );
    }
}

#[test]
fn regions_never_overlap() {
    let entries: Vec<(&str, Vec<u8>, u32)> = (0..12)
        .map(|i| {
            let size = 8 + (i % 5) * 6;
            (
                ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"][i as usize],
                solid(size, [255, 255, 255, 255]),
                size,
            )
        })
        .collect();
    let atlas = bake(&db(&entries), &AtlasSettings::default()).unwrap();

    for (i, a) in atlas.regions.iter().enumerate() {
        for b in atlas.regions.iter().skip(i + 1) {
            if a.page != b.page {
                continue;
            }
            let disjoint = a.x + a.width <= b.x
                || b.x + b.width <= a.x
                || a.y + a.height <= b.y
                || b.y + b.height <= a.y;
            assert!(disjoint, "'{}' overlaps '{}'", a.name, b.name);
        }
    }
}

#[test]
fn every_region_lands_inside_its_page() {
    let entries: Vec<(&str, Vec<u8>, u32)> = vec![
        ("head", solid(16, [255, 0, 0, 255]), 16),
        ("torso", solid(24, [0, 255, 0, 255]), 24),
    ];
    let atlas = bake(&db(&entries), &AtlasSettings::default()).unwrap();
    for r in &atlas.regions {
        let page = &atlas.pages[r.page];
        assert!(
            r.x + r.width <= page.width && r.y + r.height <= page.height,
            "'{}' at ({},{}) {}x{} escapes page {}x{}",
            r.name,
            r.x,
            r.y,
            r.width,
            r.height,
            page.width,
            page.height
        );
    }
}

#[test]
fn overflowing_one_page_opens_another_and_keeps_every_region() {
    let entries: Vec<(&str, Vec<u8>, u32)> = vec![
        ("a", solid(48, [255, 0, 0, 255]), 48),
        ("b", solid(48, [0, 255, 0, 255]), 48),
        ("c", solid(48, [0, 0, 255, 255]), 48),
        ("d", solid(48, [255, 255, 0, 255]), 48),
    ];
    let settings = AtlasSettings {
        max_page: 64,
        ..Default::default()
    };
    let atlas = bake(&db(&entries), &settings).unwrap();

    assert!(
        atlas.pages.len() > 1,
        "48px sprites cannot share a 64px page"
    );
    assert_eq!(atlas.regions.len(), 4, "no region is lost to paging");
    for r in &atlas.regions {
        assert!(r.page < atlas.pages.len(), "'{}' names a real page", r.name);
    }
}

/// A fully transparent image trims to nothing. A zero-area region divides by
/// zero in downstream UV math, so it degrades to 1x1 instead.
#[test]
fn a_fully_transparent_image_still_produces_a_usable_region() {
    let assets = db(&[("ghost", solid(16, [0, 0, 0, 0]), 16)]);
    let atlas = bake(&assets, &AtlasSettings::default()).unwrap();
    let r = &atlas.regions[0];
    assert!(r.width > 0 && r.height > 0, "region must have area");
    assert_eq!((r.original_width, r.original_height), (16, 16));
}

#[test]
fn an_image_too_large_for_a_page_is_named_in_the_error() {
    let assets = db(&[("backdrop", solid(128, [255, 0, 0, 255]), 128)]);
    let settings = AtlasSettings {
        max_page: 64,
        ..Default::default()
    };
    match bake(&assets, &settings) {
        Err(AtlasError::TooLarge { asset, .. }) => assert_eq!(asset, "backdrop"),
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn undecodable_bytes_name_the_asset_rather_than_panicking() {
    let mut assets = AssetDb::new();
    assets.add(ImageAsset::new("broken", b"not a png".to_vec(), 8, 8));
    match bake(&assets, &AtlasSettings::default()) {
        Err(AtlasError::Decode { asset, .. }) => assert_eq!(asset, "broken"),
        other => panic!("expected Decode, got {other:?}"),
    }
}

/// Extrude exists to stop a sampler reaching past a region and finding
/// background — the atlas-bleed halo. The pixel outside the edge must therefore
/// be a copy of the edge, not transparent.
#[test]
fn extrude_duplicates_the_edge_pixel_outward() {
    let assets = db(&[
        ("a", solid(16, [255, 0, 0, 255]), 16),
        ("b", solid(16, [0, 255, 0, 255]), 16),
    ]);
    let settings = AtlasSettings {
        extrude: 2,
        padding: 6,
        trim: false,
        ..Default::default()
    };
    let atlas = bake(&assets, &settings).unwrap();

    let r = atlas.regions.iter().find(|r| r.name == "a").unwrap();
    let page = &atlas.pages[r.page];
    let inside = *page.pixels.get_pixel(r.x, r.y);
    assert_eq!(inside.0, [255, 0, 0, 255]);

    for step in 1..=2 {
        let above = *page.pixels.get_pixel(r.x, r.y - step);
        assert_eq!(
            above.0, inside.0,
            "pixel {step} above the region should copy the edge, not be empty"
        );
        let left = *page.pixels.get_pixel(r.x - step, r.y);
        assert_eq!(left.0, inside.0, "pixel {step} left of the region");
    }
}

#[test]
fn power_of_two_rounds_both_page_dimensions_up() {
    let assets = db(&[("a", solid(20, [255, 0, 0, 255]), 20)]);
    let settings = AtlasSettings {
        power_of_two: true,
        trim: false,
        ..Default::default()
    };
    let atlas = bake(&assets, &settings).unwrap();
    let page = &atlas.pages[0];
    assert!(page.width.is_power_of_two(), "width {} ", page.width);
    assert!(page.height.is_power_of_two(), "height {}", page.height);
}

#[test]
fn page_filenames_follow_the_documented_convention() {
    assert_eq!(page_filename("atlas", 0), "atlas.png");
    assert_eq!(page_filename("atlas", 1), "atlas_2.png");
    assert_eq!(page_filename("atlas", 4), "atlas_5.png");
}

#[test]
fn an_empty_library_bakes_without_error() {
    let atlas = bake(&AssetDb::new(), &AtlasSettings::default()).unwrap();
    assert!(atlas.regions.is_empty());
}
