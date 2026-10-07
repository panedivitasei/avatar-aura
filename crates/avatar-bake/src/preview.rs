//! Ports the preview renderer of avatar_export.cpp: PoseSkinMatrices, PoseMeshes, FrameCamera, PreviewShade,
//! RenderPreviewImage, PickPreviewFrame, the --gen-icons framing helpers and the preview stage of Run.
//! Orthographic front view, z-buffer, three-light shading, 2x supersampling on the editor's green backdrop.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use avatar_formats::{Animation, Skeleton};
use glam::{Vec2, Vec3, Vec4};

use crate::animations::AnimExport;
use crate::args::Args;
use crate::glmath::{
    dot3, fmax, fmax3, fmin, fmin3, length3, max3, min3, normalize3, radians, Mat3, Mat4, Quat,
};
use crate::manifest::category;
use crate::resolve::Resolved;
use crate::skin::{blend_skin, has_parent, BakedMesh, JointXform};
use crate::textures::{bake_material, write_png, BakedMaterial, HeadBatchInfo, Image, Sampler};
use crate::util::sanitize_name;

#[derive(Clone, Debug, Default)]
pub struct PosedMesh {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
}

/// Skin matrices for one frame of a clip (or the rest pose), local TRS re-composed through the scaled skeleton.
pub fn pose_skin_matrices(
    xf: &[JointXform],
    skel: &Skeleton,
    anim: Option<&Animation>,
    frame: usize,
    pose_set: usize,
) -> Vec<Mat4> {
    let mut world = vec![Mat4::IDENTITY; xf.len()];
    let mut skin = vec![Mat4::IDENTITY; xf.len()];
    let frames = anim.map(|a| &a.pose_frame_sets[pose_set.min(1)].frames);
    for j in 0..xf.len() {
        let mut t = xf[j].rest_local_t;
        let mut r = xf[j].rest_local_r;
        let mut sc = xf[j].scale;
        if let Some(e) = frames.and_then(|f| f.get(frame)).and_then(|f| f.get(j)) {
            t += e.position;
            r = Quat::from_glam(e.rotation).normalize();
            sc *= e.scale;
        }
        let local = Mat4::IDENTITY
            .translate(t)
            .mul(&r.to_mat4())
            .mul(&Mat4::IDENTITY.scale(sc));
        let parent = skel.joints.get(j).map_or(255, |jt| jt.parent_index);
        world[j] = if has_parent(parent, j) {
            world[usize::from(parent)].mul(&local)
        } else {
            local
        };
        skin[j] = world[j].mul(&xf[j].bind_world.inverse());
    }
    skin
}

