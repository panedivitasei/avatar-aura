// Port of avatar_aura/faces.py: one expression id selects the same head composites in preview and export.
// Paths are returned relative to the avatar folder; turning them into media URLs is the caller's job.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use image::RgbaImage;

use crate::avatar::Avatar;
use crate::error::{invalid, io_err, Error, Result};
use crate::math::{psum, safe_id, title_case};

/// Face channel to frame; `None` leaves the channel at the neutral composite.
pub type Selection = BTreeMap<String, Option<u32>>;

const MIX_CHANNELS: [&str; 7] = [
    "mouth",
    "eyes",
    "brows",
    "eye_left",
    "eye_right",
    "brow_left",
    "brow_right",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Expression {
    /// "neutral" or "<channel>:<frame>".
    Named(String),
    /// Per-channel frames mixed into a new composite.
    Mix(Selection),
}

impl Expression {
    pub fn neutral() -> Self {
        Self::Named("neutral".into())
    }
}

/// Head material textures for an expression, keyed by material index.
pub fn texture_files(avatar: &Avatar, expression: &Expression) -> Result<BTreeMap<usize, String>> {
    let name = match expression {
        Expression::Mix(selection) => {
            let any_set = selection.values().any(|v| matches!(v, Some(f) if *f != 0));
            if avatar.face.composite_files.is_empty() && !any_set {
                return texture_files(avatar, &Expression::neutral());
            }
            return mixed_textures(avatar, selection);
        }
        Expression::Named(name) => name.as_str(),
    };
    let heads: BTreeSet<&str> = avatar.face.head_materials.iter().map(String::as_str).collect();
    let mut result: BTreeMap<usize, String> = avatar
        .materials
        .iter()
        .enumerate()
        .filter(|(_, md)| heads.contains(md.name.as_str()))
        .map(|(i, md)| (i, md.diffuse.clone()))
        .collect();
    if name == "neutral" {
        return Ok(result);
    }
    let entries: Vec<_> = avatar
        .face
        .composite_files
        .iter()
        .filter(|e| format!("{}:{}", e.channel, e.frame) == name)
        .collect();
    if entries.is_empty() {
        return Err(invalid(format!("Unknown expression: {name}")));
    }
    for (index, file) in result.iter_mut() {
        let material = &avatar.materials[*index].name;
        let found = entries.iter().find(|e| {
            let expected = format!("{material}_{}_{:02}.png", e.channel, e.frame);
            Path::new(&e.file).file_name().and_then(|n| n.to_str()) == Some(expected.as_str())
        });
        match found {
            Some(e) => *file = e.file.clone(),
            None => {
                return Err(invalid(format!(
                    "Expression {name} has no composite for {material}"
                )))
            }
        }
    }
    Ok(result)
}

/// The four RGBA bands of a PNG as signed integers.
fn image_bands(avatar: &Avatar, file: &str) -> Result<(u32, u32, Vec<[i32; 4]>)> {
    let path = avatar.file(file);
    let img = image::open(&path)
        .map_err(|source| Error::Image {
            path: path.clone(),
            source,
        })?
        .to_rgba8();
    let px = img.pixels().map(|p| p.0.map(i32::from)).collect();
    Ok((img.width(), img.height(), px))
}

/// `json.dumps(selection, sort_keys=True)`.
fn selection_json(selection: &Selection) -> String {
    let parts: Vec<String> = selection
        .iter()
        .map(|(k, v)| {
            let value = v.map_or_else(|| "null".to_string(), |f| f.to_string());
            format!("{}: {value}", serde_json::Value::String(k.clone()))
        })
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// Mixes per-channel composites into `face/mix_v2_<index>_<digest>.png`, cached on disk.
pub fn mixed_textures(avatar: &Avatar, selection: &Selection) -> Result<BTreeMap<usize, String>> {
    for channel in selection.keys() {
        if !MIX_CHANNELS.contains(&channel.as_str()) {
            return Err(invalid(format!("Invalid face channel: {channel}.")));
        }
    }
    if selection.values().all(Option::is_none) {
        return texture_files(avatar, &Expression::neutral());
    }
    let neutral = texture_files(avatar, &Expression::Named("eyes:0".into()))?;
    let digest: String = sha256_hex(selection_json(selection).as_bytes())[..16].to_string();
    let mut result = BTreeMap::new();
    for (&index, neutral_file) in &neutral {
        let relative = format!("face/mix_v2_{index}_{digest}.png");
        let output = avatar.file(&relative);
        if !output.exists() {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(io_err(parent))?;
            }
            write_mix(avatar, selection, index, neutral_file, &output)?;
        }
        result.insert(index, relative);
    }
    Ok(result)
}

/// `selection.get(key, default)` over the optional frames.
fn get_or(selection: &Selection, key: &str, default: Option<u32>) -> Option<u32> {
    match selection.get(key) {
        Some(v) => *v,
        None => default,
    }
}

fn write_mix(
    avatar: &Avatar,
    selection: &Selection,
    index: usize,
    neutral_file: &str,
    output: &Path,
) -> Result<()> {
    let xs: Vec<f64> = avatar
        .meshes
        .iter()
        .filter(|m| m.material == index)
        .flat_map(|m| m.positions.iter().map(|p| p[0]))
        .collect();
    let count = xs.len();
    let sum = psum(xs);
    let center = avatar
        .skeleton
        .iter()
        .find(|j| j.name == "HEAD")
        .map_or(0.0, |j| j.rest_world[0]);
    let side = if count > 0 && sum / count as f64 >= center {
        "left"
    } else {
        "right"
    };
    let frames = [
        ("mouth", get_or(selection, "mouth", Some(0))),
        (
            "eyes",
            get_or(
                selection,
                &format!("eye_{side}"),
                get_or(selection, "eyes", Some(0)),
            ),
        ),
        (
            "brows",
            get_or(
                selection,
                &format!("brow_{side}"),
                get_or(selection, "brows", Some(0)),
            ),
        ),
    ];
    let (width, height, base) = image_bands(avatar, neutral_file)?;
    let mut combined = base.clone();
    for (channel, frame) in frames {
        let Some(frame) = frame.filter(|f| *f != 0) else {
            continue;
        };
        let files = texture_files(avatar, &Expression::Named(format!("{channel}:{frame}")))?;
        let file = files
            .get(&index)
            .ok_or_else(|| invalid(format!("Expression {channel}:{frame} has no composite")))?;
        let (lw, lh, layer) = image_bands(avatar, file)?;
        if (lw, lh) != (width, height) {
            return Err(invalid("Face composites must have matching dimensions."));
        }
        for ((c, l), n) in combined.iter_mut().zip(&layer).zip(&base) {
            for band in 0..4 {
                c[band] = c[band] + l[band] - n[band];
            }
        }
    }
    let bytes: Vec<u8> = combined
        .iter()
        .flat_map(|p| p.map(|v| v.clamp(0, 255) as u8))
        .collect();
    let img = RgbaImage::from_raw(width, height, bytes)
        .ok_or_else(|| invalid("Face composite has an unexpected size."))?;
    img.save(output).map_err(|source| Error::Image {
        path: output.to_path_buf(),
        source,
    })
}

/// A mixed expression and the absolute texture path per head material safe id.
#[derive(Clone, Debug, PartialEq)]
pub struct MixedEntry {
    pub id: Expression,
    pub textures: BTreeMap<String, PathBuf>,
}

pub fn mixed_catalog_entry(avatar: &Avatar, selection: &Expression) -> Result<MixedEntry> {
    let files = texture_files(avatar, selection)?;
    Ok(MixedEntry {
        id: selection.clone(),
        textures: files
            .into_iter()
            .map(|(i, path)| (safe_id(&avatar.materials[i].name), avatar.file(&path)))
            .collect(),
    })
}

/// One selectable expression; `thumb` lookup stays with the app.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogExpression {
    pub id: String,
    pub name: String,
    pub textures: BTreeMap<String, PathBuf>,
}

