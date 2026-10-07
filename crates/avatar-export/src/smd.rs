// Port of avatar_aura/ae_convert.py `write_smd_reference` and `write_smd_animation`: Source engine SMD text.

use std::path::{Path, PathBuf};

use crate::avatar::{Avatar, Clip};
use crate::dae::{copy_file, create_dir, parent_dir, write_lines};
use crate::error::{bad_scene, Result};
use crate::math::{fmt, mat_rot3, mat_trans, r3_normalize_columns, r3_to_euler_xyz, safe_id, M4};
use crate::rig::{check_parents, clip_frame_locals, Rig};

fn smd_nodes(rig: &Rig, lines: &mut Vec<String>) {
    lines.push("nodes".into());
    for (i, b) in rig.bones.iter().enumerate() {
        lines.push(format!("{i} \"{}\" {}", b.name, b.parent));
    }
    lines.push("end".into());
}

fn smd_skeleton_frame(locals: &[M4], time: usize, lines: &mut Vec<String>) {
    lines.push(format!("time {time}"));
    for (i, m) in locals.iter().enumerate() {
        let t = mat_trans(m);
        let e = r3_to_euler_xyz(&r3_normalize_columns(&mat_rot3(m)));
        lines.push(format!(
            "{i} {} {} {} {} {} {}",
            fmt(t[0], 5),
            fmt(t[1], 5),
            fmt(t[2], 5),
            fmt(e[0], 6),
            fmt(e[1], 6),
            fmt(e[2], 6)
        ));
    }
}

/// Reference mesh plus bind skeleton; textures copied beside it when `copy_textures`.
pub fn write_smd_reference(
    avatar: &Avatar,
    rig: &Rig,
    out_path: &Path,
    copy_textures: bool,
) -> Result<PathBuf> {
    let out_dir = parent_dir(out_path);
    create_dir(&out_dir)?;
    let mut l = vec!["version 1".to_string()];
    smd_nodes(rig, &mut l);
    l.push("skeleton".into());
    let locals: Vec<M4> = rig.bones.iter().map(|b| b.local).collect();
    smd_skeleton_frame(&locals, 0, &mut l);
    l.push("end".into());
    l.push("triangles".into());
    for mesh in &avatar.meshes {
        let md = &avatar.materials[mesh.material];
        let mname = safe_id(&md.name);
        if copy_textures {
            let src = avatar.file(&md.diffuse);
            if src.is_file() {
                copy_file(&src, &out_dir.join(format!("{mname}_Diffuse.png")))?;
            }
        }
        let pos: Vec<_> = mesh.positions.iter().map(|p| rig.xform_point(p)).collect();
        let nrm: Vec<_> = mesh.normals.iter().map(|v| rig.xform_dir(v)).collect();
        for t in mesh.indices.as_chunks::<3>().0 {
            l.push(mname.clone());
            for &k in t {
                let k = k as usize;
                if k >= pos.len() {
                    return Err(bad_scene(format!("mesh {}: index out of range", mesh.name)));
                }
                let infl = rig.vertex_influences(&mesh.joints[k], &mesh.weights[k]);
                let (p, n) = (pos[k], nrm[k]);
                let [u, v] = mesh.uvs[k];
                let ws = infl
                    .iter()
                    .map(|(b, w)| format!("{b} {}", fmt(*w, 5)))
                    .collect::<Vec<_>>()
                    .join(" ");
                l.push(format!(
                    "{} {} {} {} {} {} {} {} {} {} {ws}",
                    infl[0].0,
                    fmt(p[0], 5),
                    fmt(p[1], 5),
                    fmt(p[2], 5),
                    fmt(n[0], 4),
                    fmt(n[1], 4),
                    fmt(n[2], 4),
                    fmt(u, 5),
                    fmt(1.0 - v, 5),
                    infl.len()
                ));
            }
        }
    }
    l.push("end".into());
    write_lines(out_path, &l)?;
    Ok(out_path.to_path_buf())
}

/// One clip as a skeleton-only SMD, one `time` block per frame.
pub fn write_smd_animation(avatar: &Avatar, rig: &Rig, anim: &Clip, out_path: &Path) -> Result<PathBuf> {
    check_parents(rig)?;
    create_dir(&parent_dir(out_path))?;
    let mut l = vec!["version 1".to_string()];
    smd_nodes(rig, &mut l);
    l.push("skeleton".into());
    for f in 0..anim.frame_count as usize {
        let locs = clip_frame_locals(avatar, rig, anim, f)?;
        smd_skeleton_frame(&locs, f, &mut l);
    }
    l.push("end".into());
    write_lines(out_path, &l)?;
    Ok(out_path.to_path_buf())
}
