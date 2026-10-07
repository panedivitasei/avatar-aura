//! Ports Run from avatar_export.cpp: manifest, packs and closet, resolve, the per-batch material bake
//! (bake_component), face frames, animations, previews or the worn-item icon, then avatar.json.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;

use anyhow::Context as _;
use avatar_export::scene;
use avatar_formats::closet::is_stock_pack_id;
use avatar_formats::model::ShaderParameterType;
use avatar_formats::Closet;
use glam::Vec2;

use crate::animations::{self, AnimExport};
use crate::args::Args;
use crate::glmath::fmax;
use crate::json::{self, Info, InfoComponent, InfoFaceTexture};
use crate::manifest::{body_type_name, category, Float4, Metadata, WHITE};
use crate::preview::{
    pick_preview_frame, pose_meshes, pose_skin_matrices, render_worn_item_icon, run_previews,
};
use crate::resolve::{
    compute_overrides, load_closet, load_pack, resolve, Assets, Resolved, ResolvedComponent,
};
use crate::skin::{
    build_joint_xforms, joint_name, skin_to_rest, to_scene_mesh, unpack_skin, world_rotation, BakedMesh,
    JointXform,
};
use crate::textures::{
    bake_batch, decode_texture_layer, kind_name, shader_name, write_face_frames, write_png, BakedMaterial,
    FrameFile, HeadBatchInfo, Image, MaterialRecipe, Sampler, ShaderKind,
};
use crate::util::{category_names, sanitize_name};

const FACE_SLOT_NAMES: [&str; 6] = ["mouth", "eyes", "brows", "face_paint", "eye_shadow", "face"];
const COLOR_NAMES: [&str; 9] = [
    "skin",
    "hair",
    "mouth",
    "iris",
    "eyebrow",
    "eye_shadow",
    "facial_hair",
    "skin_feature_1",
    "skin_feature_2",
];

/// usage_to_replacement_texture_indices from ModelToGuest.
fn replacement_slot_for_usage(usage: u32) -> Option<usize> {
    const TABLE: [i32; 13] = [-1, -1, -1, -1, -1, 5, 3, 2, 2, 1, 1, 4, 0];
    TABLE.get(usage as usize).and_then(|&s| usize::try_from(s).ok())
}

/// The bake state bake_component threads through every component.
struct Baker<'a> {
    args: &'a Args,
    rep_layers: &'a [Vec<Rc<Image>>; 6],
    materials: Vec<BakedMaterial>,
    meshes: Vec<BakedMesh>,
    material_by_key: HashMap<String, usize>,
    name_counts: HashMap<String, i32>,
    head_batches: Vec<HeadBatchInfo>,
    total_verts: usize,
}

