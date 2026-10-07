// Decodes the fixture inputs and compares against the C++ avatarextract goldens.

use std::path::{Path, PathBuf};

use avatar_formats::animation::load_option as anim_opt;
use avatar_formats::closet::parse_index;
use avatar_formats::{strb, AssetPack, BlendShape, Model, Texture};

fn fixtures() -> Option<PathBuf> {
    let dir = std::env::var_os("AVATAR_AURA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\edward\Documents\ReXGlue\avatar-aura-rust-fixtures"));
    if dir.is_dir() {
        Some(dir)
    } else {
        println!("fixtures not found at {}, skipping", dir.display());
        None
    }
}

fn golden_positions(obj: &Path) -> Vec<[f32; 3]> {
    std::fs::read_to_string(obj)
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("v "))
        .map(|l| {
            let v: Vec<f32> = l.split_whitespace().map(|t| t.parse().unwrap()).collect();
            [v[0], v[1], v[2]]
        })
        .collect()
}

fn assert_positions(model: &Model, obj: &Path) {
    let golden = golden_positions(obj);
    let ours: Vec<_> = model
        .triangle_batches
        .iter()
        .flat_map(|b| b.vertices.iter().map(|v| v.position))
        .collect();
    assert_eq!(ours.len(), golden.len(), "{}", obj.display());
    for (i, (o, g)) in ours.iter().zip(&golden).enumerate() {
        for (a, b) in [o.x, o.y, o.z].iter().zip(g) {
            assert!(
                (a - b).abs() <= 1e-5,
                "{} vertex {i}: {o:?} vs {g:?}",
                obj.display()
            );
        }
    }
}

