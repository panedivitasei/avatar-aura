// Port of avatar_aura/preview.py, preview_cache.py, motion_cache.py and posed_export.py for the Export tab.
// Everything here runs on worker threads; the UI receives finished scenes, avatars and textures.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context};
use avatar_export::avatar::Avatar;
use avatar_export::export::{
    export_baked, export_directory, export_pose, ExportOptions, Formats, PoseSelection, PoseSource,
    PosedOutcome,
};
use avatar_export::face_animation::{self, FaceAnimation};
use avatar_export::faces::{self, CatalogExpression, Expression, Selection};
use avatar_export::math::safe_id;
use avatar_export::posing::PoseBone;
use avatar_export::scene::{Animation, Scene};
use image::RgbaImage;

use crate::bake::{self, ClipQuery, Inputs};
use crate::catalog::{self, Catalog, ClipEntry, ClipSource};
use crate::paths;

/// The clip the preview poses on load, as the Python app does.
pub const STAND_CLIP: &str = "Animation Generic Stand 1";

/// Cache folder keyed by the manifest bytes and the size and mtime of every input, like preview_cache.directory.
pub fn cache_directory(inputs: &Inputs, anim_dir: &Path) -> anyhow::Result<PathBuf> {
    let mut paths: Vec<PathBuf> = vec![
        inputs.manifest.clone(),
        inputs.pack.clone(),
        inputs.pack.with_file_name("AvatarAssetPackLegacyV1.toc"),
    ];
    if inputs.closet.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&inputs.closet) {
            paths.extend(rd.flatten().map(|e| e.path()).filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("bin") || e.eq_ignore_ascii_case("tsv"))
            }));
        }
    }
    paths.extend(catalog::anim_files(&paths::asset(&["clips"])));
    paths.extend(catalog::anim_files(anim_dir));
    paths.sort();
    paths.dedup();
    let stamps: Vec<serde_json::Value> = paths
        .iter()
        .map(|p| {
            let abs = std::path::absolute(p).unwrap_or_else(|_| p.clone());
            match std::fs::metadata(p) {
                Ok(m) => {
                    let mtime = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as u64);
                    serde_json::json!([abs.display().to_string(), m.len(), mtime])
                }
                Err(_) => serde_json::json!([abs.display().to_string(), null, null]),
            }
        })
        .collect();
    let mut data =
        std::fs::read(&inputs.manifest).with_context(|| format!("reading {}", inputs.manifest.display()))?;
    data.extend_from_slice(serde_json::to_string(&serde_json::json!(["preview-rs-v1", stamps]))?.as_bytes());
    let digest = faces::sha256_hex(&data);
    Ok(paths::cache_dir().join("preview-cache").join(&digest[..24]))
}

fn clip_cache_file(dir: &Path, name: &str) -> PathBuf {
    dir.join("clips").join(format!("{}.json", paths::safe_name(name)))
}

fn write_json_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pending = path.with_extension("pending.json");
    std::fs::write(&pending, text)?;
    std::fs::rename(&pending, path)?;
    Ok(())
}

/// A loaded avatar: the cache folder, the scene with every clip loaded so far, and the clip list.
#[derive(Clone)]
pub struct Loaded {
    pub dir: PathBuf,
    pub inputs: Inputs,
    pub scene: Arc<Scene>,
    pub avatar: Arc<Avatar>,
    pub clips: Vec<ClipEntry>,
    pub expressions: Vec<CatalogExpression>,
    pub materials: HashMap<String, usize>,
}

impl Loaded {
    pub fn is_loaded(&self, clip: usize) -> bool {
        self.clips
            .get(clip)
            .is_some_and(|c| self.scene.animations.iter().any(|a| a.name == c.name))
    }

    /// Index of a clip in the avatar's (and viewer's) animation list.
    pub fn scene_index(&self, clip: usize) -> Option<usize> {
        let name = &self.clips.get(clip)?.name;
        self.scene.animations.iter().position(|a| &a.name == name)
    }

