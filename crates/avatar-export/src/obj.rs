// Port of avatar_aura/ae_convert.py `write_obj`: static Wavefront OBJ plus its MTL, textures copied beside them.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::avatar::Avatar;
use crate::dae::{copy_file, create_dir, parent_dir, sorted_used_materials};
use crate::error::{io_err, Result};
use crate::math::{fmt, safe_id};
use crate::rig::Rig;

/// Writes `out_path` and `<stem>.mtl` beside it; returns the OBJ path.
pub fn write_obj(
    avatar: &Avatar,
    rig: &Rig,
    out_path: &Path,
    copy_textures: bool,
    flip_v: bool,
) -> Result<PathBuf> {
    let out_dir = parent_dir(out_path);
    create_dir(&out_dir)?;
    let stem = out_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mtl_name = format!("{stem}.mtl");
    let mut mtl = String::new();
    for mi in sorted_used_materials(avatar) {
        let md = &avatar.materials[mi];
        let name = safe_id(&md.name);
        let dst = format!("{name}_Diffuse.png");
        let src = avatar.file(&md.diffuse);
        if copy_textures && src.is_file() {
            copy_file(&src, &out_dir.join(&dst))?;
        }
        let _ = write!(
            mtl,
            "newmtl {name}\nKa 1 1 1\nKd 1 1 1\nKs 0 0 0\nd 1\nillum 1\nmap_Kd {dst}\n"
        );
        if md.has_alpha {
            let _ = writeln!(mtl, "map_d {dst}");
        }
        mtl.push('\n');
    }
    let mtl_path = out_dir.join(&mtl_name);
    std::fs::write(&mtl_path, mtl).map_err(io_err(&mtl_path))?;

    let mut f = String::new();
    f.push_str("# ReXGlue Avatar Export\n");
    let _ = write!(f, "mtllib {mtl_name}\n\n");
    let mut base: usize = 1;
    for mesh in &avatar.meshes {
        let _ = writeln!(f, "o {}", safe_id(&mesh.name));
        for p in &mesh.positions {
            let q = rig.xform_point(p);
            let _ = writeln!(f, "v {} {} {}", fmt(q[0], 5), fmt(q[1], 5), fmt(q[2], 5));
        }
        for &[u, v] in &mesh.uvs {
            let v = if flip_v { 1.0 - v } else { v };
            let _ = writeln!(f, "vt {} {}", fmt(u, 5), fmt(v, 5));
        }
        for n in &mesh.normals {
            let q = rig.xform_dir(n);
            let _ = writeln!(f, "vn {} {} {}", fmt(q[0], 4), fmt(q[1], 4), fmt(q[2], 4));
        }
        let _ = writeln!(f, "usemtl {}", safe_id(&avatar.materials[mesh.material].name));
        for t in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|i| i as usize + base);
            let _ = writeln!(f, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
        base += mesh.positions.len();
        f.push('\n');
    }
    std::fs::write(out_path, f).map_err(io_err(out_path))?;
    Ok(out_path.to_path_buf())
}