/// Skins every mesh with the given matrices; carryables keep their rest pose.
pub fn pose_meshes(meshes: &[BakedMesh], skin: &[Mat4]) -> Vec<PosedMesh> {
    meshes
        .iter()
        .map(|m| {
            let mut out = PosedMesh {
                pos: Vec::with_capacity(m.verts.len()),
                nrm: Vec::with_capacity(m.verts.len()),
            };
            for v in &m.verts {
                if m.is_prop {
                    out.pos.push(v.position);
                    out.nrm.push(v.normal);
                    continue;
                }
                match blend_skin(v, v.orig_position, v.orig_normal, skin) {
                    Some((p, n)) => {
                        out.pos.push(p);
                        let len = length3(n);
                        out.nrm.push(if len > 1e-6 { n / len } else { v.normal });
                    }
                    None => {
                        out.pos.push(v.orig_position);
                        out.nrm.push(v.orig_normal);
                    }
                }
            }
            out
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreviewCamera {
    /// World x/y at the image centre.
    pub center: Vec2,
    /// Pixels per metre at the supersampled size.
    pub scale: f32,
    /// Lowest world y, for the contact shadow.
    pub feet_y: f32,
    /// World width of the avatar.
    pub width: f32,
}

/// FrameCamera: one camera that fits every pose, so switching poses does not re-frame.
pub fn frame_camera(all_poses: &[Vec<PosedMesh>], px: i32) -> PreviewCamera {
    let mut lo = Vec2::splat(1e9);
    let mut hi = Vec2::splat(-1e9);
    for poses in all_poses {
        for pm in poses {
            for p in &pm.pos {
                lo = Vec2::new(fmin(lo.x, p.x), fmin(lo.y, p.y));
                hi = Vec2::new(fmax(hi.x, p.x), fmax(hi.y, p.y));
            }
        }
    }
    let margin = 0.08f32;
    let mut span = fmax(hi.x - lo.x, hi.y - lo.y) * (1.0 + 2.0 * margin);
    if span < 1e-3 {
        span = 1.0;
    }
    PreviewCamera {
        center: (lo + hi) * 0.5,
        scale: px as f32 / span,
        feet_y: lo.y,
        width: hi.x - lo.x,
    }
}

/// PreviewShade: half-lambert key, fill and rim.
pub fn preview_shade(n: Vec3) -> f32 {
    let key = normalize3(Vec3::new(-0.45, 0.60, 0.70));
    let fill = normalize3(Vec3::new(0.70, 0.15, 0.55));
    let rim = normalize3(Vec3::new(0.30, 0.50, -0.80));
    let k = fmax(0.0, dot3(n, key)) * 0.5 + 0.5;
    let f = fmax(0.0, dot3(n, fill));
    let r = fmax(0.0, dot3(n, rim));
    fmin(1.25, 0.22 + 0.72 * k * k + 0.20 * f + 0.18 * r * r)
}

fn vertex_rgb(c: u32) -> Vec3 {
    Vec3::new(
        ((c >> 16) & 0xFF) as f32,
        ((c >> 8) & 0xFF) as f32,
        (c & 0xFF) as f32,
    ) / 255.0
}

/// RenderPreviewImage. `overrides` swaps a material's diffuse (per-clip face composites); `covered_px` reports
/// the drawn sample count at the supersampled size.
#[allow(clippy::too_many_arguments)]
pub fn render_preview_image(
    meshes: &[BakedMesh],
    materials: &[BakedMaterial],
    posed: &[PosedMesh],
    cam: &PreviewCamera,
    out_size: i32,
    out_png: &Path,
    overrides: &HashMap<usize, Rc<Image>>,
    transparent: bool,
    covered_px: Option<&mut usize>,
) -> bool {
    const SS: i32 = 2;
    let w = out_size.max(0) * SS;
    let h = out_size.max(0) * SS;
    let (wu, hu) = (w as usize, h as usize);
    let mut rgb = vec![0.0f32; wu * hu * 3];
    let mut cover = vec![u8::from(!transparent); wu * hu];
    let mut zbuf = vec![-1e30f32; wu * hu];
    // Backdrop: the editor's green radial gradient.
    let inner = Vec3::new(146.0, 204.0, 112.0);
    let outer = Vec3::new(34.0, 78.0, 44.0);
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.40;
    let rmax = (w as f32 * 0.5).hypot(h as f32 * 0.6);
    if !transparent {
        for y in 0..h {
            for x in 0..w {
                let mut t = (x as f32 - cx).hypot(y as f32 - cy) / rmax;
                t = fmin(1.0, t * t * 0.9 + t * 0.1);
                let c = inner + (outer - inner) * t;
                let i = (y as usize * wu + x as usize) * 3;
                rgb[i..i + 3].copy_from_slice(&[c.x, c.y, c.z]);
            }
        }
    }
    let sx_of = |wx: f32| (wx - cam.center.x) * cam.scale + w as f32 * 0.5;
    let sy_of = |wy: f32| h as f32 * 0.5 - (wy - cam.center.y) * cam.scale;
    // Contact shadow.
    if !transparent {
        let scx = sx_of(cam.center.x);
        let scy = fmin(h as f32 - 2.0, sy_of(cam.feet_y) + 2.0 * SS as f32);
        let rx = fmax(12.0, cam.width * cam.scale * 0.28);
        let ry = fmax(4.0, cam.width * cam.scale * 0.05);
        let y_lo = 0.max((scy - ry) as i32);
        let y_hi = (h - 1).min((scy + ry) as i32);
        let x_lo = 0.max((scx - rx) as i32);
        let x_hi = (w - 1).min((scx + rx) as i32);
        for y in y_lo..=y_hi {
            for x in x_lo..=x_hi {
                let dx = (x as f32 - scx) / rx;
                let dy = (y as f32 - scy) / ry;
                let d = dx * dx + dy * dy;
                if d >= 1.0 {
                    continue;
                }
                let a = 0.55 * (1.0 - d).powf(1.5);
                let i = (y as usize * wu + x as usize) * 3;
                for c in &mut rgb[i..i + 3] {
                    *c *= 1.0 - a;
                }
            }
        }
    }
    // Triangles.
    for (mi, m) in meshes.iter().enumerate() {
        let (Some(mat), Some(pm)) = (
            usize::try_from(m.material).ok().and_then(|i| materials.get(i)),
            posed.get(mi),
        ) else {
            continue;
        };
        let img = match overrides.get(&(m.material as usize)) {
            Some(o) => Some(o.clone()),
            None => mat.image.ok.then(|| mat.image.clone()),
        };
        let tex = Sampler {
            img,
            flags: 3,
            ..Default::default()
        };
        let mask = mat.has_alpha;
        let p = &pm.pos;
        let shade: Vec<f32> = pm.nrm.iter().map(|&n| preview_shade(n)).collect();
        let uv_layer = mat.uv_layer.min(5);
        let mut t = 0;
        while t + 2 < m.indices.len() {
            let tri = [
                usize::from(m.indices[t]),
                usize::from(m.indices[t + 1]),
                usize::from(m.indices[t + 2]),
            ];
            t += 3;
            if tri
                .iter()
                .any(|&i| i >= p.len() || i >= m.verts.len() || i >= shade.len())
            {
                continue;
            }
            let [ia, ib, ic] = tri;
            let a = Vec2::new(sx_of(p[ia].x), sy_of(p[ia].y));
            let b = Vec2::new(sx_of(p[ib].x), sy_of(p[ib].y));
            let c = Vec2::new(sx_of(p[ic].x), sy_of(p[ic].y));
            let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = 0.max(fmin3(a.x, b.x, c.x).floor() as i32);
            let x1 = (w - 1).min(fmax3(a.x, b.x, c.x).ceil() as i32);
            let y0 = 0.max(fmin3(a.y, b.y, c.y).floor() as i32);
            let y1 = (h - 1).min(fmax3(a.y, b.y, c.y).ceil() as i32);
            let inv = 1.0 / area;
            let vc = [
                vertex_rgb(m.verts[ia].color),
                vertex_rgb(m.verts[ib].color),
                vertex_rgb(m.verts[ic].color),
            ];
            let uva = m.verts[ia].uvs[uv_layer];
            let uvb = m.verts[ib].uvs[uv_layer];
            let uvc = m.verts[ic].uvs[uv_layer];
            for y in y0..=y1 {
                let py = y as f32 + 0.5;
                for x in x0..=x1 {
                    let px = x as f32 + 0.5;
                    let w0 = ((b.x - px) * (c.y - py) - (c.x - px) * (b.y - py)) * inv;
                    let w1 = ((c.x - px) * (a.y - py) - (a.x - px) * (c.y - py)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let z = w0 * p[ia].z + w1 * p[ib].z + w2 * p[ic].z;
                    let pi = y as usize * wu + x as usize;
                    if z <= zbuf[pi] {
                        continue;
                    }
                    let mut texel = Vec4::new(0.8, 0.8, 0.8, 1.0);
                    if tex.img.is_some() {
                        texel = tex.sample(uva * w0 + uvb * w1 + uvc * w2);
                        if mask && texel.w < 0.5 {
                            continue;
                        }
                    }
                    zbuf[pi] = z;
                    cover[pi] = 1;
                    let sh = w0 * shade[ia] + w1 * shade[ib] + w2 * shade[ic];
                    let col = vc[0] * w0 + vc[1] * w1 + vc[2] * w2;
                    let o = &mut rgb[pi * 3..pi * 3 + 3];
                    o[0] = fmin(255.0, texel.x * col.x * sh * 255.0);
                    o[1] = fmin(255.0, texel.y * col.y * sh * 255.0);
                    o[2] = fmin(255.0, texel.z * col.z * sh * 255.0);
                }
            }
        }
    }
    // Downsample.
    let os = out_size.max(0) as usize;
    let mut out = Image {
        width: os as u32,
        height: os as u32,
        rgba: vec![0; os * os * 4],
        ok: true,
    };
    for y in 0..os {
        for x in 0..os {
            let mut acc = Vec3::ZERO;
            let mut n = 0i32;
            for dy in 0..SS as usize {
                for dx in 0..SS as usize {
                    let i = (y * SS as usize + dy) * wu + (x * SS as usize + dx);
                    if cover[i] == 0 {
                        continue;
                    }
                    acc += Vec3::new(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]);
                    n += 1;
                }
            }
            if n != 0 {
                acc /= n as f32;
            }
            let o = (y * os + x) * 4;
            out.rgba[o] = acc.x.round() as u8;
            out.rgba[o + 1] = acc.y.round() as u8;
            out.rgba[o + 2] = acc.z.round() as u8;
            out.rgba[o + 3] = if transparent {
                (255 * n / (SS * SS)) as u8
            } else {
                255
            };
        }
    }
    if let Some(c) = covered_px {
        *c = cover.iter().filter(|&&c| c != 0).count();
    }
    write_png(out_png, &out)
}

/// PickPreviewFrame: `mid` (50%), `peak` (largest total joint rotation), a fraction 0..1, or a frame number.
pub fn pick_preview_frame(a: &Animation, rule: &str) -> usize {
    let n = a.frame_count as usize;
    if n == 0 {
        return 0;
    }
    if rule == "mid" {
        return n / 2;
    }
    if rule == "peak" {
        let mut best = 0;
        let mut best_score = -1.0f64;
        for (f, frame) in a.pose_frame_sets[0].frames.iter().enumerate() {
            let mut score = 0.0f64;
            for e in frame {
                score += 2.0 * f64::from(fmin(1.0, e.rotation.w.abs()).acos());
            }
            if score > best_score {
                best_score = score;
                best = f;
            }
        }
        return best;
    }
    if let Some(v) = crate::util::strtod_full(rule) {
        if (0.0..=1.0).contains(&v) {
            return (n - 1).min((v * (n - 1) as f64).round() as usize);
        }
        if v > 1.0 {
            return (n - 1).min(v as usize);
        }
    }
    n / 2
}

fn view_rotation(yaw_deg: f32, pitch_deg: f32) -> Mat3 {
    Mat4::IDENTITY
        .rotate(radians(pitch_deg), Vec3::X)
        .mul(&Mat4::IDENTITY.rotate(radians(yaw_deg), Vec3::Y))
        .to_mat3()
}

/// RotatePosed: a 3/4 view as a rotation of the posed vertices about the framed box's centre.
pub fn rotate_posed(posed: &mut [PosedMesh], pivot: Vec3, yaw_deg: f32, pitch_deg: f32) {
    let r = view_rotation(yaw_deg, pitch_deg);
    for pm in posed.iter_mut() {
        for p in pm.pos.iter_mut() {
            *p = r.mul_vec3(*p - pivot) + pivot;
        }
        for n in pm.nrm.iter_mut() {
            let rn = r.mul_vec3(*n);
            let len = length3(rn);
            if len > 1e-8 {
                *n = rn / len;
            }
        }
    }
}

pub fn rotate_box(lo: &mut Vec3, hi: &mut Vec3, pivot: Vec3, yaw_deg: f32, pitch_deg: f32) {
    let r = view_rotation(yaw_deg, pitch_deg);
    let mut nlo = Vec3::splat(1e9);
    let mut nhi = Vec3::splat(-1e9);
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 != 0 { hi.x } else { lo.x },
            if i & 2 != 0 { hi.y } else { lo.y },
            if i & 4 != 0 { hi.z } else { lo.z },
        );
        let rc = r.mul_vec3(c - pivot) + pivot;
        nlo = min3(nlo, rc);
        nhi = max3(nhi, rc);
    }
    *lo = nlo;
    *hi = nhi;
}

/// CameraForBox: frames a world box at `size` with a multiplicative margin.
pub fn camera_for_box(lo: Vec3, hi: Vec3, size: i32, margin: f32) -> PreviewCamera {
    let ext = hi - lo;
    let mut span = fmax(ext.x, ext.y) * margin;
    if span.is_nan() || span <= 1e-4 {
        span = 0.1;
    }
    PreviewCamera {
        center: Vec2::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5),
        scale: (size * 2) as f32 / span,
        feet_y: lo.y,
        width: ext.x,
    }
}