    /// A copy with `animations` merged in by name, clip metadata refreshed, and the avatar rebuilt.
    pub fn with_animations(&self, animations: Vec<Animation>) -> anyhow::Result<Loaded> {
        let mut scene = (*self.scene).clone();
        for anim in animations {
            match scene.animations.iter_mut().find(|a| a.name == anim.name) {
                Some(slot) => *slot = anim,
                None => scene.animations.push(anim),
            }
        }
        let mut next = self.clone();
        refresh_clips(&mut next.clips, &scene);
        next.avatar = Arc::new(Avatar::from_scene(&scene, self.dir.clone())?);
        next.scene = Arc::new(scene);
        Ok(next)
    }
}

fn refresh_clips(clips: &mut Vec<ClipEntry>, scene: &Scene) {
    for anim in &scene.animations {
        match clips.iter_mut().find(|c| c.name == anim.name) {
            Some(c) => {
                c.fps = f64::from(anim.fps);
                c.frames = anim.frame_count;
            }
            None => clips.push(ClipEntry {
                name: anim.name.clone(),
                fps: f64::from(anim.fps),
                frames: anim.frame_count,
                thumb: catalog::clip_thumb(&anim.name),
                source: ClipSource::Pack,
            }),
        }
    }
}

/// Bakes the avatar (meshes, materials, face composites and the stand clip) into the cache unless it is there.
pub fn load(
    inputs: &Inputs,
    anim_dir: &Path,
    catalog: &Catalog,
    log: &mut dyn FnMut(String),
) -> anyhow::Result<Loaded> {
    inputs.check()?;
    let started = std::time::Instant::now();
    let dir = cache_directory(inputs, anim_dir)?;
    let json = dir.join("avatar.json");
    let cached = json.is_file();
    if cached {
        log(format!("Using cached bake {}", dir.display()));
    } else {
        log(format!("Baking avatar into {}", dir.display()));
        bake::run(&bake::Request {
            inputs: inputs.clone(),
            out_dir: dir.clone(),
            face_frames: true,
            pack_anim_filters: vec![STAND_CLIP.into()],
            ..bake::Request::default()
        })?;
    }
    let mut scene = bake::read_scene(&json)?;
    let mut clips = catalog::clip_list(catalog, anim_dir);
    for clip in &clips {
        let file = clip_cache_file(&dir, &clip.name);
        if scene.animations.iter().any(|a| a.name == clip.name) || !file.is_file() {
            continue;
        }
        let parsed = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Animation>(&t).ok());
        if let Some(anim) = parsed {
            scene.animations.push(anim);
        }
    }
    refresh_clips(&mut clips, &scene);
    clips.retain(|c| c.frames > 0 || matches!(c.source, ClipSource::File(_)));
    if let Ok(manifest) = std::fs::read(&inputs.manifest) {
        for guid in crate::import::manifest::closet_guids(&manifest) {
            if !scene
                .components
                .iter()
                .any(|c| c.guid.eq_ignore_ascii_case(&guid))
            {
                log(format!(
                    "closet item {guid} is worn but was not found in the closet"
                ));
            }
        }
    }
    let avatar = Avatar::from_scene(&scene, dir.clone())?;
    let expressions = faces::catalog(&avatar)?;
    let materials = scene
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| (safe_id(&m.name), i))
        .collect();
    log(format!(
        "{} avatar ready in {:.2}s: {} meshes, {} materials, {} of {} clips loaded",
        if cached { "Cached" } else { "Resolved" },
        started.elapsed().as_secs_f64(),
        scene.meshes.len(),
        scene.materials.len(),
        scene.animations.len(),
        clips.len()
    ));
    Ok(Loaded {
        dir,
        inputs: inputs.clone(),
        scene: Arc::new(scene),
        avatar: Arc::new(avatar),
        clips,
        expressions,
        materials,
    })
}