/// The decal-carrying triangles of a garment whose colour map folds both halves onto the same texels; they bake
/// in the decal UV set instead, or the chest decal comes back mirrored.
#[allow(clippy::needless_range_loop)]
fn split_decal(mesh: &mut BakedMesh, decal: &Sampler, out_uv: usize) -> Vec<u16> {
    const G: i32 = 64;
    let cell = |t: Vec2| -> usize {
        let rep = |f: f32| -> i32 {
            let v = (f * G as f32).floor() as i32 % G;
            if v < 0 {
                v + G
            } else {
                v
            }
        };
        (rep(t.y) * G + rep(t.x)) as usize
    };
    let tri_count = mesh.indices.len() / 3;
    let vert_ok = |t: usize| (0..3).all(|k| usize::from(mesh.indices[t * 3 + k]) < mesh.verts.len());
    let uv_of = |t: usize, k: usize, l: usize| mesh.verts[usize::from(mesh.indices[t * 3 + k])].uvs[l];
    let mut wants_decal = vec![false; tri_count];
    let mut plain = vec![false; (G * G) as usize];
    let mut decal_tris = 0usize;
    let d = decal.uv_layer;
    for t in 0..tri_count {
        if !vert_ok(t) {
            continue;
        }
        let (d0, d1, d2) = (uv_of(t, 0, d), uv_of(t, 1, d), uv_of(t, 2, d));
        let probe = [
            d0,
            d1,
            d2,
            (d0 + d1 + d2) / 3.0,
            (d0 + d1) * 0.5,
            (d1 + d2) * 0.5,
            (d2 + d0) * 0.5,
        ];
        let mut a = 0.0f32;
        for q in probe {
            a = fmax(a, decal.sample(q).w);
        }
        if a > 1.0 / 255.0 {
            wants_decal[t] = true;
            decal_tris += 1;
        } else {
            for k in 0..3 {
                plain[cell(uv_of(t, k, out_uv))] = true;
            }
        }
    }
    let mut folded = false;
    if decal_tris != 0 && decal_tris < tri_count {
        'tris: for t in 0..tri_count {
            if !wants_decal[t] {
                continue;
            }
            for k in 0..3 {
                if plain[cell(uv_of(t, k, out_uv))] {
                    folded = true;
                    break 'tris;
                }
            }
        }
    }
    let mut decal_indices = Vec::new();
    if folded {
        let mut plain_indices = Vec::with_capacity(mesh.indices.len());
        for t in 0..tri_count {
            let dst = if wants_decal[t] {
                &mut decal_indices
            } else {
                &mut plain_indices
            };
            dst.extend_from_slice(&mesh.indices[t * 3..t * 3 + 3]);
        }
        mesh.indices = plain_indices;
    }
    decal_indices
}

