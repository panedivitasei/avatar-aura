// Port of xenia/kernel/xam/avatars/asset_pack.{h,cpp} and asset_pack_internal.h.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// Avatar asset GUID; `a`, `b`, `c` are stored big-endian on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

impl AssetId {
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        let mut d = [0u8; 8];
        d.copy_from_slice(&bytes[8..16]);
        Self {
            a: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            b: u16::from_be_bytes([bytes[4], bytes[5]]),
            c: u16::from_be_bytes([bytes[6], bytes[7]]),
            d,
        }
    }

    pub fn to_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.a.to_be_bytes());
        out[4..6].copy_from_slice(&self.b.to_be_bytes());
        out[6..8].copy_from_slice(&self.c.to_be_bytes());
        out[8..16].copy_from_slice(&self.d);
        out
    }

    pub fn is_zero(&self) -> bool {
        *self == AssetId::default()
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = &self.d;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.a, self.b, self.c, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }
}

impl FromStr for AssetId {
    type Err = Error;

    /// Parses the 8-4-4-4-12 text form (closet.cpp ParseAssetId).
    fn from_str(s: &str) -> Result<Self> {
        let b = s.as_bytes();
        if !s.is_ascii() || b.len() != 36 || b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return Err(Error::AssetIdText);
        }
        let hex = |part: &str| -> Result<u64> {
            if part.is_empty() || !part.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(Error::AssetIdText);
            }
            u64::from_str_radix(part, 16).map_err(|_| Error::AssetIdText)
        };
        let a = hex(&s[0..8])?;
        let bv = hex(&s[9..13])?;
        let c = hex(&s[14..18])?;
        let d0 = hex(&s[19..23])?;
        let d1 = hex(&s[24..36])?;
        let mut d = [0u8; 8];
        d[0] = (d0 >> 8) as u8;
        d[1] = d0 as u8;
        for (i, slot) in d[2..].iter_mut().enumerate() {
            *slot = (d1 >> (8 * (5 - i))) as u8;
        }
        Ok(AssetId {
            a: a as u32,
            b: bv as u16,
            c: c as u16,
            d,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInfo {
    pub categories: u32,
    pub bodies: u8,
    pub random_bodies: u8,
    pub flags: u8,
    pub unknown_007: [u8; 0x91],
    pub subcategory: u32,
    pub asset_ids: [AssetId; 2],
    pub name_offset: usize,
    pub data_offset: usize,
    pub data_size: usize,
}

impl Default for AssetInfo {
    fn default() -> Self {
        Self {
            categories: 0,
            bodies: 0,
            random_bodies: 0,
            flags: 0,
            unknown_007: [0; 0x91],
            subcategory: 0,
            asset_ids: [AssetId::default(); 2],
            name_offset: 0,
            data_offset: 0,
            data_size: 0,
        }
    }
}

const TOC_V1: [u8; 16] = [
    0x9A, 0xD6, 0xEB, 0xCE, 0x62, 0x62, 0x4E, 0xEB, 0x8A, 0x82, 0xA3, 0xF7, 0x0B, 0x81, 0x73, 0x69,
];
const TOC_V2: [u8; 16] = [
    0x8A, 0x76, 0x2D, 0xF4, 0xC0, 0x67, 0x4B, 0x66, 0xB4, 0xEE, 0x56, 0x46, 0x6B, 0x1B, 0x82, 0x80,
];
const TOC_V3: [u8; 16] = [
    0x58, 0x0A, 0x07, 0xD6, 0x4B, 0xCD, 0x40, 0xBE, 0xBF, 0x37, 0x25, 0x4B, 0xE8, 0x26, 0xFB, 0x0B,
];

/// The stock avatar GUID (kGenericAvatarGuid in the C++).
pub const GENERIC_AVATAR_GUID: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x04, 0x48, 0x00, 0x01, 0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0,
];

fn be32(data: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
}

/// File header size and per-asset header size/name-offset count for each TOC version.
fn layout(version: usize) -> (usize, usize, usize) {
    match version {
        1 => (0x20, 0xF8, 13),
        2 => (0x28, 0xF8, 13),
        _ => (0x30, 0x10C, 18),
    }
}

fn load_infos(data: &[u8], version: usize) -> Result<Vec<AssetInfo>> {
    let (file_header_size, asset_header_size, name_offset_count) = layout(version);
    if data.len() < file_header_size {
        return Err(Error::Malformed("asset pack file header truncated"));
    }
    let asset_count = be32(data, 0x1C) as usize;
    let headers_end = asset_count
        .checked_mul(asset_header_size)
        .and_then(|n| n.checked_add(file_header_size))
        .ok_or(Error::Malformed("asset pack header size overflow"))?;
    if data.len() < headers_end {
        return Err(Error::Malformed("asset pack asset headers truncated"));
    }

    let mut infos = Vec::with_capacity(asset_count);
    for i in 0..asset_count {
        let h = &data[file_header_size + i * asset_header_size..][..asset_header_size];
        let names = 0xBC;
        let data_offset = be32(h, names + name_offset_count * 4) as usize;
        let data_size = be32(h, names + name_offset_count * 4 + 4) as usize;
        if data.len() < data_offset + data_size {
            infos.push(AssetInfo::default());
            continue;
        }
        let mut unknown_007 = [0u8; 0x91];
        unknown_007.copy_from_slice(&h[7..7 + 0x91]);
        let mut id0 = [0u8; 16];
        let mut id1 = [0u8; 16];
        id0.copy_from_slice(&h[0x9C..0xAC]);
        id1.copy_from_slice(&h[0xAC..0xBC]);
        infos.push(AssetInfo {
            categories: be32(h, 0),
            bodies: h[4],
            random_bodies: h[5],
            flags: h[6],
            unknown_007,
            subcategory: be32(h, 0x98),
            asset_ids: [AssetId::from_bytes(&id0), AssetId::from_bytes(&id1)],
            name_offset: be32(h, names + 4) as usize,
            data_offset,
            data_size,
        });
    }
    Ok(infos)
}

/// A loaded AvatarAssetPack.toc.
#[derive(Clone, Debug, Default)]
pub struct AssetPack {
    data: Vec<u8>,
    infos: Vec<AssetInfo>,
}

impl AssetPack {
    pub fn load(data: Vec<u8>) -> Result<Self> {
        if data.len() < 16 {
            return Err(Error::Malformed("asset pack too small for a version guid"));
        }
        let version = [TOC_V1, TOC_V2, TOC_V3]
            .iter()
            .position(|g| data[..16] == g[..])
            .map(|i| i + 1)
            .ok_or(Error::UnknownPackVersion)?;
        let infos = load_infos(&data, version)?;
        Ok(Self { data, infos })
    }

    pub fn asset_infos(&self) -> &[AssetInfo] {
        &self.infos
    }

    /// Resolves through `id.b`, the pack index.
    pub fn find(&self, id: AssetId) -> Option<&AssetInfo> {
        self.infos.get(id.b as usize)
    }

    pub fn asset_data(&self, id: AssetId) -> Option<&[u8]> {
        self.asset_data_by_index(id.b as usize)
    }

    /// Fetches by pack position; several assets can share a colliding or zero `id.b`.
    pub fn asset_data_by_index(&self, index: usize) -> Option<&[u8]> {
        let info = self.infos.get(index)?;
        self.data.get(info.data_offset..info.data_offset + info.data_size)
    }

    pub fn asset_name(&self, id: AssetId) -> String {
        self.asset_name_by_index(id.b as usize)
    }

    /// NUL-terminated UTF-16BE name; empty when the asset has none.
    pub fn asset_name_by_index(&self, index: usize) -> String {
        let Some(info) = self.infos.get(index) else {
            return String::new();
        };
        if info.name_offset == 0 {
            return String::new();
        }
        let Some(bytes) = self.data.get(info.name_offset..) else {
            return String::new();
        };
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_id_text_round_trip() {
        let text = "00000008-204f-4251-ceca-b43857520802";
        let id: AssetId = text.parse().unwrap();
        assert_eq!(id.a, 8);
        assert_eq!(id.b, 0x204f);
        assert_eq!(id.c, 0x4251);
        assert_eq!(id.d, [0xce, 0xca, 0xb4, 0x38, 0x57, 0x52, 0x08, 0x02]);
        assert_eq!(id.to_string(), text);
        assert_eq!(AssetId::from_bytes(&id.to_bytes()), id);
        let upper: AssetId = "00000008-204F-4251-CECA-B43857520802".parse().unwrap();
        assert_eq!(upper, id);
    }

    #[test]
    fn asset_id_rejects_bad_text() {
        for bad in [
            "",
            "00000008-204f-4251-ceca-b4385752080",
            "00000008-204f-4251-ceca_b43857520802",
            "0000000g-204f-4251-ceca-b43857520802",
            "00000008-204f-4251-ceca-b438575208022",
            "0000000\u{e9}-204f-4251-ceca-b4385752080",
        ] {
            assert!(bad.parse::<AssetId>().is_err(), "{bad}");
        }
    }
}
