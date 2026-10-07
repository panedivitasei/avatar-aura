//! Ports the animations region of avatar_export.cpp: AnimExport, LoadAnimationFile, the --anim/--anim-dir/--pack-anims
//! collection with --pack-anim filters, and WriteAnimation's track conversion into `scene::Animation`.

use std::path::Path;
use std::rc::Rc;

use avatar_export::scene;
use avatar_formats::animation::load_option;
use avatar_formats::Animation;
use glam::Vec3;

use crate::args::Args;
use crate::manifest::category;
use crate::resolve::{Assets, Resolved};
use crate::skin::JointXform;
use crate::util::{lower, sanitize_name};

#[derive(Clone, Debug)]
pub struct AnimExport {
    pub name: String,
    pub anim: Rc<Animation>,
    /// Per-clip override of --preview-frame; empty means the global rule.
    pub preview_frame: String,
}

/// LoadAnimationFile: a whole .AvatarAnimation STRB file.
pub fn load_animation_file(path: &Path) -> Option<Animation> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 4 {
        return None;
    }
    Animation::load(&bytes, load_option::ELEMENTS).ok().flatten()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every clip the export carries, in the C++ order: --anim files, --anim-dir, pack clips, the carryable's own.
pub fn collect(args: &Args, assets: &Assets, res: &Resolved) -> Vec<AnimExport> {
    let mut anims = Vec::new();
    for p in &args.anims {
        match load_animation_file(p) {
            Some(a) => anims.push(AnimExport {
                name: stem(p),
                anim: Rc::new(a),
                preview_frame: String::new(),
            }),
            None => println!("  WARN: animation {} failed to load", p.display()),
        }
    }
    if !args.anim_dir.as_os_str().is_empty() && args.anim_dir.is_dir() {
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&args.anim_dir) {
            for e in rd.flatten() {
                let path = e.path();
                let ext = path
                    .extension()
                    .map(|x| format!(".{}", x.to_string_lossy()).to_ascii_lowercase())
                    .unwrap_or_default();
                if ext == ".avataranimation" || ext == ".anim" || ext == ".bin" {
                    files.push(path);
                }
            }
        }
        files.sort();
        for p in &files {
            match load_animation_file(p) {
                Some(a) => anims.push(AnimExport {
                    name: stem(p),
                    anim: Rc::new(a),
                    preview_frame: String::new(),
                }),
                None => println!("  WARN: animation {} failed to load", p.display()),
            }
        }
    }
    if args.pack_anims || !args.pack_anim_filters.is_empty() {
        // "<name>@<rule>": the rule (mid|peak|0..1|N) picks that clip's preview frame.
        let filters: Vec<(String, String)> = args
            .pack_anim_filters
            .iter()
            .map(|f| match f.find('@') {
                None => (lower(f), String::new()),
                Some(at) => (lower(&f[..at]), f[at + 1..].to_owned()),
            })
            .collect();
        for (i, info) in assets.pack.asset_infos().iter().enumerate() {
            if info.categories & category::ANIMATION == 0 {
                continue;
            }
            let mut name = assets.pack.asset_name_by_index(i);
            if name.is_empty() {
                name = format!("pack_anim_{i}");
            }
            let mut frame_rule = String::new();
            if !args.pack_anims {
                let lname = lower(&name);
                let Some((_, rule)) = filters.iter().find(|(f, _)| lname.contains(f.as_str())) else {
                    continue;
                };
                frame_rule = rule.clone();
            }
            let Some(data) = assets.pack.asset_data_by_index(i) else {
                continue;
            };
            let Some(a) = Animation::load(data, load_option::ELEMENTS).ok().flatten() else {
                continue;
            };
            anims.push(AnimExport {
                name,
                anim: Rc::new(a),
                preview_frame: frame_rule,
            });
        }
    }
    if let Some(anim) = res.prop.as_ref().and_then(|p| p.animation.clone()) {
        anims.push(AnimExport {
            name: format!("Carryable_{}", sanitize_name(&res.prop_name)),
            anim,
            preview_frame: String::new(),
        });
    }
    anims
}

fn v3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn q4(q: glam::Quat) -> [f32; 4] {
    [q.x, q.y, q.z, q.w]
}

/// WriteAnimation as typed data. Joint translations are deltas from the rest offset in the clips, so they are
/// made absolute and scaled by the parent's rest world scale to drive the scaled skeleton.
pub fn to_scene(ae: &AnimExport, xf: &[JointXform]) -> scene::Animation {
    let a = &*ae.anim;
    let duration = if a.frames_per_second > 0.0 {
        if a.frame_count > 0 {
            (a.frame_count - 1) as f32 / a.frames_per_second
        } else {
            0.0
        }
    } else {
        0.0
    };
    let mut out = scene::Animation {
        name: ae.name.clone(),
        fps: a.frames_per_second,
        frame_count: a.frame_count,
        joint_count: a.pose_counts[0],
        carryable_joint_count: a.pose_counts[1],
        motion_count: a.motion_count,
        texture_count: a.texture_count,
        duration,
        translation_mode: "absolute_local".into(),
        ..Default::default()
    };

    let frames = &a.pose_frame_sets[0].frames;
    for ji in 0..a.pose_counts[0] as usize {
        let x = xf.get(ji).copied().unwrap_or_default();
        let (pscale, jscale, rest) = if ji < xf.len() {
            (x.parent_world_scale, x.scale, x.rest_local_t)
        } else {
            (Vec3::ONE, Vec3::ONE, Vec3::ZERO)
        };
        let mut track = scene::Track {
            joint: ji as u32,
            ..Default::default()
        };
        for frame in frames.iter().take_while(|f| ji < f.len()) {
            let e = &frame[ji];
            track.t.push(v3((rest + e.position) * pscale));
            track.r.push(q4(e.rotation));
            track.s.push(v3(e.scale * jscale));
        }
        out.tracks.push(track);
    }

    let frames = &a.pose_frame_sets[1].frames;
    for ji in 0..a.pose_counts[1] as usize {
        let mut track = scene::Track {
            joint: ji as u32,
            ..Default::default()
        };
        for frame in frames.iter().take_while(|f| ji < f.len()) {
            let e = &frame[ji];
            track.t.push(v3(e.position));
            track.r.push(q4(e.rotation));
            track.s.push(v3(e.scale));
        }
        out.carryable_tracks.push(track);
    }

    let frames = &a.motion_frame_set.frames;
    for mi in 0..a.motion_count as usize {
        let mut track = scene::MotionTrack::default();
        for frame in frames.iter().take_while(|f| mi < f.len()) {
            track.t.push(v3(frame[mi].position));
            track.r.push(q4(frame[mi].rotation));
        }
        out.motion.push(track);
    }

    // Face channels in XAVATAR_ANIMATED_TEXTURE order; the writer emits the first `texture_count` of them.
    let frames = &a.texture_frame_set.frames;
    let channel = |ci: usize| -> Vec<u32> {
        if ci >= a.texture_count as usize {
            return Vec::new();
        }
        frames
            .iter()
            .take_while(|f| ci < f.len())
            .map(|f| f[ci].layer_index)
            .collect()
    };
    out.face = Some(scene::FaceTrack {
        mouth: channel(0),
        brow_left: channel(1),
        brow_right: channel(2),
        eye_left: channel(3),
        eye_right: channel(4),
    });
    out
}
