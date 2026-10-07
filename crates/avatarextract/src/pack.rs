// Port of mode 2 (avatar asset pack -> body.obj, RunPackMode) of avatarextract/main.cpp.
//! Lists an AvatarAssetPack.toc and writes its base body (or a chosen asset) as OBJ.
//! The base body id is {a=2, b=0, c=1 male / c=2 female}; the pack indexes by id.b, so both resolve to index 0.

use std::io::Write;
use std::path::Path;

use avatar_formats::model::load_option as model_option;
use avatar_formats::{AssetId, AssetPack, BlendShape, Model};

use crate::objwrite::write_model;

/// ComponentCategory bits from guest_asset.h.
pub mod category {
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

/// `|`-joined names of the category bits set in `categories`, `(none)` when there are none.
pub fn category_names(categories: u32) -> String {
    use category::*;
    const CATS: [(u32, &str); 14] = [
        (HEAD, "Head"),
        (BODY, "Body"),
        (HAIR, "Hair"),
        (TOP, "Top"),
        (BOTTOM, "Bottom"),
        (SHOES, "Shoes"),
        (HAT, "Hat"),
        (GLOVES, "Gloves"),
        (GLASSES, "Glasses"),
        (WRISTWEAR, "Wristwear"),
        (EARRINGS, "Earrings"),
        (RING, "Ring"),
        (PROP, "Prop"),
        (ANIMATION, "Animation"),
    ];
    let names: Vec<&str> = CATS
        .iter()
        .filter(|(bit, _)| categories & bit != 0)
        .map(|&(_, n)| n)
        .collect();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join("|")
    }
}

/// UTF-16 code units below 0x80 pass through, everything else prints as `?`.
pub fn ascii_name(s: &str) -> String {
    s.encode_utf16()
        .map(|u| if u < 0x80 { char::from(u as u8) } else { '?' })
        .collect()
}

/// Asset name through id 0, falling back to id 1 (both resolve via `id.b`).
fn asset_name(pack: &AssetPack, ids: &[AssetId; 2]) -> String {
    let mut name = pack.asset_name(ids[0]);
    if name.is_empty() {
        name = pack.asset_name(ids[1]);
    }
    ascii_name(&name)
}

