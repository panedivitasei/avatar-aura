// Port of avatar_aura/exporting.py after the bake (`export` steps 2 and 3), `ae_convert.copy_face_assets`
// and avatar_aura/posed_export.py. Layout: <out>/<name>/{source,dae,glb,obj,smd,face}/..., preview.png.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::avatar::Avatar;
use crate::dae::{copy_file, create_dir, write_dae, DaeOptions};
use crate::error::{invalid, io_err, Error, Result};
use crate::faces::Expression;
use crate::glb::{write_glb, GlbOptions};
use crate::math::{safe_id, FRAME_SOURCE, FRAME_XBOX};
use crate::obj::write_obj;
use crate::posing::{bake_posed, from_local_pose, glb_pose_rig, rebind_posed, sample, PoseBone};
use crate::rig::build_source_rig;
use crate::scene::{FaceFile, FaceSlot};
use crate::smd::{write_smd_animation, write_smd_reference};

/// Source units per metre: inches.
pub const SMD_SCALE: f64 = 100.0 / 2.54;

/// Output formats; the bake inputs (manifest, pack, closet, sizes) drive `avatarextract --avatar`.
#[derive(Clone, Debug, PartialEq)]
pub struct Formats {
    pub dae: bool,
    pub glb: bool,
    pub obj: bool,
    pub smd: bool,
}

impl Default for Formats {
    fn default() -> Self {
        Self {
            dae: true,
            glb: true,
            obj: true,
            smd: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub manifest: PathBuf,
    pub pack: PathBuf,
    pub closet: PathBuf,
    pub anim_dir: PathBuf,
    pub out_dir: PathBuf,
    pub name: String,
    pub formats: Formats,
    pub animations: bool,
    pub pack_animations: bool,
    pub clip_files: bool,
    pub face_frames: bool,
    pub bake_size: u32,
    pub apply_scale: bool,
    pub carryable: bool,
    pub preview: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            manifest: PathBuf::new(),
            pack: PathBuf::new(),
            closet: PathBuf::new(),
            anim_dir: PathBuf::new(),
            out_dir: PathBuf::new(),
            name: "Avatar".into(),
            formats: Formats::default(),
            animations: false,
            pack_animations: false,
            clip_files: false,
            face_frames: false,
            bake_size: 1024,
            apply_scale: true,
            carryable: true,
            preview: true,
        }
    }
}

impl ExportOptions {
    /// Safe file stem for the export.
    pub fn stem(&self) -> String {
        safe_id(if self.name.is_empty() {
            "Avatar"
        } else {
            &self.name
        })
    }

    /// `<out_dir>/<stem>`, created.
    pub fn root(&self) -> Result<PathBuf> {
        if self.out_dir.as_os_str().is_empty() {
            return Err(Error::Export("pick an output folder".into()));
        }
        let root = self.out_dir.join(self.stem());
        create_dir(&root)?;
        Ok(root)
    }

    /// Bake target, `<root>/source`; the bake also writes `preview_T-Pose.png` there.
    pub fn source_dir(&self) -> Result<PathBuf> {
        Ok(self.root()?.join("source"))
    }
}

#[derive(Debug)]
pub struct ExportOutcome {
    pub root: PathBuf,
    pub written: Vec<PathBuf>,
    /// Empty when no preview was copied.
    pub preview_png: PathBuf,
    pub avatar: Avatar,
}

