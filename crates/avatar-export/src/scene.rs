// The baked avatar scene: the avatar.json schema (format "rexglue-avatar-export", version 1) as typed data.
// Right-handed, +Y up, avatar faces +Z, metres, UV origin top-left; every array is flat and row-major.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "rexglue-avatar-export";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Scene {
    pub format: String,
    pub version: u32,
    pub source: Source,
    pub axes: Axes,
    pub avatar: AvatarInfo,
    pub skeleton: Skeleton,
    pub components: Vec<Component>,
    pub materials: Vec<Material>,
    pub meshes: Vec<Mesh>,
    #[serde(default)]
    pub prop_skeleton: Option<Skeleton>,
    #[serde(default)]
    pub face: Option<Face>,
    #[serde(default)]
    pub animations: Vec<Animation>,
    #[serde(default)]
    pub log: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Source {
    pub manifest: String,
    pub pack: String,
    #[serde(default)]
    pub closet: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Axes {
    pub units: String,
    pub up: String,
    pub forward: String,
    pub left: String,
    pub handedness: String,
    pub uv_origin: String,
}

impl Default for Axes {
    fn default() -> Self {
        Self {
            units: "meters".into(),
            up: "+Y".into(),
            forward: "+Z".into(),
            left: "+X".into(),
            handedness: "right".into(),
            uv_origin: "top-left".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AvatarInfo {
    pub body_type: String,
    pub height_factor: f32,
    pub weight_factor: f32,
    pub scale_applied: bool,
    /// ARGB hex, one per `color_names` entry (skin, hair, mouth, iris, ...).
    pub colors: Vec<String>,
    pub color_names: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Skeleton {
    pub version: u32,
    pub joints: Vec<Joint>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Joint {
    pub name: String,
    /// -1 for the root.
    pub parent: i32,
    pub bind_world: [f32; 3],
    pub rest_world: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rest_world_rot: [f32; 4],
    pub rest_local: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Component {
    pub index: u32,
    pub slot: String,
    pub guid: String,
    pub name: String,
    pub categories: u32,
    pub category_names: String,
    /// "pack" or "closet".
    pub source: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Material {
    pub name: String,
    /// PNG file name next to the scene, empty when the material has no diffuse map.
    pub diffuse: String,
    pub shader: u32,
    pub shader_name: String,
    pub has_alpha: bool,
    pub alpha_mask: bool,
    pub double_sided: bool,
    pub uv_layer: u32,
    pub component_guid: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub component: u32,
    pub material: u32,
    pub is_prop: bool,
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub uv_count: u32,
    /// vertex_count * 3
    pub positions: Vec<f32>,
    /// vertex_count * 3
    pub normals: Vec<f32>,
    /// The material's UV layer, vertex_count * 2.
    pub uv: Vec<f32>,
    /// Every UV layer, each vertex_count * 2.
    #[serde(default)]
    pub uv_layers: Vec<Vec<f32>>,
    /// RGBA8, vertex_count * 4.
    pub colors: Vec<u8>,
    /// Four joint indices per vertex.
    pub joints: Vec<u16>,
    /// Four weights per vertex, summing to one.
    pub weights: Vec<f32>,
    /// triangle_count * 3
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Face {
    pub head_materials: Vec<String>,
    pub slots: BTreeMap<String, FaceSlot>,
    pub layer_files: Vec<FaceFile>,
    pub composite_files: Vec<FaceFile>,
    pub layer_names: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FaceSlot {
    pub guid: String,
    pub name: String,
    pub layers: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FaceFile {
    pub channel: String,
    pub frame: u32,
    pub file: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Animation {
    pub name: String,
    pub fps: f32,
    pub frame_count: u32,
    pub joint_count: u32,
    pub carryable_joint_count: u32,
    pub motion_count: u32,
    pub texture_count: u32,
    pub duration: f32,
    /// "absolute_local": each frame holds the joint's full local transform.
    pub translation_mode: String,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub carryable_tracks: Vec<Track>,
    #[serde(default)]
    pub motion: Vec<MotionTrack>,
    #[serde(default)]
    pub face: Option<FaceTrack>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub joint: u32,
    pub t: Vec<[f32; 3]>,
    pub r: Vec<[f32; 4]>,
    pub s: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MotionTrack {
    pub t: Vec<[f32; 3]>,
    pub r: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FaceTrack {
    pub mouth: Vec<u32>,
    pub brow_left: Vec<u32>,
    pub brow_right: Vec<u32>,
    pub eye_left: Vec<u32>,
    pub eye_right: Vec<u32>,
}

impl Scene {
    pub fn from_json(text: &str) -> serde_json::Result<Self> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}