/// RenderWornItemIcon: the closet item on the blank mannequin, framed the way the marketplace art frames it,
/// turned to a 3/4 view on a transparent background.
pub fn render_worn_item_icon(
    meshes: &[BakedMesh],
    materials: &[BakedMaterial],
    mut posed: Vec<PosedMesh>,
    res: &Resolved,
    args: &Args,
) -> bool {
    let mut target: i32 = -1;
    let mut head: i32 = -1;
    for (i, c) in res.components.iter().enumerate() {
        if c.info.asset_id == args.icon_target {
            target = i as i32;
        }
        if u32::from(c.info.categories) & category::HEAD != 0 {
            head = i as i32;
        }
    }
    let box_of = |posed: &[PosedMesh], component: i32| -> Option<(Vec3, Vec3)> {
        let mut lo = Vec3::splat(1e9);
        let mut hi = Vec3::splat(-1e9);
        let mut any = false;
        for (m, pm) in meshes.iter().zip(posed.iter()) {
            if component >= 0 && m.component != component {
                continue;
            }
            for &p in &pm.pos {
                lo = min3(lo, p);
                hi = max3(hi, p);
                any = true;
            }
        }
        any.then_some((lo, hi))
    };
    let item = if target >= 0 { box_of(&posed, target) } else { None };
    let head_box = if head >= 0 { box_of(&posed, head) } else { None };
    let Some((alo, ahi)) = box_of(&posed, -1) else {
        return false;
    };
    let cat = args.icon_categories;
    let (mut lo, mut hi) = (alo, ahi);
    let mut margin = 1.08f32;
    if let Some((ilo, ihi)) = item {
        lo = ilo;
        hi = ihi;
        margin = 1.18;
        if cat & (category::HAIR | category::HAT | category::GLASSES | category::EARRINGS) != 0 {
            if let Some((hlo, hhi)) = head_box {
                lo = min3(lo, hlo);
                hi = max3(hi, hhi);
            }
            margin = 1.14;
        } else if cat & category::SHOES != 0 {
            hi.y += (hi.y - lo.y) * 0.45;
        } else if cat & (category::RING | category::WRISTWEAR | category::GLOVES) != 0 {
            let e = hi - lo;
            lo -= e * 0.35;
            hi += e * 0.35;
        }
    }
    let pivot = (lo + hi) * 0.5;
    rotate_posed(&mut posed, pivot, args.icon_yaw, args.icon_pitch);
    rotate_box(&mut lo, &mut hi, pivot, args.icon_yaw, args.icon_pitch);
    let cam = camera_for_box(lo, hi, args.icon_size, margin);
    render_preview_image(
        meshes,
        materials,
        &posed,
        &cam,
        args.icon_size,
        &args.icon_out,
        &HashMap::new(),
        true,
        None,
    )
}

