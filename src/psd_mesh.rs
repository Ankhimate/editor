//! `[mesh]` and `[weights]` on a PSD layer, applied after the import.
//!
//! The tracer lives here rather than in `formats` because `core` is the runtime
//! contract and stays dependency-light (PLAN §3.1): moving `meshgen` down so the
//! PSD reader could call it would drag `spade` and `image` into the crate that
//! has to compile for `wasm32`, to save one crate a round trip. So the reader
//! marks the layers — `PsdImport::trace_requests` — and this turns the marks
//! into meshes.
//!
//! Runs **before** the import is dispatched, on the imported skeleton rather
//! than on the document. An artist importing a rig with twelve `[mesh]` layers
//! should get one undo step, not thirteen; and a trace that fails should leave
//! a working region rather than half a rig.

use ankhimate_core::assets::AssetDb;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::skeleton::Skeleton;
use ankhimate_formats::psd::TraceRequest;

/// What a `[mesh]` request produced, for the import summary.
#[derive(Debug, Clone, PartialEq)]
pub struct Traced {
    pub attachment: String,
    pub vertices: usize,
    pub triangles: usize,
    pub weighted: bool,
}

/// Why a `[mesh]` request produced nothing.
///
/// Reported rather than swallowed: an artist who tagged a layer and got a plain
/// region back is owed the reason, and "the tag did nothing" is the failure the
/// whole tag grammar is meant to avoid.
#[derive(Debug, Clone, PartialEq)]
pub struct NotTraced {
    pub attachment: String,
    pub because: String,
}

/// Trace every `[mesh]` layer in an import, replacing its region with a mesh.
pub fn apply(
    skeleton: &mut Skeleton,
    assets: &AssetDb,
    requests: &[TraceRequest],
) -> (Vec<Traced>, Vec<NotTraced>) {
    let mut traced = Vec::new();
    let mut failed = Vec::new();

    for request in requests {
        match trace_one(skeleton, assets, request) {
            Ok(result) => traced.push(result),
            Err(because) => failed.push(NotTraced {
                attachment: request.attachment.clone(),
                because,
            }),
        }
    }
    (traced, failed)
}

fn trace_one(
    skeleton: &mut Skeleton,
    assets: &AssetDb,
    request: &TraceRequest,
) -> Result<Traced, String> {
    let slot = skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == request.slot)
        .map(|(id, _)| id)
        .ok_or_else(|| format!("no slot named `{}`", request.slot))?;
    let skin = skeleton.default_skin;

    let Some(Attachment::Region(region)) = skeleton.skins[skin].get(slot, &request.attachment)
    else {
        return Err("not a region attachment".into());
    };
    let region = region.clone();

    let image = assets
        .by_name(&region.texture)
        .and_then(|id| assets.get(id))
        .and_then(|asset| image::load_from_memory(&asset.bytes).ok())
        .ok_or_else(|| format!("`{}` could not be decoded", region.texture))?
        .to_rgba8();

    let mut options = crate::meshgen::TraceOptions::default();
    if let Some(detail) = request.detail {
        options.detail = detail.clamp(0.0, 100.0);
    }
    let outline = crate::meshgen::trace(&image, options)
        .ok_or_else(|| "nothing opaque enough to trace".to_string())?;
    let outline = crate::meshgen::refine(&outline, options);

    // Into the region's own footprint, so the mesh lands where the art already
    // is rather than jumping to a canonical box.
    let corners = region.local_corners();
    let bounds = (
        glam::Vec2::new(
            corners.iter().map(|c| c.x).fold(f32::INFINITY, f32::min),
            corners.iter().map(|c| c.y).fold(f32::INFINITY, f32::min),
        ),
        glam::Vec2::new(
            corners
                .iter()
                .map(|c| c.x)
                .fold(f32::NEG_INFINITY, f32::max),
            corners
                .iter()
                .map(|c| c.y)
                .fold(f32::NEG_INFINITY, f32::max),
        ),
    );
    let (vertices, uvs, triangles) = crate::meshgen::mesh_from_trace(&outline, bounds);
    if triangles.is_empty() {
        return Err("the trace produced no triangles".into());
    }

    let mut mesh = ankhimate_core::attachment::MeshAttachment {
        texture: region.texture.clone(),
        setup_vertices: vertices,
        uvs,
        triangles,
        weights: Vec::new(),
        ffd_keyframes: Vec::new(),
        edges: Vec::new(),
        inverse_bind_matrices: Default::default(),
        linked: None,
        sequence: region.sequence.clone(),
    };
    let (vertex_count, triangle_count) = (mesh.setup_vertices.len(), mesh.triangles.len());

    // `[weights]` binds to the bones the art already sits among. Bound here
    // rather than left to the artist because the tag asked for it; a rig with
    // meshes and no weights deforms as rigid quads, which looks like the trace
    // failed.
    let mut weighted = false;
    if request.weights {
        let bones = bone_segments(skeleton);
        if bones.is_empty() {
            return Err("[weights] needs bones to bind to, and there are none".into());
        }
        mesh.weights = crate::commands::weight_cmds::auto_weight(&mesh, &bones, 1.0, &[]);
        weighted = mesh.weights.iter().any(|w| !w.is_empty());
    }

    skeleton.skins[skin].set(slot, request.attachment.clone(), Attachment::Mesh(mesh));
    Ok(Traced {
        attachment: request.attachment.clone(),
        vertices: vertex_count,
        triangles: triangle_count,
        weighted,
    })
}