/// avatarextract's DecodeTexture: 16-bit word swap, linear blocks, BC1/2/3 or A8R8G8B8 to RGBA8.
fn decode_rgba8(tex: &Texture) -> Option<Vec<u8>> {
    if tex.is_empty || tex.data_bytes.is_empty() {
        return None;
    }
    let kind: u8 = match tex.format & 0x3F {
        18 => 1,
        19 => 2,
        20 => 3,
        6 => 0,
        _ => return None,
    };
    let (w, h) = (tex.width as usize, tex.height as usize);
    let block_dim = if kind == 0 { 1 } else { 4 };
    let bytes_per_block = match kind {
        1 => 8,
        0 => 4,
        _ => 16,
    };
    let (wb, hb) = (w.div_ceil(block_dim), h.div_ceil(block_dim));
    let mut src = tex.data_bytes.clone();
    for pair in src.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    if src.len() < wb * hb * bytes_per_block {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    if kind == 0 {
        for i in 0..w * h {
            let s = &src[i * 4..i * 4 + 4];
            out[i * 4..i * 4 + 4].copy_from_slice(&[s[2], s[1], s[0], s[3]]);
        }
        return Some(out);
    }
    for by in 0..hb {
        for bx in 0..wb {
            let block = &src[(by * wb + bx) * bytes_per_block..][..bytes_per_block];
            let color = if kind == 1 { block } else { &block[8..] };
            let pal = bc1_palette(color, kind == 1);
            let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
            let alpha = bc_alpha(kind, block);
            for y in 0..4 {
                for x in 0..4 {
                    let (px, py) = (bx * 4 + x, by * 4 + y);
                    if px >= w || py >= h {
                        continue;
                    }
                    let t = y * 4 + x;
                    let mut p = pal[((bits >> (2 * t)) & 3) as usize];
                    if let Some(a) = &alpha {
                        p[3] = a[t];
                    }
                    out[(py * w + px) * 4..][..4].copy_from_slice(&p);
                }
            }
        }
    }
    Some(out)
}

fn bc1_palette(block: &[u8], allow_alpha: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let expand = |c: u16| {
        let (r, g, b) = (
            ((c >> 11) & 0x1F) as u32,
            ((c >> 5) & 0x3F) as u32,
            (c & 0x1F) as u32,
        );
        [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
    };
    let (e0, e1) = (expand(c0), expand(c1));
    let opaque = c0 > c1 || !allow_alpha;
    let mut pal = [[0u8, 0, 0, 255]; 4];
    for i in 0..3 {
        pal[0][i] = e0[i] as u8;
        pal[1][i] = e1[i] as u8;
        if opaque {
            pal[2][i] = ((2 * e0[i] + e1[i]) / 3) as u8;
            pal[3][i] = ((e0[i] + 2 * e1[i]) / 3) as u8;
        } else {
            pal[2][i] = ((e0[i] + e1[i]) / 2) as u8;
        }
    }
    if !opaque {
        pal[3][3] = 0;
    }
    pal
}

fn bc_alpha(kind: u8, block: &[u8]) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    match kind {
        2 => {
            let bits = u64::from_le_bytes(block[..8].try_into().unwrap());
            for (t, a) in out.iter_mut().enumerate() {
                let a4 = ((bits >> (4 * t)) & 0xF) as u8;
                *a = (a4 << 4) | a4;
            }
        }
        3 => {
            let (a0, a1) = (u32::from(block[0]), u32::from(block[1]));
            let mut a = [a0, a1, 0, 0, 0, 0, 0, 0];
            if a0 > a1 {
                for i in 1..7u32 {
                    a[i as usize + 1] = ((7 - i) * a0 + i * a1) / 7;
                }
            } else {
                for i in 1..5u32 {
                    a[i as usize + 1] = ((5 - i) * a0 + i * a1) / 5;
                }
                a[6] = 0;
                a[7] = 255;
            }
            let mut bits = 0u64;
            for i in 0..6 {
                bits |= u64::from(block[2 + i]) << (8 * i);
            }
            for (t, o) in out.iter_mut().enumerate() {
                *o = a[((bits >> (3 * t)) & 7) as usize] as u8;
            }
        }
        _ => return None,
    }
    Some(out)
}

fn assert_texture_matches(tex: &Texture, png: &Path) {
    let golden = image::open(png).unwrap().to_rgba8();
    assert_eq!(
        (golden.width(), golden.height()),
        (tex.width, tex.height),
        "{}",
        png.display()
    );
    let ours = decode_rgba8(tex).expect("texture decodes");
    assert!(
        ours == golden.as_raw().as_slice(),
        "{} pixels differ",
        png.display()
    );
}

#[test]
fn asset_pack_body_matches_golden() {
    let Some(dir) = fixtures() else { return };
    let pack = AssetPack::load(std::fs::read(dir.join("inputs/AvatarAssetPack.toc")).unwrap()).unwrap();
    assert_eq!(pack.asset_infos().len(), 1282);
    assert_eq!(pack.asset_name_by_index(0), "MaleBody");
    let data = pack.asset_data_by_index(0).unwrap();
    let model = Model::load(data, 0).unwrap().unwrap();
    assert_eq!(model.triangle_batches.len(), 1);
    assert_eq!(model.vertex_count(), 1137);
    assert_eq!(model.triangle_count(), 2264);
    assert_positions(&model, &dir.join("golden/pack/body.obj"));
    let mut textures = 0;
    for (i, mt) in model.textures.iter().enumerate() {
        let png = dir.join(format!("golden/pack/body_texture{i}.png"));
        if png.exists() {
            assert_texture_matches(&mt.texture, &png);
            textures += 1;
        }
    }
    assert!(textures > 0);
}

#[test]
fn legacy_asset_pack_loads() {
    let Some(dir) = fixtures() else { return };
    let pack =
        AssetPack::load(std::fs::read(dir.join("inputs/AvatarAssetPackLegacyV1.toc")).unwrap()).unwrap();
    assert!(!pack.asset_infos().is_empty());
    let model = Model::load(pack.asset_data_by_index(0).unwrap(), 0)
        .unwrap()
        .unwrap();
    assert!(model.vertex_count() > 0);
}

#[test]
fn closet_items_match_golden() {
    let Some(dir) = fixtures() else { return };
    let mut compared = 0;
    for entry in std::fs::read_dir(dir.join("inputs/closet")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "bin") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let shapes = BlendShape::load_all(&bytes, 0).unwrap();
        assert_eq!(
            shapes.len(),
            strb::count_blocks(&bytes, strb::BlockId::ShapeOverrides).unwrap()
        );
        let model = Model::load(&bytes, 0).unwrap();
        let guid = path.file_stem().unwrap().to_string_lossy().into_owned();
        let golden = dir.join(format!("golden/item_{guid}"));
        if !golden.is_dir() {
            continue;
        }
        let model = model.expect("golden item has a model");
        assert_positions(&model, &golden.join("item.obj"));
        for (i, mt) in model.textures.iter().enumerate() {
            let png = golden.join(format!("item_texture{i}.png"));
            if png.exists() {
                assert_texture_matches(&mt.texture, &png);
            }
        }
        compared += 1;
    }
    assert_eq!(compared, 3);
}