/// The preview stage of Run: preview_T-Pose.png plus one PNG per clip at its picked frame, the face layers
/// re-composited per distinct mouth/brow/eye combination the clip's texture track names.
#[allow(clippy::too_many_arguments)]
pub fn run_previews(
    args: &Args,
    meshes: &[BakedMesh],
    materials: &[BakedMaterial],
    xforms: &[JointXform],
    skeleton: &Skeleton,
    anims: &[AnimExport],
    head_batches: &[HeadBatchInfo],
    rep_layers: &[Vec<Rc<Image>>; 6],
) -> std::io::Result<()> {
    std::fs::create_dir_all(&args.preview_dir)?;
    let t0 = Instant::now();
    let mut all_poses: Vec<Vec<PosedMesh>> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut faces: Vec<HashMap<usize, Rc<Image>>> = Vec::new();
    let mut face_cache: BTreeMap<String, Rc<Image>> = BTreeMap::new();
    let mut face_bakes = 0usize;
    let mut face_for = |anim: &Animation, f: usize| -> HashMap<usize, Rc<Image>> {
        let mut ov = HashMap::new();
        if anim.texture_count < 5 {
            return ov;
        }
        let Some(el) = anim.texture_frame_set.frames.get(f) else {
            return ov;
        };
        if el.len() < 5 {
            return ov;
        }
        let layer = |slot: usize, idx: u32| -> Option<Rc<Image>> {
            let l = &rep_layers[slot];
            if l.is_empty() {
                return None;
            }
            // Runtime clamp: an out-of-range layer shows the neutral one.
            let idx = if (idx as usize) >= l.len() {
                0
            } else {
                idx as usize
            };
            l[idx].ok.then(|| l[idx].clone())
        };
        let mouth = el[0].layer_index;
        let (brow_l, brow_r) = (el[1].layer_index, el[2].layer_index);
        let (eye_l, eye_r) = (el[3].layer_index, el[4].layer_index);
        for hb in head_batches {
            let brow = if hb.brow_usage == 8 { brow_r } else { brow_l };
            let eye = if hb.eye_usage == 10 { eye_r } else { eye_l };
            if mouth == 0 && brow == 0 && eye == 0 {
                continue;
            }
            let key = format!("{}|{mouth}|{brow}|{eye}", hb.material_index);
            let img = match face_cache.get(&key) {
                Some(img) => img.clone(),
                None => {
                    let mut r = hb.recipe.clone();
                    if r.mouth.img.is_some() {
                        if let Some(l) = layer(0, mouth) {
                            r.mouth.img = Some(l);
                        }
                    }
                    if r.brow.img.is_some() {
                        if let Some(l) = layer(2, brow) {
                            r.brow.img = Some(l);
                        }
                    }
                    if r.eye.img.is_some() {
                        if let Some(l) = layer(1, eye) {
                            r.eye.img = Some(l);
                        }
                    }
                    let hm = &meshes[hb.mesh_index];
                    let img = Rc::new(bake_material(r, &hm.verts, &hm.indices, 256, (0, 0), None));
                    face_cache.insert(key, img.clone());
                    face_bakes += 1;
                    img
                }
            };
            ov.insert(hb.material_index, img);
        }
        ov
    };
    all_poses.push(pose_meshes(
        meshes,
        &pose_skin_matrices(xforms, skeleton, None, 0, 0),
    ));
    names.push("T-Pose".into());
    faces.push(HashMap::new());
    for a in anims {
        if a.anim.pose_counts[0] == 0 || a.anim.pose_frame_sets[0].frames.is_empty() {
            continue;
        }
        let rule = if a.preview_frame.is_empty() {
            &args.preview_frame
        } else {
            &a.preview_frame
        };
        let f = pick_preview_frame(&a.anim, rule);
        all_poses.push(pose_meshes(
            meshes,
            &pose_skin_matrices(xforms, skeleton, Some(&a.anim), f, 0),
        ));
        names.push(sanitize_name(&a.name));
        faces.push(face_for(&a.anim, f));
    }
    let cam = frame_camera(&all_poses, args.preview_size * 2);
    for ((poses, name), face) in all_poses.iter().zip(&names).zip(&faces) {
        render_preview_image(
            meshes,
            materials,
            poses,
            &cam,
            args.preview_size,
            &args.preview_dir.join(format!("preview_{name}.png")),
            face,
            false,
            None,
        );
    }
    println!(
        "\n=== previews: {} images ({}px, {face_bakes} face composites) in {} ms -> {} ===",
        all_poses.len(),
        args.preview_size,
        t0.elapsed().as_millis(),
        args.preview_dir.display()
    );
    Ok(())
}