fn query_for(clips: &[&ClipEntry]) -> ClipQuery {
    let mut query = ClipQuery::default();
    for clip in clips {
        match &clip.source {
            ClipSource::Pack => query.pack_anim_filters.push(clip.name.clone()),
            ClipSource::File(path) => query.anims.push(path.clone()),
        }
    }
    query
}

fn keep_named(found: Vec<Animation>, wanted: &[&ClipEntry], dir: &Path) -> anyhow::Result<Vec<Animation>> {
    let mut out = Vec::new();
    for clip in wanted {
        let Some(anim) = found.iter().find(|a| a.name == clip.name) else {
            bail!("Animation is not available in this asset pack: {}", clip.name);
        };
        write_json_atomic(&clip_cache_file(dir, &clip.name), &serde_json::to_string(anim)?)?;
        out.push(anim.clone());
    }
    Ok(out)
}

/// Converts the given clips through the bake's animation stage and caches each as JSON.
pub fn load_clips(loaded: &Loaded, indices: &[usize]) -> anyhow::Result<Vec<Animation>> {
    let wanted: Vec<&ClipEntry> = indices.iter().filter_map(|i| loaded.clips.get(*i)).collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let found = bake::bake_clips(&loaded.inputs, &query_for(&wanted))?;
    keep_named(found, &wanted, &loaded.dir)
}

/// What Load All finished with; `done` falls short of `total` when cancelled.
pub struct LoadAllResult {
    pub loaded: Loaded,
    pub faces: Vec<PreparedFace>,
    pub done: usize,
    pub total: usize,
}

/// Load All: every missing clip in one pass, then each clip's face animation, reporting `(done, total, label)`.
pub fn load_all(
    loaded: &Loaded,
    cancel: &AtomicBool,
    textures: &TextureCache,
    progress: &mut dyn FnMut(usize, usize, String),
) -> anyhow::Result<LoadAllResult> {
    let total = loaded.clips.len() * 2;
    progress(0, total, "Preparing animation library...".into());
    let missing: Vec<usize> = (0..loaded.clips.len())
        .filter(|i| !loaded.is_loaded(*i))
        .collect();
    let wanted: Vec<&ClipEntry> = missing.iter().map(|i| &loaded.clips[*i]).collect();
    let mut next = loaded.clone();
    if !wanted.is_empty() {
        let mut query = query_for(&wanted);
        if wanted.iter().filter(|c| c.source == ClipSource::Pack).count() > 8 {
            query.pack_anims = true;
            query.pack_anim_filters.clear();
        }
        let found = bake::bake_clips(&loaded.inputs, &query)?;
        next = next.with_animations(keep_named(found, &wanted, &loaded.dir)?)?;
    }
    let mut done = 0;
    let mut faces = Vec::new();
    for index in 0..next.clips.len() {
        for face in [false, true] {
            if cancel.load(Ordering::Relaxed) {
                return Ok(LoadAllResult {
                    loaded: next,
                    faces,
                    done,
                    total,
                });
            }
            let label = format!(
                "{done} / {total} · {}{}",
                if face { "Face: " } else { "" },
                next.clips[index].name
            );
            progress(done, total, label);
            if face && next.is_loaded(index) {
                faces.push(prepare_face(&next, index, textures)?);
            }
            done += 1;
        }
    }
    Ok(LoadAllResult {
        loaded: next,
        faces,
        done,
        total,
    })
}

/// Decoded expression textures shared by the expression strips and the face player.
#[derive(Default)]
pub struct TextureCache {
    images: Mutex<HashMap<PathBuf, Arc<RgbaImage>>>,
}

impl TextureCache {
    pub fn get(&self, path: &Path) -> anyhow::Result<Arc<RgbaImage>> {
        if let Some(img) = self.images.lock().ok().and_then(|m| m.get(path).cloned()) {
            return Ok(img);
        }
        let img = Arc::new(
            image::open(path)
                .with_context(|| format!("loading {}", path.display()))?
                .to_rgba8(),
        );
        if let Ok(mut map) = self.images.lock() {
            if map.len() > 256 {
                map.clear();
            }
            map.insert(path.to_path_buf(), img.clone());
        }
        Ok(img)
    }
}

