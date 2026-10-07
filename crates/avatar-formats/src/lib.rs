#![forbid(unsafe_code)]
//! Decoders for Xbox 360 avatar assets: asset packs, STRB containers, models, textures,
//! skeletons, blend shapes, animations and the marketplace closet.

pub mod animation;
pub mod asset_pack;
pub mod bits;
pub mod blend_shape;
pub mod closet;
pub mod compression;
pub mod error;
pub mod lzx;
pub mod model;
pub mod prop;
pub mod serializers;
pub mod skeleton;
pub mod strb;
pub mod texture;

pub use animation::Animation;
pub use asset_pack::{AssetId, AssetInfo, AssetPack};
pub use blend_shape::BlendShape;
pub use closet::{Closet, ClosetItem};
pub use error::{Error, Result};
pub use model::Model;
pub use prop::Prop;
pub use skeleton::Skeleton;
pub use texture::Texture;