/// Neutral plus every distinct composite (or layer) channel frame.
pub fn catalog(avatar: &Avatar) -> Result<Vec<CatalogExpression>> {
    let mut expressions = vec![("neutral".to_string(), "Neutral".to_string())];
    let mut seen = BTreeSet::new();
    let face = &avatar.face;
    let source = if face.composite_files.is_empty() {
        &face.layer_files
    } else {
        &face.composite_files
    };
    for entry in source {
        let key = format!("{}:{}", entry.channel, entry.frame);
        if !seen.insert(key.clone()) {
            continue;
        }
        let labels = face.layer_names.get(&entry.channel);
        let label = match labels.and_then(|l| l.get(entry.frame as usize)) {
            Some(l) => title_case(&l.replace('_', " ")),
            None => entry.frame.to_string(),
        };
        expressions.push((key, format!("{} · {label}", title_case(&entry.channel))));
    }
    let mut out = Vec::with_capacity(expressions.len());
    for (id, name) in expressions {
        let textures = if id == "neutral" || !face.composite_files.is_empty() {
            texture_files(avatar, &Expression::Named(id.clone()))?
                .into_iter()
                .map(|(i, path)| (safe_id(&avatar.materials[i].name), avatar.file(&path)))
                .collect()
        } else {
            BTreeMap::new()
        };
        out.push(CatalogExpression { id, name, textures });
    }
    Ok(out)
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