impl Baker<'_> {
    fn bake_component(
        &mut self,
        comp: &ResolvedComponent,
        comp_index: usize,
        is_prop: bool,
        skin_xf: Option<&[JointXform]>,
    ) {
        let model = &comp.model;
        let images: Vec<Rc<Image>> = model
            .textures
            .iter()
            .map(|t| Rc::new(decode_texture_layer(&t.texture, 0)))
            .collect();
        let is_head = !is_prop && u32::from(comp.info.categories) & category::HEAD != 0;
        let guid = comp.info.asset_id.to_string();

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
                    ShaderParameterType::PixelConstant | ShaderParameterType::VertexConstant => {
                        usage_const.insert(sp.usage, sp.constant_values);
                    }
                    _ => {}
                }
            }
            for (&k, &v) in &comp.overrides {
                usage_const.insert(k, v);
            }

            let head_kind = (b.shader_id == 4 || is_head) && usage_tex.contains_key(&5);
            let mut recipe = MaterialRecipe {
                shader_id: b.shader_id,
                kind: if head_kind {
                    ShaderKind::Head
                } else {
                    ShaderKind::Body
                },
                ..Default::default()
            };
            let rep_layers = self.rep_layers;
            let tex_for = |usage: u32, s: &mut Sampler| {
                let Some(&ti) = usage_tex.get(&usage) else {
                    return;
                };
                s.uv_layer = usage_uv.get(&usage).copied().unwrap_or(0).clamp(0, 5) as usize;
                s.flags = usage_flags.get(&usage).copied().unwrap_or(0);
                // Clamp-addressed face feature decals must not repeat across the head.
                s.transparent_border = (6..=12).contains(&usage) && s.flags == 0;
                let slot = if is_head {
                    replacement_slot_for_usage(usage)
                } else {
                    None
                };
                if let Some(layer) = slot.and_then(|s| rep_layers[s].first()).filter(|l| l.ok) {
                    s.img = Some(layer.clone());
                } else if let Some(img) = usize::try_from(ti)
                    .ok()
                    .and_then(|i| images.get(i))
                    .filter(|i| i.ok)
                {
                    s.img = Some(img.clone());
                }
            };
            if recipe.kind == ShaderKind::Body {
                tex_for(1, &mut recipe.color);
                tex_for(2, &mut recipe.intensity);
                tex_for(3, &mut recipe.decal);
                for k in 0..3u32 {
                    recipe.custom[k as usize] = usage_const.get(&(22 + k)).copied().unwrap_or(WHITE);
                }
                recipe.out_uv_layer = if recipe.color.img.is_some() {
                    recipe.color.uv_layer
                } else {
                    0
                };
            } else {
                tex_for(5, &mut recipe.base);
                tex_for(11, &mut recipe.eyeshadow);
                tex_for(12, &mut recipe.mouth);
                tex_for(if usage_tex.contains_key(&9) { 9 } else { 10 }, &mut recipe.eye);
                tex_for(6, &mut recipe.facial_hair);
                tex_for(if usage_tex.contains_key(&7) { 7 } else { 8 }, &mut recipe.brow);
                for k in 0..9u32 {
                    recipe.tint[k as usize] = usage_const.get(&(13 + k)).copied().unwrap_or(WHITE);
                }
                recipe.out_uv_layer = if recipe.base.img.is_some() {
                    recipe.base.uv_layer
                } else {
                    0
                };
            }

            let mut mesh = BakedMesh {
                component: comp_index as i32,
                is_prop,
                uv_count: b.uv_count,
                verts: b.vertices.iter().map(|v| unpack_skin(v, b.uv_count)).collect(),
                indices: b.indices.clone(),
                ..Default::default()
            };

            let decal_layer = recipe.decal.clone();
            let mut decal_indices: Vec<u16> = Vec::new();
            if recipe.kind == ShaderKind::Body
                && decal_layer.img.as_ref().is_some_and(|i| i.ok)
                && decal_layer.uv_layer != recipe.out_uv_layer
                && mesh.indices.len() >= 3
            {
                decal_indices = split_decal(&mut mesh, &decal_layer, recipe.out_uv_layer);
                if !decal_indices.is_empty() {
                    println!(
                        "  {}: {} decal triangles baked in uv{}, uv{} folds over itself",
                        comp.slot,
                        decal_indices.len() / 3,
                        decal_layer.uv_layer,
                        recipe.out_uv_layer
                    );
                }
            }

            let batch_verts = if decal_indices.is_empty() {
                Vec::new()
            } else {
                mesh.verts.clone()
            };
            let parts = if decal_indices.is_empty() { 1 } else { 2 };
            for part in 0..parts {
                if part == 1 {
                    recipe.out_uv_layer = decal_layer.uv_layer;
                    mesh = BakedMesh {
                        component: comp_index as i32,
                        is_prop,
                        uv_count: b.uv_count,
                        ..Default::default()
                    };
                    let mut remap = vec![-1i32; batch_verts.len()];
                    mesh.indices.reserve(decal_indices.len());
                    for &vi in &decal_indices {
                        let vi = usize::from(vi);
                        if remap[vi] < 0 {
                            remap[vi] = mesh.verts.len() as i32;
                            mesh.verts.push(batch_verts[vi]);
                        }
                        mesh.indices.push(remap[vi] as u16);
                    }
                }

                // Material dedup by textures and constants within the component.
                let mut key = format!("{guid}|{}{}", b.shader_id, if part == 1 { "|decal" } else { "" });
                // A bake across UV sets is rasterized from this batch's own triangles.
                if !recipe.single_uv_set() {
                    key += &format!("|b{bi}");
                }
                for (u, t) in &usage_tex {
                    key += &format!("|t{u}={t}/{}", usage_uv.get(u).copied().unwrap_or(0));
                }
                for (u, c) in &usage_const {
                    key += &format!(
                        "|c{u}={:.3},{:.3},{:.3},{:.3}",
                        f64::from(c[0]),
                        f64::from(c[1]),
                        f64::from(c[2]),
                        f64::from(c[3])
                    );
                }
                let mat_index = match self.material_by_key.get(&key) {
                    Some(&i) => i,
                    None => {
                        let baked = bake_batch(
                            &mut recipe,
                            &mut mesh.verts,
                            &mut mesh.indices,
                            self.args.bake_size,
                            Some(&comp.slot),
                        );
                        let mut base_name = if is_prop {
                            "Prop".to_owned()
                        } else {
                            comp.slot.clone()
                        };
                        if comp.from_closet && !comp.name.is_empty() && !is_prop {
                            base_name = format!("{}_{}", comp.slot, sanitize_name(&comp.name));
                        }
                        let counter = self.name_counts.entry(base_name.clone()).or_insert(0);
                        let n = *counter;
                        *counter += 1;
                        let name = if n == 0 {
                            base_name
                        } else {
                            format!("{base_name}_{n}")
                        };
                        let file = format!("{name}_Diffuse.png");
                        let has_alpha = baked.rgba.iter().skip(3).step_by(4).any(|&a| a < 128);
                        if !write_png(&self.args.out_dir.join(&file), &baked) {
                            println!("  WARN: failed to write {file}");
                        }
                        let mut flag_desc = String::new();
                        for (u, f) in &usage_flags {
                            flag_desc += &format!(
                                " u{u}:t{}/uv{}/f{f}",
                                usage_tex.get(u).copied().unwrap_or(0),
                                usage_uv.get(u).copied().unwrap_or(0)
                            );
                        }
                        let cdesc = if recipe.kind == ShaderKind::Body {
                            let c = &recipe.custom;
                            let p = |v: f32| format!("{:.2}", f64::from(v));
                            format!(
                                " C0={},{},{} C1={},{},{} C2={},{},{}",
                                p(c[0][0]),
                                p(c[0][1]),
                                p(c[0][2]),
                                p(c[1][0]),
                                p(c[1][1]),
                                p(c[1][2]),
                                p(c[2][0]),
                                p(c[2][1]),
                                p(c[2][2])
                            )
                        } else {
                            String::new()
                        };
                        println!(
                            "  material {:<28} {} {}x{} uv{}{}  [{} ]{}",
                            name,
                            shader_name(b.shader_id),
                            baked.width,
                            baked.height,
                            recipe.out_uv_layer,
                            if has_alpha { " alpha" } else { "" },
                            flag_desc,
                            cdesc
                        );
                        let index = self.materials.len();
                        self.materials.push(BakedMaterial {
                            name,
                            file,
                            image: Rc::new(baked),
                            shader_id: b.shader_id,
                            has_alpha,
                            uv_layer: recipe.out_uv_layer,
                            component_guid: guid.clone(),
                        });
                        self.material_by_key.insert(key, index);
                        index
                    }
                };
                mesh.material = mat_index as i32;
                mesh.name = format!(
                    "{}_b{bi}{}",
                    self.materials[mat_index].name,
                    if part == 1 { "d" } else { "" }
                );

                if let Some(xf) = skin_xf {
                    skin_to_rest(&mut mesh.verts, xf);
                    self.total_verts += mesh.verts.len();
                }
                if is_head && recipe.kind == ShaderKind::Head {
                    self.head_batches.push(HeadBatchInfo {
                        mesh_index: self.meshes.len(),
                        recipe: recipe.clone(),
                        material_index: mat_index,
                        eye_usage: if usage_tex.contains_key(&9) { 9 } else { 10 },
                        brow_usage: if usage_tex.contains_key(&7) { 7 } else { 8 },
                    });
                }
                self.meshes.push(std::mem::take(&mut mesh));
            }
        }
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn print_info(args: &Args, metadata: &Metadata, res: &Resolved, assets: &Assets) {
    let info = Info {
        manifest: path_str(&args.manifest),
        body_type: body_type_name(res.body_type).into(),
        height_factor: metadata.height_factor,
        weight_factor: metadata.weight_factor,
        colors: metadata.colors.iter().map(|c| format!("{c:08X}")).collect(),
        components: res
            .components
            .iter()
            .map(|c| InfoComponent {
                slot: c.slot.clone(),
                guid: c.info.asset_id.to_string(),
                name: c.name.clone(),
                categories: u32::from(c.info.categories),
                category_names: category_names(u32::from(c.info.categories)),
                source: if c.from_closet { "closet" } else { "pack" }.into(),
                batches: c.model.triangle_batches.len(),
            })
            .collect(),
        face_textures: (0..6)
            .filter_map(|s| {
                res.replacement_textures[s].as_ref().map(|t| InfoFaceTexture {
                    slot: FACE_SLOT_NAMES[s].into(),
                    guid: res.replacement_ids[s].to_string(),
                    name: assets.name(&res.replacement_ids[s]),
                    layers: t.layer_count,
                })
            })
            .collect(),
        prop: res
            .prop
            .as_ref()
            .map(|_| (res.prop_info.asset_id.to_string(), res.prop_name.clone())),
    };
    print!("{}", json::info_to_json(&info));
    println!();
}

