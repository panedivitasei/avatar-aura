//! Ports --prop-icons and --gen-icons from avatar_export.cpp: PropScene/BuildPropScene, RunPropIcons, ChoosePropFrame,
//! RenderPropIcon and RunGenIcons with its blank-mannequin worn-item renders.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use avatar_formats::animation::load_option as anim_opt;
use avatar_formats::closet::parse_index;
use avatar_formats::model::ShaderParameterType;
use avatar_formats::prop::PropLoadOptions;
use avatar_formats::{Animation, Closet, ClosetItem, Model, Prop, Skeleton};
use glam::Vec3;

use crate::args::Args;
use crate::glmath::{fmax, length3, max3, min3, Mat4};
use crate::manifest::{category, color_to_float4, Float4, WHITE};
use crate::msvc_sort;
use crate::preview::{
    camera_for_box, pose_meshes, pose_skin_matrices, render_preview_image, rotate_posed, PosedMesh,
    PreviewCamera,
};
use crate::resolve::{custom_color_table, load_closet};
use crate::skin::{build_joint_xforms, unpack_skin, BakedMesh, JointXform};
use crate::textures::{
    bake_batch, decode_texture_layer, shader_name, BakedMaterial, MaterialRecipe, Sampler, ShaderKind,
};
use crate::util::{atof, atoi, lower};

/// A closet prop's textures baked and its batches unpacked, shared by --prop-icons and --gen-icons.
#[derive(Default)]
pub struct PropScene {
    pub materials: Vec<BakedMaterial>,
    pub meshes: Vec<BakedMesh>,
    pub has_table: bool,
    pub empty_intensity: bool,
    pub shaders: String,
    pub verts: usize,
    pub tris: usize,
}

fn prop_options() -> PropLoadOptions {
    PropLoadOptions {
        model: 0,
        skeleton: 0,
        animation: anim_opt::ELEMENTS,
        blend_shape: 0,
    }
}

pub fn build_prop_scene(bytes: &[u8], model: &Model) -> PropScene {
    let mut s = PropScene::default();
    let mut overrides: BTreeMap<u32, Float4> = BTreeMap::new();
    overrides.insert(22, WHITE);
    if let Some(colors) = custom_color_table(bytes) {
        for (i, &c) in colors.iter().enumerate() {
            overrides.insert(22 + i as u32, color_to_float4(c));
        }
        s.has_table = true;
    }
    let images: Vec<Rc<_>> = model
        .textures
        .iter()
        .map(|t| Rc::new(decode_texture_layer(&t.texture, 0)))
        .collect();
    for (bi, b) in model.triangle_batches.iter().enumerate() {
        if b.triangle_count == 0 || b.vertices.is_empty() {
            continue;
        }
        let mut usage_tex: BTreeMap<u32, i32> = BTreeMap::new();
        let mut usage_uv: BTreeMap<u32, i32> = BTreeMap::new();
        let mut usage_flags: BTreeMap<u32, u32> = BTreeMap::new();
        let mut usage_const: BTreeMap<u32, Float4> = BTreeMap::new();
        for sp in &b.shader_parameters {
            match sp.r#type {
                ShaderParameterType::Texture => {
                    usage_tex.insert(sp.usage, i32::from(sp.texture.index));
                    usage_uv.insert(sp.usage, i32::from(sp.texture.uv_layer));
                    usage_flags.insert(sp.usage, u32::from(sp.texture.flags));
                }
                ShaderParameterType::Invalid => {}
                _ => {
                    usage_const.insert(sp.usage, sp.constant_values);
                }
            }
        }
        for (&k, &v) in &overrides {
            usage_const.insert(k, v);
        }
        let mut recipe = MaterialRecipe {
            kind: ShaderKind::Body,
            shader_id: b.shader_id,
            ..Default::default()
        };
        let mut empty_intensity = false;
        let mut tex_for = |usage: u32, smp: &mut Sampler| {
            let Some(&ti) = usage_tex.get(&usage) else {
                return;
            };
            smp.uv_layer = usage_uv.get(&usage).copied().unwrap_or(0).clamp(0, 5) as usize;
            smp.flags = usage_flags.get(&usage).copied().unwrap_or(0);
            match usize::try_from(ti)
                .ok()
                .and_then(|i| images.get(i))
                .filter(|i| i.ok)
            {
                Some(img) => smp.img = Some(img.clone()),
                None if usage == 2 => empty_intensity = true,
                None => {}
            }
        };
        tex_for(1, &mut recipe.color);
        tex_for(2, &mut recipe.intensity);
        tex_for(3, &mut recipe.decal);
        s.empty_intensity |= empty_intensity;
        for k in 0..3u32 {
            recipe.custom[k as usize] = usage_const.get(&(22 + k)).copied().unwrap_or(WHITE);
        }
        recipe.out_uv_layer = if recipe.color.img.is_some() {
            recipe.color.uv_layer
        } else {
            0
        };
        if !s.shaders.is_empty() {
            s.shaders.push('|');
        }
        s.shaders.push_str(shader_name(b.shader_id));
        let mut mesh = BakedMesh {
            component: 0,
            is_prop: true,
            uv_count: b.uv_count,
            verts: b.vertices.iter().map(|v| unpack_skin(v, b.uv_count)).collect(),
            indices: b.indices.clone(),
            ..Default::default()
        };
        let image = bake_batch(&mut recipe, &mut mesh.verts, &mut mesh.indices, 256, None);
        let has_alpha = image.rgba.iter().skip(3).step_by(4).any(|&a| a < 128);
        let mat = BakedMaterial {
            name: format!("Prop_{bi}"),
            shader_id: b.shader_id,
            image: Rc::new(image),
            uv_layer: recipe.out_uv_layer,
            has_alpha,
            ..Default::default()
        };
        mesh.material = s.materials.len() as i32;
        s.materials.push(mat);
        s.verts += mesh.verts.len();
        s.tris += mesh.indices.len() / 3;
        s.meshes.push(mesh);
    }
    s
}

