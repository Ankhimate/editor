use glam::Vec2;
use serde::{Deserialize, Serialize};

/// A local (parent-relative) transform. This is document data; world space is
/// always derived via [`crate::transforms::Affine2`] — never stored.
///
/// `rotation` and `shear` are **radians** (see `transforms` module docs; degrees
/// exist only at the `.ankh` serialization boundary and in editor widgets).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec2,
    pub rotation: f32,
    pub scale: Vec2,
    /// `shear.x` skews the Y axis toward X, `shear.y` skews X toward Y.
    pub shear: Vec2,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::ONE,
            shear: Vec2::ZERO,
        }
    }
}

impl Transform {
    /// This transform as a 2×3 affine. Kept as a convenience alias so callers
    /// don't have to import `transforms` for the common case.
    pub fn to_affine(&self) -> crate::transforms::Affine2 {
        crate::transforms::Affine2::compose(self)
    }
}
