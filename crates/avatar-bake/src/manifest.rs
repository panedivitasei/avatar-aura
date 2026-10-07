//! Ports X_AVATAR_METADATA and friends (guest_asset.h) plus GetBodyType and ColorToFloat4 from avatar_export.cpp.
//! The manifest is the 1000-byte big-endian record the editor saves for an avatar.

use avatar_formats::skeleton::scaling::BodyType;
use avatar_formats::AssetId;

/// ComponentCategory bits.
pub mod category {
    pub const NONE: u32 = 0;
    pub const HEAD: u32 = 1 << 0;
    pub const BODY: u32 = 1 << 1;
    pub const HAIR: u32 = 1 << 2;
    pub const TOP: u32 = 1 << 3;
    pub const BOTTOM: u32 = 1 << 4;
    pub const SHOES: u32 = 1 << 5;
    pub const HAT: u32 = 1 << 6;
    pub const GLOVES: u32 = 1 << 7;
    pub const GLASSES: u32 = 1 << 8;
    pub const WRISTWEAR: u32 = 1 << 9;
    pub const EARRINGS: u32 = 1 << 10;
    pub const RING: u32 = 1 << 11;
    pub const PROP: u32 = 1 << 12;
    pub const ANIMATION: u32 = 1 << 22;
}

pub const METADATA_SIZE: usize = 1000;

/// RGBA in [0, 1], the C++ Float4 (x = r, y = g, z = b, w = a).
pub type Float4 = [f32; 4];

pub const WHITE: Float4 = [1.0, 1.0, 1.0, 1.0];

/// ColorToFloat4: ARGB word to normalized RGBA.
pub fn color_to_float4(argb: u32) -> Float4 {
    [
        ((argb >> 16) & 0xFF) as f32 / 255.0,
        ((argb >> 8) & 0xFF) as f32 / 255.0,
        (argb & 0xFF) as f32 / 255.0,
        ((argb >> 24) & 0xFF) as f32 / 255.0,
    ]
}

/// X_AVATAR_COMPONENT_INFO.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComponentInfo {
    pub asset_id: AssetId,
    pub categories: u16,
}

impl ComponentInfo {
    pub fn matches(&self, category_mask: u32) -> bool {
        !self.asset_id.is_zero() && u32::from(self.categories) & category_mask != 0
    }
}

/// X_AVATAR_METADATA_TEXTURE.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureInfo {
    pub asset_id: AssetId,
    pub scale: f32,
    pub rotation: f32,
    pub translation: [f32; 2],
}

/// X_AVATAR_METADATA.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub version: u32,
    pub weight_factor: f32,
    pub height_factor: f32,
    pub blend_shapes: [AssetId; 3],
    pub textures: [TextureInfo; 6],
    pub colors: [u32; 9],
    pub body_component: ComponentInfo,
    pub head_component: ComponentInfo,
    pub components: [ComponentInfo; 13],
    pub fallback_components: [ComponentInfo; 4],
    pub owner_xuid: u64,
    pub source_console_id: [u8; 5],
}

#[derive(Debug, thiserror::Error)]
#[error("manifest is {0} bytes, not a 1000-byte X_AVATAR_METADATA")]
pub struct ManifestSizeError(pub usize);

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn bef32(b: &[u8], o: usize) -> f32 {
    f32::from_bits(be32(b, o))
}

fn asset_id(b: &[u8], o: usize) -> AssetId {
    let mut id = [0u8; 16];
    id.copy_from_slice(&b[o..o + 16]);
    AssetId::from_bytes(&id)
}

fn component(b: &[u8], o: usize) -> ComponentInfo {
    ComponentInfo {
        asset_id: asset_id(b, o),
        categories: u16::from_be_bytes([b[o + 16], b[o + 17]]),
    }
}

impl Metadata {
    pub fn parse(b: &[u8]) -> Result<Self, ManifestSizeError> {
        if b.len() != METADATA_SIZE {
            return Err(ManifestSizeError(b.len()));
        }
        let mut m = Metadata {
            version: be32(b, 0x000),
            weight_factor: bef32(b, 0x004),
            height_factor: bef32(b, 0x008),
            ..Default::default()
        };
        for (i, s) in m.blend_shapes.iter_mut().enumerate() {
            *s = asset_id(b, 0x00C + i * 16);
        }
        for (i, t) in m.textures.iter_mut().enumerate() {
            let o = 0x03C + i * 32;
            *t = TextureInfo {
                asset_id: asset_id(b, o),
                scale: bef32(b, o + 16),
                rotation: bef32(b, o + 20),
                translation: [bef32(b, o + 24), bef32(b, o + 28)],
            };
        }
        for (i, c) in m.colors.iter_mut().enumerate() {
            *c = be32(b, 0x0FC + i * 4);
        }
        m.body_component = component(b, 0x120);
        m.head_component = component(b, 0x140);
        for (i, c) in m.components.iter_mut().enumerate() {
            *c = component(b, 0x160 + i * 32);
        }
        for (i, c) in m.fallback_components.iter_mut().enumerate() {
            *c = component(b, 0x300 + i * 32);
        }
        let mut xuid = [0u8; 8];
        xuid.copy_from_slice(&b[0x380..0x388]);
        m.owner_xuid = u64::from_be_bytes(xuid);
        m.source_console_id.copy_from_slice(&b[0x388..0x38D]);
        Ok(m)
    }

    /// GetBodyType: the stock male/female body ids, else the gender nibble of a closet body.
    pub fn body_type(&self) -> BodyType {
        const TAIL: [u8; 8] = [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0];
        let male = AssetId {
            a: 2,
            b: 0,
            c: 1,
            d: TAIL,
        };
        let female = AssetId {
            a: 2,
            b: 0,
            c: 2,
            d: TAIL,
        };
        let id = self.body_component.asset_id;
        if id == male {
            return BodyType::Male;
        }
        if id == female {
            return BodyType::Female;
        }
        match id.c {
            1 => BodyType::Male,
            2 => BodyType::Female,
            _ => BodyType::Unknown,
        }
    }
}

pub fn body_type_name(t: BodyType) -> &'static str {
    match t {
        BodyType::Male => "male",
        BodyType::Female => "female",
        BodyType::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_size() {
        assert!(Metadata::parse(&[0u8; 999]).is_err());
    }

    #[test]
    fn reads_fields() {
        let mut b = vec![0u8; METADATA_SIZE];
        b[0x004..0x008].copy_from_slice(&0.5f32.to_be_bytes());
        b[0x0FC..0x100].copy_from_slice(&0xFF102030u32.to_be_bytes());
        b[0x120..0x124].copy_from_slice(&2u32.to_be_bytes());
        b[0x126..0x128].copy_from_slice(&2u16.to_be_bytes());
        b[0x130..0x132].copy_from_slice(&0x0002u16.to_be_bytes());
        let m = Metadata::parse(&b).unwrap();
        assert_eq!(m.weight_factor, 0.5);
        assert_eq!(m.colors[0], 0xFF102030);
        assert_eq!(m.body_component.categories, 2);
        assert_eq!(m.body_type(), BodyType::Female);
        assert_eq!(color_to_float4(0xFF0000FF), [0.0, 0.0, 1.0, 1.0]);
    }
}