/// Material index and image per head material for one mixed expression.
pub type ExpressionTextures = Vec<(usize, Arc<RgbaImage>)>;

fn resolve_entry(
    loaded: &Loaded,
    textures: &std::collections::BTreeMap<String, PathBuf>,
    cache: &TextureCache,
) -> anyhow::Result<ExpressionTextures> {
    textures
        .iter()
        .map(|(material, path)| {
            let index = *loaded
                .materials
                .get(material)
                .ok_or_else(|| anyhow!("no material {material}"))?;
            Ok((index, cache.get(path)?))
        })
        .collect()
}

/// The expression a strip selection names: `None` channels keep the neutral composite.
pub fn selection(mouth: Option<u32>, eyes: Option<u32>, brows: Option<u32>) -> Expression {
    let mut s = Selection::new();
    s.insert("mouth".into(), mouth);
    s.insert("eyes".into(), eyes);
    s.insert("brows".into(), brows);
    Expression::Mix(s)
}

pub fn mix_expression(
    loaded: &Loaded,
    expression: &Expression,
    cache: &TextureCache,
) -> anyhow::Result<ExpressionTextures> {
    let entry = faces::mixed_catalog_entry(&loaded.avatar, expression)?;
    resolve_entry(loaded, &entry.textures, cache)
}

/// A face animation with every entry's textures decoded for playback.
pub struct PreparedFace {
    pub clip: usize,
    pub animation: FaceAnimation,
    pub entries: Vec<ExpressionTextures>,
}

pub fn prepare_face(loaded: &Loaded, clip: usize, cache: &TextureCache) -> anyhow::Result<PreparedFace> {
    let scene_index = loaded
        .scene_index(clip)
        .ok_or_else(|| anyhow!("Load the animation before its face animation."))?;
    let animation = face_animation::prepare(&loaded.avatar, scene_index)?;
    let entries = animation
        .entries
        .iter()
        .map(|e| resolve_entry(loaded, &e.textures, cache))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(PreparedFace {
        clip,
        animation,
        entries,
    })
}

/// Free-pose bones in the editor's serialized form from rig-local matrices.
pub fn pose_bones(names: &[String], locals: &[glam::Mat4]) -> Vec<PoseBone> {
    names
        .iter()
        .zip(locals)
        .map(|(name, m)| {
            let (_, r, t) = m.to_scale_rotation_translation();
            PoseBone {
                name: safe_id(name),
                position: [f64::from(t.x), f64::from(t.y), f64::from(t.z)],
                rotation: [f64::from(r.x), f64::from(r.y), f64::from(r.z), f64::from(r.w)],
            }
        })
        .collect()
}

pub fn today() -> String {
    chrono::Local::now().format("%m-%d-%Y").to_string()
}

/// The posed export of posed_export.py: a free pose or one clip frame with the chosen expression.
pub fn export_posed(
    loaded: &Loaded,
    pose: PoseSource,
    expression: Expression,
    formats: Formats,
    frame_label: String,
    destination: &Path,
) -> anyhow::Result<PosedOutcome> {
    let selection = PoseSelection {
        pose,
        expression,
        formats,
        frame_label,
    };
    Ok(export_pose(&loaded.avatar, &selection, destination, &today())?)
}