/// PoseMeshes for carryable meshes, which are flagged to keep their rest pose: posed explicitly.
fn pose_prop(meshes: &mut [BakedMesh], skin: &[Mat4]) -> Vec<PosedMesh> {
    for m in meshes.iter_mut() {
        m.is_prop = false;
    }
    let posed = pose_meshes(meshes, skin);
    for m in meshes.iter_mut() {
        m.is_prop = true;
    }
    posed
}

fn rest_posed(meshes: &[BakedMesh]) -> Vec<PosedMesh> {
    meshes
        .iter()
        .map(|m| PosedMesh {
            pos: m.verts.iter().map(|v| v.position).collect(),
            nrm: m.verts.iter().map(|v| v.normal).collect(),
        })
        .collect()
}

fn prop_clip(prop: &Prop) -> Option<&Animation> {
    prop.animation
        .as_ref()
        .filter(|a| a.pose_counts[1] > 0 && !a.pose_frame_sets[1].frames.is_empty())
}

/// The closet's items in the C++ order: closet_index.tsv rows put through MSVC std::sort by name, which leaves
/// equal names in a different order than the stable sort the Rust closet uses.
fn closet_order(closet: &Closet) -> Vec<ClosetItem> {
    let Ok(bytes) = std::fs::read(closet.dir().join("closet_index.tsv")) else {
        return closet.items().to_vec();
    };
    let mut items = parse_index(&String::from_utf8_lossy(&bytes));
    msvc_sort::sort_by(&mut items, |a, b| a.name < b.name);
    items
}

