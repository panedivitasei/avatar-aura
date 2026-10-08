// Strip tiles rendered from the loaded avatar after Load All, keyed like the bundled `assets/thumbs` files.
// Both sets render on the GPU viewer: expressions with build_assets.py's head framing, clips full body on the
// bake preview's green backdrop.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::anyhow;
use glam::Vec3;
use image::RgbaImage;

use crate::catalog;
use crate::session::{self, Loaded, TextureCache};
use crate::viewport::{self, Gpu};

pub const EXPRESSION_SIZE: u32 = 128;
pub const CLIP_SIZE: u32 = 192;
/// Clip tiles pose this fraction of the clip, as `--preview-frame 0.35` did for the bundled set.
const CLIP_FRAME: f64 = 0.35;
/// Clip tiles render at this multiple and box-filter down, which also thins the stage-coloured MSAA edge.
const SUPERSAMPLE: u32 = 3;
/// Vertical lens of the clip camera, narrow enough to read as the bundled orthographic set.
const CLIP_FOV_DEGREES: f32 = 3.0;

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
        apply_textures(&mut viewer, &mut overridden, &textures)?;
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

/// Poses each clip at 35% of its length with its face frame and renders a full-body front tile, reporting every
/// clip in order, skipped ones included; `None` when cancelled.
pub fn clip_tiles(
    loaded: &Loaded,
    gpu: &Gpu,
    cache: &TextureCache,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize),
) -> anyhow::Result<Option<Vec<Tile>>> {
    let size = CLIP_SIZE * SUPERSAMPLE;
    let blank = avatar_view::Viewer::new(&gpu.device, &gpu.queue, viewport::FORMAT, true);
    let stage = blank
        .render_to_rgba(&gpu.device, &gpu.queue, size, size)
        .map_err(|e| anyhow!("backdrop render: {e}"))?;
    let mut viewer = gpu.build_viewer(&loaded.scene, &loaded.dir)?;
    // The load framing put the viewer's contact blob 1 cm under the feet of this pose.
    let floor = viewer.bounds().map_or(0.0, |b| b.min.y - 0.01);

    // Scene clip, viewer clip and posed frame per clip; `None` for clips the session has not loaded.
    let poses: Vec<Option<ClipPose>> = (0..loaded.clips.len())
        .map(|i| {
            let scene = loaded.scene_index(i)?;
            let anim = &loaded.scene.animations[scene];
            let viewer_clip = viewer.clip_index(&anim.name)?;
            (anim.frame_count > 0).then(|| ClipPose {
                scene,
                viewer: viewer_clip,
                frame: preview_frame(anim.frame_count),
            })
        })
        .collect();
    let framing = Framing::fit(&mut viewer, &poses)?;
    framing.apply(viewer.camera_mut(), floor);
    let backdrop = framing.backdrop(size);

    let mut tiles = Vec::with_capacity(loaded.clips.len());
    let mut overridden = BTreeSet::new();
    for (i, (clip, pose)) in loaded.clips.iter().zip(&poses).enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if let Some(pose) = pose {
            viewer.set_clip_frame(pose.viewer, pose.frame as f32)?;
            let textures = session::face_frame_textures(loaded, pose.scene, pose.frame, cache)?;
            apply_textures(&mut viewer, &mut overridden, &textures)?;
            let frame = viewer
                .render_to_rgba(&gpu.device, &gpu.queue, size, size)
                .map_err(|e| anyhow!("{} tile: {e}", clip.name))?;
            let keyed = key_out(&frame, &stage);
            tiles.push((
                clip.thumb.clone(),
                downsample(&composite(&keyed, &backdrop), SUPERSAMPLE),
            ));
        }
        progress(i + 1);
    }
    Ok(Some(tiles))
}

#[derive(Clone, Copy)]
struct ClipPose {
    scene: usize,
    viewer: usize,
    frame: usize,
}

/// Swaps head materials to `textures`, restoring the ones the previous tile overrode and this one does not.
fn apply_textures(
    viewer: &mut avatar_view::Viewer,
    overridden: &mut BTreeSet<usize>,
    textures: &session::ExpressionTextures,
) -> anyhow::Result<()> {
    let current: BTreeSet<usize> = textures.iter().map(|(i, _)| *i).collect();
    for stale in overridden.difference(&current) {
        viewer.reset_material_texture(*stale)?;
    }
    for (index, image) in textures {
        viewer.set_material_texture(*index, image)?;
    }
    *overridden = current;
    Ok(())
}

/// The bake's `--preview-frame 0.35` pick: that fraction of the last frame, rounded.
fn preview_frame(frames: u32) -> usize {
    let last = frames.saturating_sub(1) as usize;
    ((CLIP_FRAME * last as f64).round() as usize).min(last)
}

/// One camera for every clip tile, as the bake's FrameCamera fit the rest pose and every posed frame.
struct Framing {
    center: Vec3,
    /// Edge of the square front-view box, with the bake's 8% margin on each side.
    span: f32,
    feet_y: f32,
    width: f32,
}