/// The rigged export of exporting.py: a fresh bake with the T-pose preview into `<export>/source`, then every format.
pub fn export_rigged(
    inputs: &Inputs,
    formats: Formats,
    destination: &Path,
    log: &mut dyn FnMut(String),
) -> anyhow::Result<(PathBuf, Vec<PathBuf>)> {
    if destination.as_os_str().is_empty() {
        bail!("Choose an output directory.");
    }
    let root = export_directory(destination, &today())?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Avatar".into());
    let opts = ExportOptions {
        manifest: inputs.manifest.clone(),
        pack: inputs.pack.clone(),
        closet: inputs.closet.clone(),
        out_dir: destination.to_path_buf(),
        name,
        formats,
        preview: true,
        ..ExportOptions::default()
    };
    let source = opts.source_dir()?;
    log("=== step 1/3: resolving + baking the avatar ===".into());
    bake::run(&bake::Request {
        inputs: inputs.clone(),
        out_dir: source.clone(),
        preview_dir: source,
        ..bake::Request::default()
    })?;
    let mut lines = Vec::new();
    let outcome = export_baked(&opts, &mut |line: &str| lines.push(line.to_string()), None)?;
    for line in lines {
        log(line);
    }
    Ok((outcome.root, outcome.written))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn default_inputs() -> Option<Inputs> {
        let mut s = Settings::default();
        s.fill_defaults();
        let inputs = Inputs {
            manifest: paths::expand(&s.manifest),
            pack: paths::expand(&s.pack),
            closet: paths::expand(&s.closet),
        };
        inputs.check().ok().map(|_| inputs)
    }

    #[test]
    fn export_paths_run_on_the_default_avatar() {
        let Some(inputs) = default_inputs() else {
            return;
        };
        let catalog = Catalog::load().unwrap();
        let loaded = load(&inputs, Path::new(""), &catalog, &mut |_| {}).unwrap();
        let cheer = loaded.clips.iter().position(|c| c.name == "Cheer").unwrap();
        let wave = loaded
            .clips
            .iter()
            .position(|c| c.name == "Animation Generic Wave")
            .unwrap();
        let anims = load_clips(&loaded, &[cheer, wave]).unwrap();
        assert_eq!(anims.len(), 2);
        let loaded = loaded.with_animations(anims).unwrap();
        assert!(loaded.is_loaded(cheer) && loaded.is_loaded(wave));

        let cache = TextureCache::default();
        let face = prepare_face(&loaded, wave, &cache).unwrap();
        assert_eq!(face.entries.len(), face.animation.entries.len());
        let mixed = mix_expression(&loaded, &selection(Some(3), Some(2), Some(1)), &cache).unwrap();
        assert!(!mixed.is_empty());

        let out = std::env::temp_dir().join(format!("aura-export-{}", std::process::id()));
        let formats = Formats {
            dae: true,
            glb: true,
            obj: true,
            smd: true,
        };
        let clip = loaded.scene_index(cheer).unwrap();
        let posed = export_posed(
            &loaded,
            PoseSource::Clip { clip, frame: 10 },
            selection(Some(3), None, None),
            formats.clone(),
            "10".into(),
            &out,
        )
        .unwrap();
        assert_eq!(posed.written.len(), 4);

        let viewer_names: Vec<String> = loaded.avatar.skeleton.iter().map(|j| j.name.clone()).collect();
        let rig = avatar_export::rig::build_source_rig(&loaded.avatar, avatar_export::rig::FRAME_XBOX, 1.0)
            .unwrap();
        let locals: Vec<glam::Mat4> = rig
            .bones
            .iter()
            .map(|b| avatar_view::pose::to_mat4(&b.local))
            .collect();
        let free = export_posed(
            &loaded,
            PoseSource::Free(pose_bones(&viewer_names, &locals)),
            selection(None, None, None),
            formats.clone(),
            "0".into(),
            &out,
        )
        .unwrap();
        assert_eq!(free.written.len(), 4);

        let mut lines = Vec::new();
        let (root, written) = export_rigged(&inputs, formats, &out, &mut |l| lines.push(l)).unwrap();
        assert!(root.join("preview.png").is_file(), "{lines:?}");
        assert!(written.len() >= 4);
        let _ = std::fs::remove_dir_all(&out);
    }
}