/// `--prop-icons <closet_dir> [--icons-out <dir>] [--size N] [--report <csv>] [--filter <substr>] [--limit N]`.
pub fn run_prop_icons(argv: &[String]) -> anyhow::Result<i32> {
    let mut icons_out = PathBuf::new();
    let mut report = PathBuf::new();
    let mut size = 128i32;
    let mut limit = -1i32;
    let mut filter = String::new();
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let has_next = i + 1 < argv.len();
        match argv[i].as_str() {
            "--icons-out" if has_next => {
                i += 1;
                icons_out = PathBuf::from(&argv[i]);
            }
            "--report" if has_next => {
                i += 1;
                report = PathBuf::from(&argv[i]);
            }
            "--size" if has_next => {
                i += 1;
                size = atoi(&argv[i]).max(16);
            }
            "--limit" if has_next => {
                i += 1;
                limit = atoi(&argv[i]);
            }
            "--filter" if has_next => {
                i += 1;
                filter = argv[i].clone();
            }
            _ => pos.push(argv[i].clone()),
        }
        i += 1;
    }
    let Some(first) = pos.first() else {
        println!(
            "usage: avatarextract --prop-icons <closet_dir> [--icons-out <dir>] [--size N] \
             [--report <csv>] [--filter <substr>] [--limit N]"
        );
        return Ok(1);
    };
    let closet_dir = PathBuf::from(first);
    if icons_out.as_os_str().is_empty() {
        icons_out = closet_dir.join("icons");
    }
    let closet = load_closet(&closet_dir);
    println!(
        "(mode: prop icons)\ncloset: {} ({} items)",
        closet_dir.display(),
        closet.items().len()
    );
    std::fs::create_dir_all(&icons_out)?;
    let mut csv = if report.as_os_str().is_empty() {
        None
    } else {
        File::create(&report).ok()
    };
    if let Some(f) = csv.as_mut() {
        writeln!(f, "guid\tname\tstatus\tverts\ttris\tbatches\tframes\tbbox_x\tbbox_y\tbbox_z\tdark_frac\tcolor_table\tempty_intensity\tshaders")?;
    }
    let lfilter = lower(&filter);
    let (mut done, mut ok, mut failed, mut dark) = (0usize, 0usize, 0usize, 0usize);
    let t0 = Instant::now();
    for item in &closet_order(&closet) {
        if item.categories & category::PROP == 0 {
            continue;
        }
        if !lfilter.is_empty() && !lower(&item.name).contains(&lfilter) {
            continue;
        }
        if limit >= 0 && done as i64 >= i64::from(limit) {
            break;
        }
        done += 1;
        let guid = item.id.to_string();
        let mut status = "ok";
        let bytes = closet.read_item_bytes(&item.id).unwrap_or_default();
        let mut prop: Option<Prop> = None;
        let mut loose_model: Option<Model> = None;
        if bytes.is_empty() {
            status = "unreadable";
        } else {
            prop = Prop::load(&bytes, &prop_options()).ok().flatten();
            if prop.is_none() {
                loose_model = Model::load(&bytes, 0).ok().flatten();
            }
            if prop.is_none() && loose_model.is_none() {
                status = "no-model";
            } else if prop.is_none() {
                status = "no-skeleton";
            } else if prop.as_ref().is_some_and(|p| p.animation.is_none()) {
                status = "no-animation";
            }
        }
        let Some(model) = prop.as_ref().map(|p| &p.model).or(loose_model.as_ref()) else {
            failed += 1;
            if let Some(f) = csv.as_mut() {
                writeln!(
                    f,
                    "{guid}\t{}\t{status}\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t",
                    item.name
                )?;
            }
            continue;
        };
        let mut scene = build_prop_scene(&bytes, model);
        // Pose at the middle of the carryable clip: spawn-in props are at scale 0 on frame 0.
        let mut frames = 0usize;
        let posed = match prop.as_ref() {
            Some(p) => {
                let xf = build_joint_xforms(&p.skeleton, false);
                let a = prop_clip(p);
                let f = a.map_or(0, |a| a.frame_count as usize / 2);
                if let Some(a) = a {
                    frames = a.frame_count as usize;
                }
                let skin = pose_skin_matrices(&xf, &p.skeleton, a, f, 1);
                pose_prop(&mut scene.meshes, &skin)
            }
            None => rest_posed(&scene.meshes),
        };
        let mut lo = Vec3::splat(1e9);
        let mut hi = Vec3::splat(-1e9);
        for pm in &posed {
            for &q in &pm.pos {
                lo = min3(lo, q);
                hi = max3(hi, q);
            }
        }
        let ext = hi - lo;
        let mut span = fmax(ext.x, ext.y) * 1.15;
        if span.is_nan() || span <= 1e-4 {
            span = 0.1;
        }
        let cam = PreviewCamera {
            center: glam::Vec2::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5),
            scale: (size * 2) as f32 / span,
            feet_y: lo.y,
            width: ext.x,
        };
        let png = icons_out.join(format!("{guid}.png"));
        render_preview_image(
            &scene.meshes,
            &scene.materials,
            &posed,
            &cam,
            size,
            &png,
            &HashMap::new(),
            true,
            None,
        );
        // Darkness of the baked textures over their opaque texels.
        let (mut dark_px, mut all_px) = (0usize, 0usize);
        for m in &scene.materials {
            for p in m.image.rgba.as_chunks::<4>().0 {
                if p[3] < 128 {
                    continue;
                }
                all_px += 1;
                if i32::from(p[0]) + i32::from(p[1]) + i32::from(p[2]) < 60 {
                    dark_px += 1;
                }
            }
        }
        let dark_frac = if all_px != 0 {
            dark_px as f32 / all_px as f32
        } else {
            0.0
        };
        if dark_frac > 0.6 {
            dark += 1;
        }
        ok += 1;
        if let Some(f) = csv.as_mut() {
            writeln!(
                f,
                "{guid}\t{}\t{status}\t{}\t{}\t{}\t{frames}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}",
                item.name,
                scene.verts,
                scene.tris,
                scene.meshes.len(),
                f64::from(ext.x),
                f64::from(ext.y),
                f64::from(ext.z),
                f64::from(dark_frac),
                u8::from(scene.has_table),
                u8::from(scene.empty_intensity),
                scene.shaders
            )?;
        }
        if done % 200 == 0 {
            println!("  {done} props...");
        }
    }
    println!(
        "props: {done} processed, {ok} rendered, {failed} failed, {dark} mostly-dark, in {} ms -> {}",
        t0.elapsed().as_millis(),
        icons_out.display()
    );
    Ok(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropFrameChoice {
    pub frame: usize,
    pub frames: usize,
    /// Smoothed joint motion at the chosen frame.
    pub motion: f32,
    /// Joint-cloud extent at the chosen frame.
    pub extent: f32,
    pub why: &'static str,
}

impl Default for PropFrameChoice {
    fn default() -> Self {
        PropFrameChoice {
            frame: 0,
            frames: 0,
            motion: 0.0,
            extent: 0.0,
            why: "bind",
        }
    }
}

fn joint_world_scale(world: &Mat4) -> f32 {
    world.to_mat3().determinant().abs().cbrt()
}

/// ChoosePropFrame: the frame that reads as the prop, fully spawned and as still as the clip gets, with the intro
/// discouraged; the pick is centred in its low-motion run so a hold is shown, not its edge.
#[allow(clippy::needless_range_loop)]
pub fn choose_prop_frame(
    skel: &Skeleton,
    xf: &[JointXform],
    a: Option<&Animation>,
    meshes: &[BakedMesh],
) -> PropFrameChoice {
    let mut c = PropFrameChoice::default();
    let Some(a) = a.filter(|a| a.pose_counts[1] != 0 && !a.pose_frame_sets[1].frames.is_empty()) else {
        return c;
    };
    let n = (a.frame_count as usize).min(a.pose_frame_sets[1].frames.len());
    if n == 0 {
        return c;
    }
    c.frames = n;
    // Vertex mass per joint (dominant weight): what a frame still shows when some joints scale to nothing.
    let mut joint_mass = vec![0.0f32; xf.len()];
    for m in meshes {
        for v in &m.verts {
            let mut best: Option<usize> = None;
            let mut bw = 0.0f32;
            for k in 0..4 {
                if v.weights[k] > bw && usize::from(v.joints[k]) < joint_mass.len() {
                    bw = v.weights[k];
                    best = Some(usize::from(v.joints[k]));
                }
            }
            if let Some(b) = best {
                joint_mass[b] += 1.0;
            }
        }
    }
    let mut pos: Vec<Vec<Vec3>> = vec![Vec::new(); n];
    let mut extent = vec![0.0f32; n];
    let mut mass = vec![0.0f32; n];
    let mut motion = vec![0.0f32; n];
    for f in 0..n {
        let skin = pose_skin_matrices(xf, skel, Some(a), f, 1);
        pos[f] = vec![Vec3::ZERO; skin.len()];
        let mut lo = Vec3::splat(1e9);
        let mut hi = Vec3::splat(-1e9);
        for j in 0..skin.len() {
            let world = skin[j].mul(&xf[j].bind_world);
            pos[f][j] = world.col3(3);
            lo = min3(lo, pos[f][j]);
            hi = max3(hi, pos[f][j]);
            if joint_world_scale(&world) >= 0.1 && j < joint_mass.len() {
                mass[f] += joint_mass[j];
            }
        }
        extent[f] = if skin.is_empty() { 0.0 } else { length3(hi - lo) };
    }
    let mut max_mass = 0.0f32;
    for &m in &mass {
        max_mass = fmax(max_mass, m);
    }
    for f in 1..n {
        let mut m = 0.0f32;
        for (p, q) in pos[f].iter().zip(pos[f - 1].iter()) {
            m += length3(*p - *q);
        }
        motion[f] = m;
    }
    if n > 1 {
        motion[0] = motion[1];
    }
    let mut smooth = vec![0.0f32; n];
    let mut max_smooth = 0.0f32;
    for f in 0..n {
        let mut sum = 0.0f32;
        let mut cnt = 0i32;
        for d in -2i64..=2 {
            let g = f as i64 + d;
            if g < 0 || g >= n as i64 {
                continue;
            }
            sum += motion[g as usize];
            cnt += 1;
        }
        smooth[f] = if cnt != 0 { sum / cnt as f32 } else { 0.0 };
        max_smooth = fmax(max_smooth, smooth[f]);
    }
    // Candidates are spawned frames; spread is ignored, multi-part props read worst when most spread out.
    let mut ok: Vec<bool> = (0..n)
        .map(|f| max_mass <= 0.0 || mass[f] >= 0.6 * max_mass)
        .collect();
    let candidates = ok.iter().filter(|&&o| o).count();
    if candidates == 0 {
        ok.iter_mut().for_each(|o| *o = true);
    }
    let mut best = n / 2;
    let mut best_score = 1e30f32;
    for f in 0..n {
        if !ok[f] {
            continue;
        }
        let mut score = if max_smooth > 1e-9 {
            smooth[f] / max_smooth
        } else {
            0.0
        };
        score += 0.15 * (1.0 - f as f32 / n as f32);
        if f < n / 5 {
            score += 0.10;
        }
        if score < best_score {
            best_score = score;
            best = f;
        }
    }
    let band = smooth[best] * 1.25 + 1e-6;
    let (mut lo, mut hi) = (best, best);
    while lo > 0 && ok[lo - 1] && smooth[lo - 1] <= band {
        lo -= 1;
    }
    while hi + 1 < n && ok[hi + 1] && smooth[hi + 1] <= band {
        hi += 1;
    }
    c.frame = (lo + hi) / 2;
    c.motion = smooth[c.frame];
    c.extent = extent[c.frame];
    c.why = if candidates != 0 {
        "rest"
    } else {
        "rest-unfiltered"
    };
    c
}

/// RenderPropIcon: the prop alone at its chosen frame, 3/4 view, transparent background. The error string is
/// the report status.
pub fn render_prop_icon(
    bytes: &[u8],
    size: i32,
    yaw: f32,
    pitch: f32,
    png: &Path,
    choice: &mut PropFrameChoice,
) -> Result<(), &'static str> {
    let prop = Prop::load(bytes, &prop_options()).ok().flatten();
    let loose = if prop.is_none() {
        Model::load(bytes, 0).ok().flatten()
    } else {
        None
    };
    let Some(model) = prop.as_ref().map(|p| &p.model).or(loose.as_ref()) else {
        return Err("no-model");
    };
    let mut scene = build_prop_scene(bytes, model);
    if scene.meshes.is_empty() {
        return Err("no-geometry");
    }
    // Framing ignores geometry the pose scales away: a vertex counts while its blended joint scale is material.
    let mut visible: Vec<Vec<bool>> = vec![Vec::new(); scene.meshes.len()];
    let mut posed = match prop.as_ref() {
        Some(p) => {
            let xf = build_joint_xforms(&p.skeleton, false);
            *choice = choose_prop_frame(&p.skeleton, &xf, p.animation.as_ref(), &scene.meshes);
            let a = if choice.frames == 0 {
                None
            } else {
                p.animation.as_ref()
            };
            let skin = pose_skin_matrices(&xf, &p.skeleton, a, choice.frame, 1);
            let jscale: Vec<f32> = skin
                .iter()
                .zip(xf.iter())
                .map(|(s, x)| joint_world_scale(&s.mul(&x.bind_world)))
                .chain(std::iter::repeat(1.0))
                .take(skin.len())
                .collect();
            let posed = pose_prop(&mut scene.meshes, &skin);
            for (vis, m) in visible.iter_mut().zip(scene.meshes.iter()) {
                *vis = m
                    .verts
                    .iter()
                    .map(|v| {
                        let (mut sc, mut wsum) = (0.0f32, 0.0f32);
                        for k in 0..4 {
                            let w = v.weights[k];
                            let j = usize::from(v.joints[k]);
                            if w <= 0.0 || j >= jscale.len() {
                                continue;
                            }
                            sc += w * jscale[j];
                            wsum += w;
                        }
                        !(wsum > 0.0 && sc / wsum < 0.05)
                    })
                    .collect();
            }
            posed
        }
        None => rest_posed(&scene.meshes),
    };
    let frame_box = |posed: &[PosedMesh]| -> Option<(Vec3, Vec3)> {
        let mut lo = Vec3::splat(1e9);
        let mut hi = Vec3::splat(-1e9);
        let mut any = false;
        for (mi, pm) in posed.iter().enumerate() {
            for (vi, &p) in pm.pos.iter().enumerate() {
                if visible.get(mi).and_then(|v| v.get(vi)) == Some(&false) {
                    continue;
                }
                lo = min3(lo, p);
                hi = max3(hi, p);
                any = true;
            }
        }
        if !any {
            // Everything scaled away: fall back to all geometry.
            for pm in posed {
                for &q in &pm.pos {
                    lo = min3(lo, q);
                    hi = max3(hi, q);
                    any = true;
                }
            }
        }
        any.then_some((lo, hi))
    };
    let Some((lo, hi)) = frame_box(&posed) else {
        return Err("no-geometry");
    };
    // Flat props (mats, rugs, boards lying down) are edge-on at the default pitch: look down on them instead.
    let mut pitch = pitch;
    let e = hi - lo;
    let footprint = fmax(e.x, e.z);
    let flatness = if footprint > 1e-6 { e.y / footprint } else { 1.0 };
    if flatness < 0.25 {
        pitch = fmax(pitch, 55.0);
    } else if flatness < 0.5 {
        pitch = fmax(pitch, 32.0);
    }
    let pivot = (lo + hi) * 0.5;
    rotate_posed(&mut posed, pivot, yaw, pitch);
    let (lo, hi) = frame_box(&posed).unwrap_or((lo, hi));
    let cam = camera_for_box(lo, hi, size, 1.14);
    let mut covered = 0usize;
    if !render_preview_image(
        &scene.meshes,
        &scene.materials,
        &posed,
        &cam,
        size,
        png,
        &HashMap::new(),
        true,
        Some(&mut covered),
    ) {
        return Err("render-failed");
    }
    // Nothing visible (animation-only props, fully transparent art) is not an icon.
    if covered < (size as usize * 2) * (size as usize * 2) / 250 {
        return Err("empty-render");
    }
    Ok(())
}

