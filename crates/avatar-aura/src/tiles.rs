// Strip tiles rendered from the loaded avatar after Load All, keyed like the bundled `assets/thumbs` files.
// Expression tiles use the GPU viewer with build_assets.py's head framing; clip tiles come from the bake's preview stage.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, Context};
use avatar_bake::util::sanitize_name;
use glam::Vec3;
use image::RgbaImage;

use crate::bake;
use crate::catalog::{self, ClipEntry, ClipSource};
use crate::paths;
use crate::session::{self, Loaded, TextureCache};
use crate::viewport::{self, Gpu};

pub const EXPRESSION_SIZE: u32 = 128;
pub const CLIP_SIZE: i32 = 192;
/// Clip tiles pose this fraction of the clip, as `--preview-frame 0.35` did for the bundled set.
const CLIP_FRAME: &str = "0.35";

/// One rendered tile under its thumbnail file name.
pub type Tile = (String, RgbaImage);

/// Tile count for a session: every expression plus one per clip (the face strip shares the clip tile).
pub fn total(loaded: &Loaded) -> usize {
    loaded.expressions.len() + loaded.clips.len()
}

/// Renders the expression tiles, reporting each finished tile; `None` when cancelled.
pub fn expression_tiles(
    loaded: &Loaded,
    gpu: &Gpu,
    cache: &TextureCache,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize),
) -> anyhow::Result<Option<Vec<Tile>>> {
    let size = EXPRESSION_SIZE;
    let blank = avatar_view::Viewer::new(&gpu.device, &gpu.queue, viewport::FORMAT, true);
    let backdrop = blank
        .render_to_rgba(&gpu.device, &gpu.queue, size, size)
        .map_err(|e| anyhow!("backdrop render: {e}"))?;
    let mut viewer = gpu.build_viewer(&loaded.scene, &loaded.dir)?;
    match viewer.clip_index(session::STAND_CLIP) {
        Some(stand) => viewer.set_clip_frame(stand, 0.0)?,
        None => viewer.set_rest_pose(),
    }
    let head = viewer
        .joint_names()
        .iter()
        .position(|n| n.eq_ignore_ascii_case("HEAD"));
    let target = match head {
        Some(i) => viewer.joint_worlds()[i].w_axis.truncate(),
        None => viewer.camera().target,
    } + Vec3::new(0.0, 0.05, 0.0);
    viewer
        .camera_mut()
        .look_from(target + Vec3::new(0.0, 0.0, 0.85), target);

    let mut tiles = Vec::with_capacity(loaded.expressions.len());
    let mut overridden = BTreeSet::new();
    for entry in &loaded.expressions {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let textures = session::catalog_textures(loaded, entry, cache)?;
        let current: BTreeSet<usize> = textures.iter().map(|(i, _)| *i).collect();
        for stale in overridden.difference(&current) {
            viewer.reset_material_texture(*stale)?;
        }
        for (index, image) in &textures {
            viewer.set_material_texture(*index, image)?;
        }
        overridden = current;
        let frame = viewer
            .render_to_rgba(&gpu.device, &gpu.queue, size, size)
            .map_err(|e| anyhow!("{} tile: {e}", entry.id))?;
        tiles.push((catalog::expression_thumb(&entry.id), key_out(&frame, &backdrop)));
        progress(tiles.len());
    }
    Ok(Some(tiles))
}

/// Clears the viewer's stage gradient so the tile is transparent like the bundled head shots.
fn key_out(frame: &RgbaImage, backdrop: &RgbaImage) -> RgbaImage {
    let mut out = frame.clone();
    for (px, bg) in out.pixels_mut().zip(backdrop.pixels()) {
        let diff = (0..3).map(|c| px.0[c].abs_diff(bg.0[c])).max().unwrap_or(0);
        // Ramp over a few levels so anti-aliased silhouette edges fade instead of stepping.
        let alpha = (f32::from(diff.saturating_sub(3)) / 15.0).min(1.0);
        px.0[3] = (alpha * 255.0).round() as u8;
    }
    out
}

/// Bakes the avatar once more with the preview stage on and returns one 192px tile per clip at 35% of its length.
pub fn clip_tiles(loaded: &Loaded, scratch: &std::path::Path) -> anyhow::Result<Vec<Tile>> {
    let clips: Vec<&ClipEntry> = loaded.clips.iter().collect();
    let mut query = session::query_for(&clips);
    if clips.iter().filter(|c| c.source == ClipSource::Pack).count() > 8 {
        query.pack_anims = true;
        query.pack_anim_filters.clear();
    }
    bake::run(&bake::Request {
        inputs: loaded.inputs.clone(),
        out_dir: scratch.to_path_buf(),
        pack_anims: query.pack_anims,
        pack_anim_filters: query.pack_anim_filters,
        anims: query.anims,
        preview_dir: scratch.to_path_buf(),
        preview_size: Some(CLIP_SIZE),
        preview_frame: Some(CLIP_FRAME.into()),
        ..bake::Request::default()
    })?;
    let mut tiles = Vec::with_capacity(clips.len());
    for clip in clips {
        let file = scratch.join(format!("preview_{}.png", sanitize_name(&clip.name)));
        if !file.is_file() {
            continue;
        }
        let image = image::open(&file)
            .with_context(|| format!("reading {}", file.display()))?
            .to_rgba8();
        tiles.push((clip.thumb.clone(), image));
    }
    Ok(tiles)
}

/// A per-run folder for the preview bake, removed by the caller; folders a killed run left behind go first.
pub fn scratch_dir(session: u64) -> PathBuf {
    let root = paths::cache_dir().join("tile-bake");
    let now = std::time::SystemTime::now();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            // A bake takes seconds, so anything this old has no live owner.
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age.as_secs() > 600);
            if stale {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let stamp = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    root.join(format!("{}-{session}-{stamp}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::settings::Settings;

    /// Renders every tile for the configured avatar; `AURA_TILE_DIR` keeps the PNGs for inspection.
    #[test]
    fn tiles_render_for_the_default_avatar() {
        let mut s = Settings::load();
        s.fill_defaults();
        let inputs = bake::Inputs {
            manifest: paths::expand(&s.manifest),
            pack: paths::expand(&s.pack),
            closet: paths::expand(&s.closet),
        };
        if inputs.check().is_err() {
            return;
        }
        let Ok((gpu, _)) = crate::gpu::headless() else {
            return;
        };
        let catalog = Catalog::load().unwrap();
        let loaded = session::load(&inputs, std::path::Path::new(""), &catalog, &mut |_| {}).unwrap();
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let faces = expression_tiles(&loaded, &gpu, &TextureCache::default(), &cancel, &mut |_| {})
            .unwrap()
            .unwrap();
        let faces_s = started.elapsed().as_secs_f64();
        let scratch = scratch_dir(0);
        let clips = clip_tiles(&loaded, &scratch).unwrap();
        let _ = std::fs::remove_dir_all(&scratch);
        println!(
            "{} expression tiles in {faces_s:.2}s, {} clip tiles, {:.2}s total",
            faces.len(),
            clips.len(),
            started.elapsed().as_secs_f64()
        );
        assert_eq!(faces.len(), loaded.expressions.len());
        assert_eq!(clips.len(), loaded.clips.len());
        assert!(faces
            .iter()
            .all(|(_, img)| img.width() == EXPRESSION_SIZE && img.pixels().any(|p| p.0[3] == 0)));
        if let Some(dir) = std::env::var_os("AURA_TILE_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, img) in faces.iter().chain(&clips) {
                img.save(dir.join(name)).unwrap();
            }
        }
    }
}