/// Mode 2: lists the pack, then writes asset `want_index` (or the base body when negative) as `body.obj`.
pub fn run_pack(in_path: &Path, out_dir: &Path, want_index: i32, log: &mut dyn Write) -> i32 {
    let toc = match std::fs::read(in_path) {
        Ok(b) => b,
        Err(_) => {
            say!(log, "ERROR: cannot open asset pack: {}\n", in_path.display());
            return 1;
        }
    };
    if toc.is_empty() {
        say!(log, "ERROR: empty asset pack.\n");
        return 1;
    }
    let Ok(pack) = AssetPack::load(toc) else {
        say!(
            log,
            "ERROR: AssetPack::Load failed (unknown version / corrupt header).\n"
        );
        return 2;
    };
    let infos = pack.asset_infos();
    say!(log, "=== AvatarAssetPack: {} assets ===\n", infos.len());
    say!(
        log,
        "  {:<5} {:<28} {:<10} {:<6} {:<38} {:<12} {}\n",
        "idx",
        "category",
        "subcat",
        "bodies",
        "asset_id",
        "data_size",
        "name"
    );
    for (i, a) in infos.iter().enumerate() {
        say!(
            log,
            "  {:<5} {:<28} 0x{:<8X} {:<6} {:<38} {:<12} {}\n",
            i,
            category_names(a.categories),
            a.subcategory,
            a.bodies,
            a.asset_ids[0].to_string(),
            a.data_size,
            asset_name(&pack, &a.asset_ids)
        );
    }
    say!(log, "\n");

    let body_assets: Vec<usize> = infos
        .iter()
        .enumerate()
        .filter(|(_, a)| a.categories & category::BODY != 0)
        .map(|(i, _)| i)
        .collect();
    say!(log, "=== Body-category assets (categories & kBody) ===\n");
    if body_assets.is_empty() {
        say!(log, "  (none found; falling back to asset index 0)\n");
    }
    for &idx in &body_assets {
        let a = &infos[idx];
        say!(
            log,
            "  idx={} subcat=0x{:X} bodies={} name=\"{}\" id={} size={}\n",
            idx,
            a.subcategory,
            a.bodies,
            asset_name(&pack, &a.asset_ids),
            a.asset_ids[0],
            a.data_size
        );
    }
    say!(log, "\n");

    // Explicit --index wins, then the canonical body at id.b == 0, then the first Body asset.
    let mut chosen = i64::from(want_index);
    let mut reason = "explicit --index";
    if chosen < 0 {
        if infos.first().is_some_and(|a| a.categories & category::BODY != 0) {
            chosen = 0;
            reason = "canonical body id.b==0 (Body category)";
        } else if let Some(&first) = body_assets.first() {
            chosen = first as i64;
            reason = "first Body-category asset";
        } else if !infos.is_empty() {
            chosen = 0;
            reason = "fallback to asset index 0";
        }
    }
    if chosen < 0 || chosen >= infos.len() as i64 {
        say!(
            log,
            "ERROR: no asset to extract (chosen index {chosen} out of range).\n"
        );
        return 3;
    }
    say!(log, "Chosen base body asset: index {chosen} ({reason})\n");

    // By index, not id: id.b collides (or is zero) across this pack. Load option NONE keeps the rest pose of mode 1.
    let Some(buf) = pack.asset_data_by_index(chosen as usize) else {
        say!(log, "ERROR: GetAssetDataByIndex failed for chosen asset.\n");
        return 4;
    };
    let b = |i: usize| buf.get(i).copied().unwrap_or(0);
    say!(
        log,
        "Asset bytes: {} (first4: {:02X} {:02X} {:02X} {:02X})\n",
        buf.len(),
        b(0),
        b(1),
        b(2),
        b(3)
    );

    let Ok(Some(model)) = Model::load(buf, model_option::NONE) else {
        // Not a model; a blend shape (a wearable's body-hiding template) prints its patch structure.
        if let Ok(Some(shape)) = BlendShape::load(buf, avatar_formats::blend_shape::load_option::NONE) {
            let ip = &shape.index_patch;
            let vp = &shape.vertex_patch;
            say!(log, "=== blend shape ===\n");
            say!(
                log,
                "  index_patch : original_id={} total_buffer_size=0x{:X} indices={}\n",
                ip.original_asset_id,
                ip.total_buffer_size,
                ip.indices.len()
            );
            say!(
                log,
                "  vertex_patch: original_id={} total_buffer_size=0x{:X} vertices={}\n",
                vp.original_asset_id,
                vp.total_buffer_size,
                vp.vertices.len()
            );
            return 0;
        }
        say!(
            log,
            "ERROR: Model::Load failed for body asset (no kModel STRB block, or\n\
             this asset is a skeleton/animation rather than a renderable model).\n"
        );
        return 5;
    };

    let Some(t) = write_model(&model, out_dir, "body", log) else {
        return 6;
    };
    say!(log, "\n=== Done ===\n");
    say!(
        log,
        "Wrote base body (asset idx {chosen}) with {} batches, {} verts, {} tris.\n",
        model.triangle_batches.len(),
        t.vertices,
        t.triangles
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_names_join_and_none() {
        assert_eq!(category_names(0), "(none)");
        assert_eq!(category_names(category::BODY), "Body");
        assert_eq!(
            category_names(category::HEAD | category::PROP | category::ANIMATION),
            "Head|Prop|Animation"
        );
    }

    #[test]
    fn ascii_name_masks_non_ascii() {
        assert_eq!(ascii_name("Hat"), "Hat");
        assert_eq!(ascii_name("H\u{e9}t"), "H?t");
        assert_eq!(ascii_name("\u{1F600}"), "??");
    }
}