/// Everything avatar.json carries, gathered after the bake.
struct SceneParts<'a> {
    args: &'a Args,
    metadata: &'a Metadata,
    res: &'a Resolved,
    assets: &'a Assets,
    xforms: &'a [JointXform],
    prop_xforms: &'a [JointXform],
    materials: &'a [BakedMaterial],
    meshes: &'a [BakedMesh],
    head_batches: &'a [HeadBatchInfo],
    layer_files: &'a [FrameFile],
    composite_files: &'a [FrameFile],
    anims: &'a [AnimExport],
}

fn v3(v: glam::Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn build_scene(p: &SceneParts<'_>) -> scene::Scene {
    let (args, res, metadata) = (p.args, p.res, p.metadata);
    let joint_count = res.skeleton.joints.len();
    let mut s = scene::Scene {
        format: scene::FORMAT.into(),
        version: scene::VERSION,
        source: scene::Source {
            manifest: path_str(&args.manifest),
            pack: path_str(&args.toc),
            closet: path_str(&args.closet),
        },
        axes: scene::Axes::default(),
        avatar: scene::AvatarInfo {
            body_type: body_type_name(res.body_type).into(),
            height_factor: metadata.height_factor,
            weight_factor: metadata.weight_factor,
            scale_applied: !args.no_scale,
            colors: metadata.colors.iter().map(|c| format!("{c:08X}")).collect(),
            color_names: COLOR_NAMES.iter().map(|s| (*s).into()).collect(),
        },
        ..Default::default()
    };
    s.skeleton.version = args.skeleton_version as u32;
    for (i, (jt, x)) in res.skeleton.joints.iter().zip(p.xforms.iter()).enumerate() {
        let rq = world_rotation(&x.rest_world);
        s.skeleton.joints.push(scene::Joint {
            name: joint_name(i, joint_count),
            parent: if jt.parent_index == 255 {
                -1
            } else {
                i32::from(jt.parent_index)
            },
            bind_world: v3(x.bind_world.col3(3)),
            rest_world: v3(x.rest_world.col3(3)),
            rest_world_rot: [rq.x, rq.y, rq.z, rq.w],
            rest_local: v3(x.rest_local_t * x.parent_world_scale),
            scale: v3(x.scale),
        });
    }
    for (ci, c) in res.components.iter().enumerate() {
        s.components.push(scene::Component {
            index: ci as u32,
            slot: c.slot.clone(),
            guid: c.info.asset_id.to_string(),
            name: c.name.clone(),
            categories: u32::from(c.info.categories),
            category_names: category_names(u32::from(c.info.categories)),
            source: if c.from_closet { "closet" } else { "pack" }.into(),
            notes: c.notes.clone(),
        });
    }
    if res.prop.is_some() {
        s.components.push(scene::Component {
            index: res.components.len() as u32,
            slot: "Prop".into(),
            guid: res.prop_info.asset_id.to_string(),
            name: res.prop_name.clone(),
            categories: u32::from(res.prop_info.categories),
            category_names: "Prop".into(),
            source: if is_stock_pack_id(&res.prop_info.asset_id) {
                "pack"
            } else {
                "closet"
            }
            .into(),
            notes: Vec::new(),
        });
    }
    for m in p.materials {
        s.materials.push(scene::Material {
            name: m.name.clone(),
            diffuse: m.file.clone(),
            shader: m.shader_id,
            shader_name: shader_name(m.shader_id).into(),
            has_alpha: m.has_alpha,
            alpha_mask: m.has_alpha,
            double_sided: false,
            uv_layer: m.uv_layer as u32,
            component_guid: m.component_guid.clone(),
        });
    }
    for m in p.meshes {
        let out_uv = usize::try_from(m.material)
            .ok()
            .and_then(|i| p.materials.get(i))
            .map_or(0, |mat| mat.uv_layer);
        s.meshes.push(to_scene_mesh(m, out_uv));
    }
    if let Some(prop) = res.prop.as_ref().filter(|_| !p.prop_xforms.is_empty()) {
        let mut sk = scene::Skeleton::default();
        for (i, (jt, x)) in prop.skeleton.joints.iter().zip(p.prop_xforms.iter()).enumerate() {
            let rq = world_rotation(&x.rest_world);
            sk.joints.push(scene::Joint {
                name: format!("prop_joint_{i}"),
                parent: if jt.parent_index == 255 {
                    -1
                } else {
                    i32::from(jt.parent_index)
                },
                bind_world: [0.0; 3],
                rest_world: v3(x.rest_world.col3(3)),
                rest_world_rot: [rq.x, rq.y, rq.z, rq.w],
                rest_local: v3(x.rest_local_t),
                scale: [1.0; 3],
            });
        }
        s.prop_skeleton = Some(sk);
    }
    let mut face = scene::Face::default();
    let mut seen = std::collections::HashSet::new();
    for hb in p.head_batches {
        if seen.insert(hb.material_index) {
            face.head_materials
                .push(p.materials[hb.material_index].name.clone());
        }
    }
    for (slot, tex) in res.replacement_textures.iter().enumerate() {
        let Some(tex) = tex else {
            continue;
        };
        face.slots.insert(
            FACE_SLOT_NAMES[slot].into(),
            scene::FaceSlot {
                guid: res.replacement_ids[slot].to_string(),
                name: p.assets.name(&res.replacement_ids[slot]),
                layers: tex.layer_count,
                width: tex.width,
                height: tex.height,
            },
        );
    }
    let files = |v: &[FrameFile]| -> Vec<scene::FaceFile> {
        v.iter()
            .map(|f| scene::FaceFile {
                channel: f.channel.clone(),
                frame: f.frame,
                file: f.file.clone(),
            })
            .collect()
    };
    face.layer_files = files(p.layer_files);
    face.composite_files = files(p.composite_files);
    let names = |v: &[&str]| -> Vec<String> { v.iter().map(|s| (*s).into()).collect() };
    face.layer_names.insert(
        "eyes".into(),
        names(&[
            "NEUTRAL",
            "SAD",
            "ANGRY",
            "CONFUSED",
            "LAUGHING",
            "SHOCKED",
            "HAPPY",
            "YAWNING",
            "SLEEPING",
            "LOOK_UP",
            "LOOK_DOWN",
            "LOOK_OUTER",
            "LOOK_INNER",
            "BLINK",
        ]),
    );
    face.layer_names.insert(
        "mouth".into(),
        names(&[
            "NEUTRAL",
            "SAD",
            "ANGRY",
            "CONFUSED",
            "LAUGHING",
            "SHOCKED",
            "HAPPY",
            "PHONETIC_O",
            "PHONETIC_AI",
            "PHONETIC_EE",
            "PHONETIC_FV",
            "PHONETIC_W",
            "PHONETIC_L",
            "PHONETIC_DTH",
        ]),
    );
    face.layer_names.insert(
        "brows".into(),
        names(&["NEUTRAL", "SAD", "ANGRY", "CONFUSED", "RAISED"]),
    );
    s.face = Some(face);
    s.animations = p
        .anims
        .iter()
        .map(|a| animations::to_scene(a, p.xforms))
        .collect();
    s.log = res.log.clone();
    s
}

/// Run: the `--avatar` export (or `--avatar-info`, or a --gen-icons worn-item render). Returns the C++ exit code:
/// 1 bad manifest, 2 asset pack, 3 skeleton, 4 avatar.json write, 5 icon render.
pub fn run_export(args: &Args) -> anyhow::Result<i32> {
    println!("(mode: avatar export)\n");
    let metadata = match std::fs::read(&args.manifest).ok().map(|b| Metadata::parse(&b)) {
        Some(Ok(m)) => m,
        _ => {
            println!(
                "ERROR: manifest {} is not a 1000-byte X_AVATAR_METADATA",
                args.manifest.display()
            );
            return Ok(1);
        }
    };
    let Some(pack) = load_pack(&args.toc) else {
        println!("ERROR: cannot load asset pack {}", args.toc.display());
        return Ok(2);
    };
    println!(
        "asset pack: {} ({} assets)",
        args.toc.display(),
        pack.asset_infos().len()
    );
    let mut legacy_pack = None;
    if !args.legacy_toc.as_os_str().is_empty() && args.legacy_toc.exists() {
        legacy_pack = load_pack(&args.legacy_toc);
        println!(
            "legacy pack: {} ({})",
            args.legacy_toc.display(),
            if legacy_pack.is_some() { "loaded" } else { "FAILED" }
        );
    }
    let closet = if args.closet.as_os_str().is_empty() {
        Closet::default()
    } else {
        let c = load_closet(&args.closet);
        println!("closet: {} ({} items)", args.closet.display(), c.items().len());
        c
    };
    let assets = Assets {
        pack,
        legacy_pack,
        closet,
    };

    if args.list_anims {
        println!("=== pack animations ===");
        for (i, info) in assets.pack.asset_infos().iter().enumerate() {
            if info.categories & category::ANIMATION != 0 {
                println!("  {:<5} {}", i, assets.pack.asset_name_by_index(i));
            }
        }
        return Ok(0);
    }

    println!("\n=== resolving ===");
    let Some(res) = resolve(&metadata, args, &assets) else {
        return Ok(3);
    };
    println!(
        "  body type: {}  height {:.3}  weight {:.3}",
        body_type_name(res.body_type),
        f64::from(metadata.height_factor),
        f64::from(metadata.weight_factor)
    );
    for c in &res.components {
        println!(
            "  [{}] {} '{}' ({}, {}) batches={} textures={}",
            c.slot,
            c.info.asset_id,
            c.name,
            category_names(u32::from(c.info.categories)),
            if c.from_closet { "closet" } else { "pack" },
            c.model.triangle_batches.len(),
            c.model.textures.len()
        );
    }

    if args.info_only {
        print_info(args, &metadata, &res, &assets);
        return Ok(0);
    }

    std::fs::create_dir_all(&args.out_dir).with_context(|| format!("creating {}", args.out_dir.display()))?;

    let xforms = build_joint_xforms(&res.skeleton, !args.no_scale);
    println!(
        "\n=== skeleton v{}: {} joints, scale {} ===",
        args.skeleton_version,
        res.skeleton.joints.len(),
        if args.no_scale { "off" } else { "on" }
    );

    println!("\n=== baking ===");
    let mut rep_layers: [Vec<Rc<Image>>; 6] = Default::default();
    for (s, tex) in res.replacement_textures.iter().enumerate() {
        let Some(t) = tex else {
            continue;
        };
        rep_layers[s] = (0..t.layer_count)
            .map(|l| Rc::new(decode_texture_layer(t, l)))
            .collect();
        println!(
            "  face slot {s}: {}x{} {} x{} layers",
            t.width,
            t.height,
            kind_name(t.format),
            t.layer_count
        );
    }

    let mut baker = Baker {
        args,
        rep_layers: &rep_layers,
        materials: Vec::new(),
        meshes: Vec::new(),
        material_by_key: HashMap::new(),
        name_counts: HashMap::new(),
        head_batches: Vec::new(),
        total_verts: 0,
    };
    for (ci, comp) in res.components.iter().enumerate() {
        baker.bake_component(comp, ci, false, Some(&xforms));
    }
    // The carryable bakes on its own skeleton.
    let mut prop_xforms: Vec<JointXform> = Vec::new();
    if let Some(prop) = &res.prop {
        prop_xforms = build_joint_xforms(&prop.skeleton, false);
        let mut pc = ResolvedComponent {
            info: res.prop_info,
            model: prop.model.clone(),
            name: res.prop_name.clone(),
            slot: "Prop".into(),
            from_closet: !is_stock_pack_id(&res.prop_info.asset_id),
            ..Default::default()
        };
        pc.info.categories = category::PROP as u16;
        // White CUSTOM_0 unless the closet colour table says otherwise.
        compute_overrides(&metadata, &mut pc, &assets.closet);
        let xf = if prop_xforms.is_empty() {
            None
        } else {
            Some(prop_xforms.as_slice())
        };
        baker.bake_component(&pc, res.components.len(), true, xf);
    }
    println!("  {} vertices skinned", baker.total_verts);
    let Baker {
        materials,
        meshes,
        head_batches,
        ..
    } = baker;

    let face_dir = args.out_dir.join("face");
    std::fs::create_dir_all(&face_dir).with_context(|| format!("creating {}", face_dir.display()))?;
    let (layer_files, composite_files) = write_face_frames(
        &args.out_dir,
        &metadata.colors,
        &rep_layers,
        if args.face_frames { &head_batches } else { &[] },
        &meshes,
        &materials,
        args.face_frames.then_some(args.face_frame_size),
    );
    println!(
        "  face layers written: {}, composites: {}",
        layer_files.len(),
        composite_files.len()
    );

    let anims = animations::collect(args, &assets, &res);
    if !anims.is_empty() {
        println!("\n=== animations: {} ===", anims.len());
    }

    // Icon mode (--gen-icons): one worn-item icon, no exports.
    if !args.icon_out.as_os_str().is_empty() {
        let clip = anims
            .iter()
            .find(|a| a.anim.pose_counts[0] > 0 && !a.anim.pose_frame_sets[0].frames.is_empty());
        let perform = res
            .prop
            .as_ref()
            .and_then(|p| p.animation.as_deref())
            .filter(|a| a.pose_counts[0] > 0 && !a.pose_frame_sets[0].frames.is_empty());
        let skin = if let Some(perform) = perform {
            // The avatar performing the prop's own clip at its most expressive frame.
            let f = pick_preview_frame(perform, "peak");
            pose_skin_matrices(&xforms, &res.skeleton, Some(perform), f, 0)
        } else if let Some(clip) = clip {
            let rule = if clip.preview_frame.is_empty() {
                &args.preview_frame
            } else {
                &clip.preview_frame
            };
            let f = pick_preview_frame(&clip.anim, rule);
            pose_skin_matrices(&xforms, &res.skeleton, Some(&clip.anim), f, 0)
        } else {
            pose_skin_matrices(&xforms, &res.skeleton, None, 0, 0)
        };
        let posed = pose_meshes(&meshes, &skin);
        return Ok(if render_worn_item_icon(&meshes, &materials, posed, &res, args) {
            0
        } else {
            5
        });
    }

    if !args.preview_dir.as_os_str().is_empty() {
        run_previews(
            args,
            &meshes,
            &materials,
            &xforms,
            &res.skeleton,
            &anims,
            &head_batches,
            &rep_layers,
        )
        .with_context(|| format!("creating {}", args.preview_dir.display()))?;
    }

    let json_path = args.out_dir.join("avatar.json");
    let scene = build_scene(&SceneParts {
        args,
        metadata: &metadata,
        res: &res,
        assets: &assets,
        xforms: &xforms,
        prop_xforms: &prop_xforms,
        materials: &materials,
        meshes: &meshes,
        head_batches: &head_batches,
        layer_files: &layer_files,
        composite_files: &composite_files,
        anims: &anims,
    });
    if std::fs::write(&json_path, json::scene_to_json(&scene)).is_err() {
        println!("ERROR: cannot write {}", json_path.display());
        return Ok(4);
    }
    println!(
        "\n=== Done ===\n  {}\n  {} meshes, {} materials, {} animations",
        json_path.display(),
        meshes.len(),
        materials.len(),
        anims.len()
    );
    Ok(0)
}