/// Every bone as `(id, start, end)` in world space, which is what `auto_weight`
/// measures distance against.
fn bone_segments(
    skeleton: &Skeleton,
) -> Vec<(ankhimate_core::ids::BoneId, glam::Vec2, glam::Vec2)> {
    // The setup pose: `evaluate` with no animations copies the rig's own
    // transforms, which is where the art sits at import time.
    let mut pose = ankhimate_core::pose::Pose::new();
    ankhimate_core::pose::evaluate(skeleton, &[], &mut pose);
    skeleton
        .bones
        .iter()
        .filter_map(|(id, bone)| {
            let world = pose.worlds.get(id)?;
            let start = world.transform_point(glam::Vec2::ZERO);
            let end = world.transform_point(glam::Vec2::new(bone.length, 0.0));
            Some((id, start, end))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::assets::ImageAsset;
    use ankhimate_core::attachment::{Rect, RegionAttachment};
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_core::slot::Slot;

    /// A skeleton with one bone, one slot and one region over a solid image.
    fn rig() -> (Skeleton, AssetDb) {
        let mut skeleton = Skeleton::new();
        let bone = skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 40.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = skeleton.add_slot(Slot {
            attachment: Some("cape".into()),
            ..Slot::new("cape_slot".to_string(), bone)
        });
        let skin = skeleton.default_skin;
        skeleton.skins[skin].set(
            slot,
            "cape",
            Attachment::Region(RegionAttachment {
                texture: "cape".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 40.0,
                height: 40.0,
                sequence: None,
                uv_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                pivot: glam::Vec2::splat(0.5),
            }),
        );

        let mut image = image::RgbaImage::new(40, 40);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([200, 120, 60, 255]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode");

        let mut assets = AssetDb::new();
        assets.add(ImageAsset::new("cape".to_string(), png, 40, 40));
        (skeleton, assets)
    }

    fn request(weights: bool) -> TraceRequest {
        TraceRequest {
            attachment: "cape".into(),
            slot: "cape_slot".into(),
            detail: None,
            weights,
        }
    }

    #[test]
    fn a_mesh_request_replaces_the_region_with_a_mesh() {
        let (mut skeleton, assets) = rig();
        let (traced, failed) = apply(&mut skeleton, &assets, &[request(false)]);

        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(traced.len(), 1);
        assert!(
            traced[0].triangles > 0,
            "a mesh with no triangles is not one"
        );

        let skin = skeleton.default_skin;
        let slot = skeleton.slots.iter().next().map(|(id, _)| id).unwrap();
        assert!(
            matches!(
                skeleton.skins[skin].get(slot, "cape"),
                Some(Attachment::Mesh(_))
            ),
            "the region became a mesh"
        );
    }

    #[test]
    fn weights_are_bound_only_when_asked_for() {
        // `[mesh]` alone leaves the vertices rigid to the slot's bone, which is
        // what a mesh with no weights means. `[weights]` is a separate request
        // because tracing and binding are separate decisions.
        let (mut skeleton, assets) = rig();
        let (plain, _) = apply(&mut skeleton, &assets, &[request(false)]);
        assert!(!plain[0].weighted);

        let (mut skeleton, assets) = rig();
        let (weighted, failed) = apply(&mut skeleton, &assets, &[request(true)]);
        assert!(failed.is_empty(), "{failed:?}");
        assert!(
            weighted[0].weighted,
            "[weights] bound the traced vertices to the bones around them"
        );
    }

    #[test]
    fn a_request_naming_nothing_is_reported_not_swallowed() {
        // An artist who tagged a layer and got a plain region back is owed the
        // reason. Silence here is the exact failure the tag grammar exists to
        // avoid.
        let (mut skeleton, assets) = rig();
        let (traced, failed) = apply(
            &mut skeleton,
            &assets,
            &[TraceRequest {
                slot: "no_such_slot".into(),
                ..request(false)
            }],
        );
        assert!(traced.is_empty());
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0].because.contains("no slot"),
            "the reason names the problem: {}",
            failed[0].because
        );
    }

    #[test]
    fn a_failed_trace_leaves_a_working_region() {
        // Half a rig is worse than an untraced one: the import must still open.
        let (mut skeleton, assets) = rig();
        let (_, failed) = apply(
            &mut skeleton,
            &assets,
            &[TraceRequest {
                attachment: "not_here".into(),
                ..request(false)
            }],
        );
        assert_eq!(failed.len(), 1);

        let skin = skeleton.default_skin;
        let slot = skeleton.slots.iter().next().map(|(id, _)| id).unwrap();
        assert!(
            matches!(
                skeleton.skins[skin].get(slot, "cape"),
                Some(Attachment::Region(_))
            ),
            "the untouched attachment is still a drawable region"
        );
    }
}
