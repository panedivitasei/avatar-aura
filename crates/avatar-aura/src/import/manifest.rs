// Port of avatar_aura/manifest.py: the 1000-byte big-endian X_AVATAR_METADATA and the mannequin writer.

use anyhow::bail;

pub const SIZE: usize = 1000;
const STOCK_TAIL: [u8; 8] = [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0];
const BODY: usize = 0x120;
const HEAD: usize = 0x140;
const CATEGORY_OFFSET: usize = 16;
const COMPONENTS: usize = 0x160;
const COLORS: (usize, usize) = (0x0FC, 9);
/// weight, height, blend shapes, textures, components, fallbacks: offset and byte length.
const CLEARED: [(usize, usize); 6] = [
    (0x004, 4),
    (0x008, 4),
    (0x00C, 16 * 3),
    (0x03C, 32 * 6),
    (0x160, 32 * 13),
    (0x300, 32 * 4),
];

fn guid_bytes(guid: &str) -> anyhow::Result<[u8; 16]> {
    let hex = guid.replace('-', "");
    if hex.len() != 32 {
        bail!("{guid} is not a product guid");
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}

/// Closet guids worn by a manifest: body, head and the thirteen component slots, skipping empty and stock ids.
pub fn closet_guids(data: &[u8]) -> Vec<String> {
    if data.len() != SIZE {
        return Vec::new();
    }
    let offsets = [BODY, HEAD]
        .into_iter()
        .chain((0..13).map(|i| COMPONENTS + i * 32));
    offsets
        .filter_map(|o| {
            let g = &data[o..o + 16];
            (g.iter().any(|b| *b != 0) && g[8..] != STOCK_TAIL).then(|| {
                let h: String = g.iter().map(|b| format!("{b:02x}")).collect();
                format!(
                    "{}-{}-{}-{}-{}",
                    &h[0..8],
                    &h[8..12],
                    &h[12..16],
                    &h[16..20],
                    &h[20..32]
                )
            })
        })
        .collect()
}

/// The mannequin with its outfit cleared, white colours and the item worn in the first component slot.
pub fn wear_on_manifest(base: &[u8], guid: &str, categories: u32) -> anyhow::Result<Vec<u8>> {
    if base.len() != SIZE {
        bail!("mannequin manifest is not 1000 bytes");
    }
    let mut data = base.to_vec();
    for (offset, len) in CLEARED {
        data[offset..offset + len].fill(0);
    }
    for i in 0..COLORS.1 {
        let o = COLORS.0 + i * 4;
        data[o..o + 4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    }
    data[COMPONENTS..COMPONENTS + 16].copy_from_slice(&guid_bytes(guid)?);
    let cat = (categories & 0x1FFF) as u16;
    data[COMPONENTS + CATEGORY_OFFSET..COMPONENTS + CATEGORY_OFFSET + 2].copy_from_slice(&cat.to_be_bytes());
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wears_item_on_bundled_mannequin() {
        let base = crate::paths::bundled_bytes("mannequins/mannequin_male.amd").unwrap();
        let worn = wear_on_manifest(base, "00000008-0004-c172-caeb-f9c44d5308c9", 8).unwrap();
        assert_eq!(worn.len(), SIZE);
        assert_eq!(&worn[COMPONENTS..COMPONENTS + 4], &[0, 0, 0, 8]);
        assert_eq!(&worn[COMPONENTS + 16..COMPONENTS + 18], &[0, 8]);
        assert_eq!(closet_guids(&worn), vec!["00000008-0004-c172-caeb-f9c44d5308c9"]);
    }
}