#[test]
fn animations_load_with_elements() {
    let Some(dir) = fixtures() else { return };
    let mut count = 0;
    for entry in std::fs::read_dir(dir.join("inputs")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "AvatarAnimation") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let anim = avatar_formats::Animation::load(&bytes, anim_opt::ELEMENTS)
            .unwrap()
            .unwrap();
        assert!(anim.frame_count > 0, "{}", path.display());
        assert_eq!(anim.pose_frame_sets[0].frames.len(), anim.frame_count as usize);
        assert_eq!(
            anim.pose_frame_sets[0].frames[0].len(),
            anim.pose_counts[0] as usize
        );
        let lean = avatar_formats::Animation::load(&bytes, anim_opt::COMPRESSED_DATA)
            .unwrap()
            .unwrap();
        assert!(lean.pose_frame_sets[0].frames.is_empty());
        assert!(!lean.compressed_data_bytes.is_empty());
        count += 1;
    }
    assert!(count > 0);
}

#[test]
fn closet_index_parses_every_row() {
    let Some(dir) = fixtures() else { return };
    let closet_dir = dir.join("inputs/closet");
    let text = std::fs::read_to_string(closet_dir.join("closet_index.tsv")).unwrap();
    let rows = text.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(parse_index(&text).len(), rows);
    let closet = avatar_formats::Closet::load(&closet_dir).unwrap();
    assert!(closet.is_loaded());
    assert_eq!(closet.items().len(), rows);
    for item in closet.items() {
        assert_eq!(closet.find(&item.id).unwrap().id, item.id);
    }
    let id: avatar_formats::AssetId = "00000008-204f-4251-ceca-b43857520802".parse().unwrap();
    assert!(closet.read_item_bytes(&id).is_some());
}

#[test]
fn item_shape_applies_to_stock_body() {
    let Some(dir) = fixtures() else { return };
    let pack = AssetPack::load(std::fs::read(dir.join("inputs/AvatarAssetPack.toc")).unwrap()).unwrap();
    let mut body = Model::load(pack.asset_data_by_index(0).unwrap(), 0)
        .unwrap()
        .unwrap();
    let before = body.clone();
    let item = std::fs::read(dir.join("inputs/closet/00000008-204f-4251-ceca-b43857520802.bin")).unwrap();
    let shape = BlendShape::load(&item, 0).unwrap().unwrap();
    assert_eq!(shape.index_patch.indices.len(), 340);
    assert_eq!(shape.vertex_patch.vertices.len(), 64);
    let target = shape.index_patch.original_asset_id;
    assert!(avatar_formats::blend_shape::apply::apply_blend_shape(
        &shape, &target, &mut body
    ));
    assert_ne!(
        body.triangle_batches[0].indices,
        before.triangle_batches[0].indices
    );
    let wrong = avatar_formats::AssetId { a: 3, ..target };
    assert!(!avatar_formats::blend_shape::apply::apply_blend_shape(
        &shape, &wrong, &mut body
    ));
}