/// Writes every selected format from `<root>/source/avatar.json`, which the bake must have produced.
pub fn export_baked(
    opts: &ExportOptions,
    log: &mut dyn FnMut(&str),
    mut progress: Option<&mut dyn FnMut(u32)>,
) -> Result<ExportOutcome> {
    let name = opts.stem();
    let root = opts.root()?;
    let json_path = root.join("source").join("avatar.json");
    if !json_path.is_file() {
        return Err(Error::Export("bake produced no avatar.json".into()));
    }
    let mut written = Vec::new();
    log("");
    log("=== step 2/3: writing model files ===");
    let av = Avatar::load(&json_path)?;
    log(&format!(
        "  {} meshes, {} materials, {} clips, height {:.2} m",
        av.meshes.len(),
        av.materials.len(),
        av.animations.len(),
        av.height_m()
    ));
    let rig = build_source_rig(&av, FRAME_XBOX, 1.0)?;

    if opts.formats.dae {
        let d = root.join("dae");
        let p = write_dae(&av, &rig, &d.join(format!("{name}.dae")), &DaeOptions::default())?;
        log(&format!(
            "  .dae: {}  (textures beside it as <Material>_Diffuse.png)",
            p.display()
        ));
        written.push(p);
        if opts.clip_files && !av.animations.is_empty() {
            let ad = d.join("clips");
            for a in &av.animations {
                let dopts = DaeOptions {
                    animation: Some(a),
                    ..DaeOptions::default()
                };
                let file = ad.join(format!("{name}_{}.dae", safe_id(&a.name)));
                written.push(write_dae(&av, &rig, &file, &dopts)?);
            }
            log(&format!(
                "  {} animated .dae clips: {}",
                av.animations.len(),
                ad.display()
            ));
        }
    }
    if opts.formats.glb {
        let d = root.join("glb");
        create_dir(&d)?;
        let gopts = GlbOptions {
            include_animations: opts.animations,
            ..GlbOptions::default()
        };
        let p = write_glb(&av, &rig, &d.join(format!("{name}.glb")), &gopts)?;
        let clips = if opts.animations { av.animations.len() } else { 0 };
        let suffix = if clips > 0 {
            format!("  ({clips} clips)")
        } else {
            String::new()
        };
        log(&format!("  .glb: {}{suffix}", p.display()));
        written.push(p);
    }
    if opts.formats.obj {
        let p = write_obj(
            &av,
            &rig,
            &root.join("obj").join(format!("{name}.obj")),
            true,
            true,
        )?;
        log(&format!("  .obj: {}", p.display()));
        written.push(p);
    }
    if opts.formats.smd {
        let d = root.join("smd");
        let src_rig = build_source_rig(&av, FRAME_SOURCE, SMD_SCALE)?;
        written.push(write_smd_reference(
            &av,
            &src_rig,
            &d.join(format!("{name}_reference.smd")),
            true,
        )?);
        if opts.animations {
            for a in &av.animations {
                let file = d.join("anims").join(format!("{}.smd", safe_id(&a.name)));
                written.push(write_smd_animation(&av, &src_rig, a, &file)?);
            }
        }
        log(&format!("  .smd: {}", d.display()));
    }
    if opts.face_frames {
        let files = copy_face_assets(&av, &root)?;
        log(&format!(
            "  face frames: {} files under {}",
            files.len(),
            root.join("face").display()
        ));
        written.extend(files);
    }
    if let Some(p) = progress.as_mut() {
        p(85);
    }

    let mut preview_png = PathBuf::new();
    if opts.preview {
        let src = root.join("source").join("preview_T-Pose.png");
        if src.is_file() {
            preview_png = root.join("preview.png");
            copy_file(&src, &preview_png)?;
            written.push(preview_png.clone());
            log("");
            log("=== step 3/3: preview ===");
            log(&format!("  {}", preview_png.display()));
        }
    }
    if let Some(p) = progress.as_mut() {
        p(100);
    }
    Ok(ExportOutcome {
        root,
        written,
        preview_png,
        avatar: av,
    })
}

#[derive(Serialize)]
struct FaceIndexClip<'a> {
    name: &'a str,
    fps: f64,
    face: serde_json::Value,
}

#[derive(Serialize)]
struct FaceIndex<'a> {
    slots: &'a std::collections::BTreeMap<String, FaceSlot>,
    layer_names: &'a std::collections::BTreeMap<String, Vec<String>>,
    layers: &'a [FaceFile],
    composites: &'a [FaceFile],
    animations: Vec<FaceIndexClip<'a>>,
}

/// Copies the tinted face layers and frame composites under `out_dir` and writes `face/face_index.json`.
pub fn copy_face_assets(avatar: &Avatar, out_dir: &Path) -> Result<Vec<PathBuf>> {
    let face = &avatar.face;
    let mut written = Vec::new();
    for entry in face.layer_files.iter().chain(&face.composite_files) {
        let src = avatar.file(&entry.file);
        if !src.is_file() {
            continue;
        }
        let dst = out_dir.join(&entry.file);
        if let Some(parent) = dst.parent() {
            create_dir(parent)?;
        }
        copy_file(&src, &dst)?;
        written.push(dst);
    }
    let mut animations = Vec::with_capacity(avatar.animations.len());
    for a in &avatar.animations {
        let face = match &a.face {
            Some(f) => serde_json::to_value(f)?,
            None => serde_json::json!({}),
        };
        animations.push(FaceIndexClip {
            name: &a.name,
            fps: a.fps,
            face,
        });
    }
    let index = FaceIndex {
        slots: &face.slots,
        layer_names: &face.layer_names,
        layers: &face.layer_files,
        composites: &face.composite_files,
        animations,
    };
    let p = out_dir.join("face").join("face_index.json");
    create_dir(&out_dir.join("face"))?;
    let mut buf = Vec::new();
    let mut ser =
        serde_json::Serializer::with_formatter(&mut buf, serde_json::ser::PrettyFormatter::with_indent(b" "));
    index.serialize(&mut ser)?;
    std::fs::write(&p, buf).map_err(io_err(&p))?;
    written.push(p);
    Ok(written)
}