impl Framing {
    fn fit(viewer: &mut avatar_view::Viewer, poses: &[Option<ClipPose>]) -> anyhow::Result<Self> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let mut grow = |viewer: &avatar_view::Viewer| {
            if let Some(b) = viewer.bounds() {
                lo = lo.min(b.min);
                hi = hi.max(b.max);
            }
        };
        viewer.set_rest_pose();
        grow(viewer);
        for pose in poses.iter().flatten() {
            viewer.set_clip_frame(pose.viewer, pose.frame as f32)?;
            grow(viewer);
        }
        if lo.x > hi.x {
            return Err(anyhow!("the avatar has no geometry to frame"));
        }
        let mut span = (hi.x - lo.x).max(hi.y - lo.y) * (1.0 + 2.0 * 0.08);
        if span < 1e-3 {
            span = 1.0;
        }
        Ok(Self {
            center: (lo + hi) * 0.5,
            span,
            feet_y: lo.y,
            width: hi.x - lo.x,
        })
    }

    /// A narrow lens far down +Z stands in for the bake's orthographic projection.
    /// The eye sits level with `floor` so the viewer's contact blob is edge-on and draws nothing.
    fn apply(&self, camera: &mut avatar_view::Camera, floor: f32) {
        let distance = self.span * 0.5 / (CLIP_FOV_DEGREES * 0.5).to_radians().tan();
        let eye = Vec3::new(self.center.x, floor, self.center.z + distance);
        camera.look_from(eye, self.center);
        camera.fov_y_degrees = CLIP_FOV_DEGREES;
        camera.near = (distance - self.span * 2.0).max(0.01);
        camera.far = distance + self.span * 2.0;
    }

    /// The editor's green radial gradient with the bake's contact shadow under the feet, at the supersampled size.
    fn backdrop(&self, size: u32) -> RgbaImage {
        let inner = [146.0f32, 204.0, 112.0];
        let outer = [34.0f32, 78.0, 44.0];
        let s = size as f32;
        let (cx, cy) = (s * 0.5, s * 0.40);
        let rmax = (s * 0.5).hypot(s * 0.6);
        let scale = s / self.span;
        let feet = s * 0.5 - (self.feet_y - self.center.y) * scale;
        let sy = (feet + 2.0 * SUPERSAMPLE as f32).min(s - 2.0);
        let rx = (self.width * scale * 0.28).max(6.0 * SUPERSAMPLE as f32);
        let ry = (self.width * scale * 0.05).max(2.0 * SUPERSAMPLE as f32);
        RgbaImage::from_fn(size, size, |x, y| {
            let (x, y) = (x as f32, y as f32);
            let t = (x - cx).hypot(y - cy) / rmax;
            let t = (t * t * 0.9 + t * 0.1).min(1.0);
            let (dx, dy) = ((x - cx) / rx, (y - sy) / ry);
            let d = dx * dx + dy * dy;
            let shade = if d < 1.0 {
                1.0 - 0.55 * (1.0 - d).powf(1.5)
            } else {
                1.0
            };
            let c = |i: usize| ((inner[i] + (outer[i] - inner[i]) * t) * shade).round() as u8;
            image::Rgba([c(0), c(1), c(2), 255])
        })
    }
}

/// Lays a keyed render over an opaque backdrop of the same size.
fn composite(front: &RgbaImage, back: &RgbaImage) -> RgbaImage {
    let mut out = back.clone();
    for (o, f) in out.pixels_mut().zip(front.pixels()) {
        let a = f32::from(f.0[3]) / 255.0;
        for c in 0..3 {
            o.0[c] = (f32::from(f.0[c]) * a + f32::from(o.0[c]) * (1.0 - a)).round() as u8;
        }
    }
    out
}

/// Box filter by an integer factor, as the bake resolved its supersampling.
fn downsample(image: &RgbaImage, factor: u32) -> RgbaImage {
    let n = factor * factor;
    RgbaImage::from_fn(image.width() / factor, image.height() / factor, |x, y| {
        let mut acc = [0u32; 4];
        for dy in 0..factor {
            for dx in 0..factor {
                let p = image.get_pixel(x * factor + dx, y * factor + dy);
                for (a, v) in acc.iter_mut().zip(p.0) {
                    *a += u32::from(v);
                }
            }
        }
        image::Rgba(acc.map(|v| ((v + n / 2) / n) as u8))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::Catalog;
    use crate::settings::Settings;
    use crate::{bake, paths};

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
        let cache = TextureCache::default();
        let loaded = session::load_all(&loaded, &cancel, &cache, &mut |_, _, _| {})
            .unwrap()
            .loaded;
        let started = std::time::Instant::now();
        let faces = expression_tiles(&loaded, &gpu, &cache, &cancel, &mut |_| {})
            .unwrap()
            .unwrap();
        let faces_s = started.elapsed().as_secs_f64();
        let mut reported = 0;
        let clips = clip_tiles(&loaded, &gpu, &cache, &cancel, &mut |done| reported = done)
            .unwrap()
            .unwrap();
        assert_eq!(reported, loaded.clips.len());
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
        assert!(clips.iter().all(|(_, img)| img.width() == CLIP_SIZE));
        if let Some(dir) = std::env::var_os("AURA_TILE_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, img) in faces.iter().chain(&clips) {
                img.save(dir.join(name)).unwrap();
            }
        }
    }
}