/// The editor's blank mannequin wearing one item: default height and weight, no shapes, no face features,
/// every colour white, the item alone in component slot 0 with its category mask.
fn wear_manifest(base: &[u8], id_bytes: [u8; 16], categories: u32) -> Vec<u8> {
    let mut m = base.to_vec();
    m[4..12].fill(0);
    m[0x0C..0x3C].fill(0);
    m[0x3C..0xFC].fill(0);
    m[0xFC..0xFC + 36].fill(0xFF);
    m[0x160..0x300].fill(0);
    m[0x300..0x380].fill(0);
    m[0x160..0x170].copy_from_slice(&id_bytes);
    let cats = (categories & 0x1FFF) as u16;
    m[0x160 + 16] = (cats >> 8) as u8;
    m[0x160 + 17] = (cats & 0xFF) as u8;
    m
}

/// `--gen-icons <closet_dir> [--toc <pack.toc>] [--mannequins <dir>] [--icons-out <dir>] [--existing <dir>]
/// [--size N] [--filter <substr>] [--limit N] [--force] [--report <tsv>] [--yaw DEG] [--pitch DEG] [--shard k/n]`.
pub fn run_gen_icons(argv: &[String]) -> anyhow::Result<i32> {
    let mut icons_out = PathBuf::new();
    let mut report = PathBuf::new();
    let mut toc = PathBuf::new();
    let mut mannequins = PathBuf::new();
    let mut existing = PathBuf::new();
    let (mut size, mut limit, mut shard, mut shards) = (128i32, -1i32, 0i32, 1i32);
    let (mut yaw, mut pitch) = (25.0f32, 10.0f32);
    let mut force = false;
    let mut filter = String::new();
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let has_next = i + 1 < argv.len();
        let a = argv[i].as_str();
        let mut take = || {
            i += 1;
            argv[i].clone()
        };
        match a {
            "--icons-out" if has_next => icons_out = PathBuf::from(take()),
            "--report" if has_next => report = PathBuf::from(take()),
            "--toc" if has_next => toc = PathBuf::from(take()),
            "--mannequins" if has_next => mannequins = PathBuf::from(take()),
            "--existing" if has_next => existing = PathBuf::from(take()),
            "--size" if has_next => size = atoi(&take()).max(16),
            "--limit" if has_next => limit = atoi(&take()),
            "--yaw" if has_next => yaw = atof(&take()) as f32,
            "--pitch" if has_next => pitch = atof(&take()) as f32,
            "--filter" if has_next => filter = take(),
            "--force" => force = true,
            "--shard" if has_next => {
                // k/n: this process takes the items whose index % n == k.
                let v = take();
                if let Some(slash) = v.find('/') {
                    shard = atoi(&v[..slash]);
                    shards = atoi(&v[slash + 1..]).max(1);
                }
            }
            _ => pos.push(argv[i].clone()),
        }
        i += 1;
    }
    let Some(first) = pos.first() else {
        println!(
            "usage: avatarextract --gen-icons <closet_dir> [--toc <AvatarAssetPack.toc>] \
             [--mannequins <dir>] [--icons-out <dir>] [--existing <dir>] [--size N] [--filter <substr>] \
             [--limit N] [--force] [--report <tsv>] [--yaw DEG] [--pitch DEG]"
        );
        return Ok(1);
    };
    let closet_dir = PathBuf::from(first);
    if icons_out.as_os_str().is_empty() {
        icons_out = closet_dir.join("icons");
    }
    // --existing: the icons that count as already there, so a review run can render into a scratch dir.
    if existing.as_os_str().is_empty() {
        existing = icons_out.clone();
    }
    if toc.as_os_str().is_empty() {
        let cand = closet_dir
            .parent()
            .unwrap_or(Path::new(""))
            .join("AvatarAssetPack.toc");
        if cand.exists() {
            toc = cand;
        }
    }
    if mannequins.as_os_str().is_empty() {
        mannequins = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
    }
    if toc.as_os_str().is_empty() || !toc.exists() {
        println!("ERROR: asset pack not found; pass --toc <AvatarAssetPack.toc>");
        return Ok(1);
    }
    let mq_male = mannequins.join("mannequin_male.amd");
    let mq_female = mannequins.join("mannequin_female.amd");
    if !mq_male.exists() {
        println!(
            "ERROR: mannequin manifests not found in {} (pass --mannequins <dir>)",
            mannequins.display()
        );
        return Ok(1);
    }
    let _ = std::fs::create_dir_all(&icons_out);
    let closet = load_closet(&closet_dir);
    let gen_name = if shards > 1 {
        format!("icons_generated_shard{shard}.tsv")
    } else {
        "icons_generated.tsv".to_owned()
    };
    let gen_tsv = icons_out.parent().unwrap_or(Path::new("")).join(gen_name);
    let mut generated: HashSet<String> = HashSet::new();
    if let Ok(f) = File::open(&gen_tsv) {
        for line in BufReader::new(f).split(b'\n').map_while(Result::ok) {
            if line.iter().position(|&b| b == b'\t') == Some(36) {
                generated.insert(String::from_utf8_lossy(&line[..36]).into_owned());
            }
        }
    }
    let mut gen = OpenOptions::new().append(true).create(true).open(&gen_tsv).ok();
    let mut csv = if report.as_os_str().is_empty() {
        None
    } else {
        File::create(&report).ok()
    };
    if let Some(f) = csv.as_mut() {
        writeln!(f, "guid\tname\tkind\tstatus\tframe\tframes\tmotion\textent\tms")?;
    }
    // Per-shard scratch: parallel shards must not share the manifest or bake directory.
    let work = std::env::temp_dir().join(format!("avatarextract_genicons_{shard}"));
    let _ = std::fs::create_dir_all(&work);
    let mq_bytes = [
        std::fs::read(&mq_male).unwrap_or_default(),
        if mq_female.exists() {
            std::fs::read(&mq_female).unwrap_or_default()
        } else {
            Vec::new()
        },
    ];
    let lfilter = lower(&filter);
    println!(
        "(mode: generate fill-in icons)\ncloset: {} ({} items, {} stand-ins on record)",
        closet_dir.display(),
        closet.items().len(),
        generated.len()
    );
    let (mut done, mut ok, mut failed, mut present, mut props, mut worn) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let t0 = Instant::now();
    for (index, item) in closet_order(&closet).iter().enumerate() {
        if (index % shards as usize) as i32 != shard {
            continue;
        }
        if !lfilter.is_empty() && !lower(&item.name).contains(&lfilter) {
            continue;
        }
        let guid = item.id.to_string();
        let png = icons_out.join(format!("{guid}.png"));
        // Present in the reference set, or already rendered into icons_out (resume).
        if (existing.join(format!("{guid}.png")).exists() || png.exists())
            && !(force && generated.contains(&guid))
        {
            present += 1;
            continue;
        }
        if limit >= 0 && done as i64 >= i64::from(limit) {
            break;
        }
        done += 1;
        let t1 = Instant::now();
        let mut status = String::from("ok");
        let mut kind = "worn";
        let mut choice = PropFrameChoice::default();
        let mut good = false;
        // The blank mannequin wearing (or performing) the item.
        let render_worn = |status: &mut String| -> bool {
            let base = if item.bodies == 2 && !mq_bytes[1].is_empty() {
                &mq_bytes[1]
            } else {
                &mq_bytes[0]
            };
            if base.len() != 1000 {
                *status = "mannequin-manifest".into();
                return false;
            }
            let m = wear_manifest(base, item.id.to_bytes(), item.categories);
            let manifest = work.join("wear.bin");
            if std::fs::write(&manifest, &m).is_err() {
                *status = "work-dir".into();
                return false;
            }
            let mut a = Args {
                manifest,
                out_dir: work.join("out"),
                toc: toc.clone(),
                closet: closet_dir.clone(),
                no_scale: true,
                bake_size: 256,
                pack_anim_filters: vec!["Animation Generic Stand 0@0.5".into()],
                icon_out: png.clone(),
                icon_target: item.id,
                icon_categories: item.categories,
                icon_size: size,
                icon_yaw: yaw,
                icon_pitch: pitch,
                ..Default::default()
            };
            let legacy = toc
                .parent()
                .unwrap_or(Path::new(""))
                .join("AvatarAssetPackLegacyV1.toc");
            if legacy.exists() {
                a.legacy_toc = legacy;
            }
            let _ = std::fs::create_dir_all(&a.out_dir);
            let rc = crate::bake::run_export(&a).unwrap_or(-1);
            let _ = std::fs::remove_dir_all(&a.out_dir);
            let done_ok = rc == 0 && png.exists();
            if !done_ok {
                *status = format!("render-failed({rc})");
            }
            done_ok
        };
        let bytes = closet.read_item_bytes(&item.id).unwrap_or_default();
        if bytes.is_empty() {
            status = "unreadable".into();
        } else if item.categories & category::PROP != 0 {
            kind = "prop";
            props += 1;
            match render_prop_icon(&bytes, size, yaw, pitch, &png, &mut choice) {
                Ok(()) => good = true,
                Err(s) => status = s.into(),
            }
            if !good && status == "empty-render" {
                kind = "performed";
                good = render_worn(&mut status);
                if good {
                    status = "ok(avatar performing)".into();
                }
            }
        } else {
            worn += 1;
            good = render_worn(&mut status);
        }
        let ms = t1.elapsed().as_millis();
        if good {
            ok += 1;
            if !generated.contains(&guid) {
                if let Some(g) = gen.as_mut() {
                    let _ = writeln!(
                        g,
                        "{guid}\t{}\t{kind}\t{}\t{}",
                        item.name, choice.frame, choice.frames
                    );
                    let _ = g.flush();
                }
                generated.insert(guid.clone());
            }
        } else {
            failed += 1;
            let _ = std::fs::remove_file(&png);
        }
        if let Some(f) = csv.as_mut() {
            writeln!(
                f,
                "{guid}\t{}\t{kind}\t{status}\t{}\t{}\t{:.4}\t{:.4}\t{ms}",
                item.name,
                choice.frame,
                choice.frames,
                f64::from(choice.motion),
                f64::from(choice.extent)
            )?;
        }
        if done % 100 == 0 {
            println!("  {done} done ({ok} ok, {failed} failed) ...");
        }
    }
    drop(gen);
    let _ = std::fs::remove_dir_all(&work);
    println!(
        "gen-icons: {ok} rendered ({props} props, {worn} worn), {failed} failed, {present} already had icons, in {} ms -> {}",
        t0.elapsed().as_millis(),
        icons_out.display()
    );
    Ok(0)
}