/// How a posed export picks its skeleton.
#[derive(Clone, Debug, PartialEq)]
pub enum PoseSource {
    /// Caller-supplied local transforms, every bone.
    Free(Vec<PoseBone>),
    /// One integer frame of clip `clip`.
    Clip { clip: usize, frame: i64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoseSelection {
    pub pose: PoseSource,
    pub expression: Expression,
    pub formats: Formats,
    /// Frame label for the log line of a free pose.
    pub frame_label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PosedOutcome {
    pub root: PathBuf,
    pub written: Vec<PathBuf>,
    pub log: Vec<String>,
}

/// Next free `<destination>/AvatarExport-<date>-<n>`; `date` is `%m-%d-%Y`.
pub fn export_directory(destination: &Path, date: &str) -> Result<PathBuf> {
    create_dir(destination)?;
    let prefix = format!("AvatarExport-{date}-");
    let mut number: u64 = 0;
    for entry in std::fs::read_dir(destination).map_err(io_err(destination))? {
        let entry = entry.map_err(io_err(destination))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix(&prefix) {
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(n) = rest.parse::<u64>() {
                    number = number.max(n);
                }
            }
        }
    }
    number += 1;
    loop {
        let root = destination.join(format!("{prefix}{number}"));
        match std::fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => number += 1,
            Err(e) => return Err(io_err(&root)(e)),
        }
    }
}

/// Writes the posed avatar through the unchanged writers into a fresh export directory.
pub fn export_pose(
    avatar: &Avatar,
    selection: &PoseSelection,
    destination: &Path,
    date: &str,
) -> Result<PosedOutcome> {
    if destination.as_os_str().is_empty() {
        return Err(invalid("Choose an output directory."));
    }
    let f = &selection.formats;
    if !(f.dae || f.glb || f.obj || f.smd) {
        return Err(invalid("Choose at least one supported format."));
    }
    let (rig, pose_name) = match &selection.pose {
        PoseSource::Free(bones) => (
            from_local_pose(avatar, bones)?,
            format!("Visible pose at frame {}", selection.frame_label),
        ),
        PoseSource::Clip { clip, frame } => {
            let c = avatar
                .animations
                .get(*clip)
                .ok_or_else(|| invalid("Choose an animation before exporting."))?;
            (sample(avatar, c, *frame)?, format!("{}_{frame}", c.name))
        }
    };
    let posed = bake_posed(avatar, &rig, &selection.expression)?;
    let root = export_directory(destination, date)?;
    let stem = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut written = Vec::new();
    if f.dae {
        let p = root.join("dae").join(format!("{stem}.dae"));
        written.push(write_dae(&posed, &rig, &p, &DaeOptions::default())?);
    }
    if f.glb {
        let glb_rig = glb_pose_rig(avatar, &rig)?;
        let skinned = rebind_posed(&posed, &rig, &glb_rig)?;
        let gopts = GlbOptions {
            include_animations: false,
            ..GlbOptions::default()
        };
        let p = root.join("glb").join(format!("{stem}.glb"));
        written.push(write_glb(&skinned, &glb_rig, &p, &gopts)?);
    }
    if f.obj {
        let p = root.join("obj").join(format!("{stem}.obj"));
        written.push(write_obj(&posed, &rig, &p, true, true)?);
    }
    if f.smd {
        let source_rig = build_source_rig(&posed, FRAME_SOURCE, SMD_SCALE)?;
        let p = root.join("smd").join(format!("{stem}.smd"));
        written.push(write_smd_reference(&posed, &source_rig, &p, true)?);
    }
    let expression = match &selection.expression {
        Expression::Named(n) => n.clone(),
        Expression::Mix(m) => format!("{m:?}"),
    };
    let mut log = vec![format!("Exported {pose_name}, {expression}")];
    log.extend(written.iter().map(|p| format!("wrote {}", p.display())));
    Ok(PosedOutcome { root, written, log })
}
