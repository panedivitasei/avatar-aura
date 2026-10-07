// The bundled catalog.json (expression and clip names with thumbnails) and the clip list it seeds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::paths;

#[derive(Clone, Debug, Deserialize)]
pub struct CatalogExpression {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CatalogClip {
    pub name: String,
    pub fps: f64,
    pub frames: u32,
    pub thumb: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Catalog {
    pub expressions: Vec<CatalogExpression>,
    pub clips: Vec<CatalogClip>,
}

impl Catalog {
    /// The catalog beside the assets, else the embedded copy.
    pub fn load() -> anyhow::Result<Self> {
        let path = paths::asset(&["catalog.json"]);
        let text = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => paths::bundled_bytes("catalog.json")
                .ok_or_else(|| anyhow::anyhow!("catalog.json is missing"))?
                .to_vec(),
        };
        Ok(serde_json::from_slice(&text)?)
    }
}

/// Thumbnail file name for a clip, `clip_<safe name>.png`.
pub fn clip_thumb(name: &str) -> String {
    format!("clip_{}.png", paths::safe_name(name))
}

/// Thumbnail file name for an expression id such as `mouth:3`.
pub fn expression_thumb(id: &str) -> String {
    format!("{}.png", paths::safe_name(id))
}

/// Decodes every PNG under `assets/thumbs`, keyed by file name.
pub fn decode_thumbs(dir: &Path) -> HashMap<String, image::RgbaImage> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
            if let Ok(img) = image::open(&path) {
                let name = entry.file_name().to_string_lossy().into_owned();
                out.insert(name, img.to_rgba8());
            }
        }
    }
    out
}

/// Where a clip comes from: a pack animation by name or an `.AvatarAnimation` file.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipSource {
    Pack,
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct ClipEntry {
    pub name: String,
    pub fps: f64,
    pub frames: u32,
    pub thumb: String,
    pub source: ClipSource,
}

/// Catalog clips, then every `.AvatarAnimation` in `anim_dir` not already listed; bundled clips map to their files.
pub fn clip_list(catalog: &Catalog, anim_dir: &Path) -> Vec<ClipEntry> {
    let bundled = paths::asset(&["clips"]);
    let mut clips: Vec<ClipEntry> = catalog
        .clips
        .iter()
        .map(|c| {
            let source = if c.name.starts_with("Animation ") {
                ClipSource::Pack
            } else {
                ClipSource::File(bundled.join(format!("{}.AvatarAnimation", c.name)))
            };
            ClipEntry {
                name: c.name.clone(),
                fps: c.fps,
                frames: c.frames,
                thumb: c.thumb.clone(),
                source,
            }
        })
        .collect();
    for path in anim_files(anim_dir) {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() || clips.iter().any(|c| c.name == name) {
            continue;
        }
        clips.push(ClipEntry {
            thumb: clip_thumb(&name),
            name,
            fps: 30.0,
            frames: 0,
            source: ClipSource::File(path),
        });
    }
    clips
}

/// Sorted `*.AvatarAnimation` files of a folder; empty when the folder is unset or missing.
pub fn anim_files(dir: &Path) -> Vec<PathBuf> {
    if dir.as_os_str().is_empty() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("AvatarAnimation"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses() {
        let bytes = paths::bundled_bytes("catalog.json").unwrap();
        let c: Catalog = serde_json::from_slice(bytes).unwrap();
        assert_eq!(c.clips.len(), 44);
        assert!(c.expressions.iter().any(|e| e.id == "brows:2"));
        assert_eq!(expression_thumb("brows:2"), "brows2.png");
    }
}
