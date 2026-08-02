use crate::ids::BoneId;
use serde::{Deserialize, Serialize};

pub type AttachmentName = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Additive,
    Multiply,
    Screen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    pub bone: BoneId,
    pub attachment: Option<AttachmentName>,
    pub color: [f32; 4],
    pub dark_color: Option<[f32; 4]>,
    pub blend_mode: BlendMode,
}

impl Slot {
    pub fn new(name: String, bone: BoneId) -> Self {
        Self {
            name,
            bone,
            attachment: None,
            color: [1.0, 1.0, 1.0, 1.0],
            dark_color: None,
            blend_mode: BlendMode::default(),
        }
    }
}
