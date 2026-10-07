// Port of avatar_aura/ae_convert.py `write_glb` and `_BinBuilder`: glTF 2.0 binary, skinned, clips on the joints.
// glTF is Y-up right-handed with +Z forward, the avatar's own frame, so the rig is FRAME_XBOX at scale 1.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::avatar::Avatar;
use crate::dae::{copy_file, create_dir, parent_dir};
use crate::error::{bad_scene, io_err, Result};
use crate::math::{mat_inverse, mat_rot3, mat_trans, psum, r3_normalize_columns, r3_to_quat, safe_id, vlen};
use crate::rig::{check_parents, clip_frame_locals, Rig};

const FLOAT: u32 = 5126;
const USHORT: u32 = 5123;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

#[derive(Clone, Debug)]
pub struct GlbOptions {
    pub include_animations: bool,
    pub include_colors: bool,
    /// PNGs inside the binary chunk; otherwise copied beside the .glb.
    pub embed_textures: bool,
    pub mask_alpha: bool,
}

impl Default for GlbOptions {
    fn default() -> Self {
        Self {
            include_animations: true,
            include_colors: true,
            embed_textures: true,
            mask_alpha: true,
        }
    }
}

/// One buffer, views padded to four bytes.
#[derive(Default)]
struct BinBuilder {
    blob: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl BinBuilder {
    fn add(&mut self, data: &[u8], target: Option<u32>) -> usize {
        let offset = self.blob.len();
        self.blob.extend_from_slice(data);
        let pad = (4 - data.len() % 4) % 4;
        self.blob.extend(std::iter::repeat_n(0u8, pad));
        let mut view = Map::new();
        view.insert("buffer".into(), json!(0));
        view.insert("byteOffset".into(), json!(offset));
        view.insert("byteLength".into(), json!(data.len()));
        if let Some(t) = target {
            view.insert("target".into(), json!(t));
        }
        self.views.push(Value::Object(view));
        self.views.len() - 1
    }

    fn accessor(
        &mut self,
        data: &[u8],
        comp_type: u32,
        count: usize,
        atype: &str,
        target: Option<u32>,
        minmax: Option<(Vec<f64>, Vec<f64>)>,
    ) -> usize {
        let vi = self.add(data, target);
        let mut acc = Map::new();
        acc.insert("bufferView".into(), json!(vi));
        acc.insert("componentType".into(), json!(comp_type));
        acc.insert("count".into(), json!(count));
        acc.insert("type".into(), json!(atype));
        if let Some((min, max)) = minmax {
            acc.insert("min".into(), json!(min));
            acc.insert("max".into(), json!(max));
        }
        self.accessors.push(Value::Object(acc));
        self.accessors.len() - 1
    }
}

fn f32s(values: impl IntoIterator<Item = f64>) -> Vec<u8> {
    values
        .into_iter()
        .flat_map(|v| (v as f32).to_le_bytes())
        .collect()
}

/// Writes `out_path` and returns it.
pub fn write_glb(avatar: &Avatar, rig: &Rig, out_path: &Path, opts: &GlbOptions) -> Result<PathBuf> {
    check_parents(rig)?;
    let out_dir = parent_dir(out_path);
    create_dir(&out_dir)?;
    let mut bb = BinBuilder::default();
    let mut nodes: Vec<Map<String, Value>> = Vec::new();
    let mut meshes: Vec<Value> = Vec::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    let mut scene_nodes: Vec<usize> = Vec::new();
    let mut animations: Vec<Value> = Vec::new();

    // materials and images
    let mut mat_index: BTreeMap<usize, usize> = BTreeMap::new();
    for (mi, md) in avatar.materials.iter().enumerate() {
        let name = safe_id(&md.name);
        let src = avatar.file(&md.diffuse);
        let mut has_image = false;
        if src.is_file() {
            if opts.embed_textures {
                let png = std::fs::read(&src).map_err(io_err(&src))?;
                let vi = bb.add(&png, None);
                images.push(json!({"bufferView": vi, "mimeType": "image/png", "name": name}));
            } else {
                let dst = format!("{name}_Diffuse.png");
                copy_file(&src, &out_dir.join(&dst))?;
                images.push(json!({"uri": dst, "name": name}));
            }
            textures.push(json!({"sampler": 0, "source": images.len() - 1, "name": name}));
            has_image = true;
        }
        let mut pbr = Map::new();
        pbr.insert("metallicFactor".into(), json!(0.0));
        pbr.insert("roughnessFactor".into(), json!(0.9));
        if has_image {
            pbr.insert("baseColorTexture".into(), json!({"index": textures.len() - 1}));
        }
        let mut mat = Map::new();
        mat.insert("name".into(), json!(name));
        mat.insert("pbrMetallicRoughness".into(), Value::Object(pbr));
        mat.insert("doubleSided".into(), json!(md.double_sided));
        if opts.mask_alpha && md.alpha_mask {
            mat.insert("alphaMode".into(), json!("MASK"));
            mat.insert("alphaCutoff".into(), json!(0.5));
        }
        materials.push(Value::Object(mat));
        mat_index.insert(mi, materials.len() - 1);
    }

    // skeleton nodes
    let mut joint_nodes: Vec<usize> = Vec::with_capacity(rig.bones.len());
    for b in &rig.bones {
        let t = mat_trans(&b.local);
        let q = r3_to_quat(&r3_normalize_columns(&mat_rot3(&b.local)));
        let mut node = Map::new();
        node.insert("name".into(), json!(b.name));
        node.insert("translation".into(), json!(t));
        node.insert("rotation".into(), json!(q));
        nodes.push(node);
        joint_nodes.push(nodes.len() - 1);
    }
    for (i, b) in rig.bones.iter().enumerate() {
        if let Ok(p) = usize::try_from(b.parent) {
            let children = nodes[joint_nodes[p]]
                .entry("children")
                .or_insert_with(|| json!([]));
            if let Value::Array(a) = children {
                a.push(json!(joint_nodes[i]));
            }
        }
    }
    let roots: Vec<usize> = rig
        .bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.parent < 0)
        .map(|(i, _)| joint_nodes[i])
        .collect();
    let root = *roots
        .first()
        .ok_or_else(|| bad_scene("the rig has no root bone"))?;
    let ibm = f32s(rig.bones.iter().flat_map(|b| {
        let inv = mat_inverse(&b.world);
        (0..16).map(move |k| inv[k % 4][k / 4])
    }));
    let ibm_acc = bb.accessor(&ibm, FLOAT, rig.bones.len(), "MAT4", None, None);
    let skin = json!({"name": "Armature", "joints": joint_nodes, "inverseBindMatrices": ibm_acc,
                      "skeleton": root});
    let mut armature = Map::new();
    armature.insert("name".into(), json!("Armature"));
    armature.insert("children".into(), json!(roots));
    nodes.push(armature);
    scene_nodes.push(nodes.len() - 1);

    // meshes
    for mesh in &avatar.meshes {
        let n = mesh.positions.len();
        if mesh
            .indices
            .iter()
            .any(|&i| i as usize >= n || i > u32::from(u16::MAX))
        {
            return Err(bad_scene(format!(
                "mesh {}: index out of range for 16-bit indices",
                mesh.name
            )));
        }
        let pos: Vec<_> = mesh.positions.iter().map(|p| rig.xform_point(p)).collect();
        let nrm: Vec<_> = mesh.normals.iter().map(|v| rig.xform_dir(v)).collect();
        let minmax = (!pos.is_empty()).then(|| {
            let mut pmin = pos[0];
            let mut pmax = pos[0];
            for p in &pos[1..] {
                for i in 0..3 {
                    if p[i] < pmin[i] {
                        pmin[i] = p[i];
                    }
                    if p[i] > pmax[i] {
                        pmax[i] = p[i];
                    }
                }
            }
            (pmin.to_vec(), pmax.to_vec())
        });
        let pos_acc = bb.accessor(
            &f32s(pos.iter().flatten().copied()),
            FLOAT,
            n,
            "VEC3",
            Some(ARRAY_BUFFER),
            minmax,
        );
        let nrm_acc = bb.accessor(
            &f32s(nrm.iter().flatten().copied()),
            FLOAT,
            n,
            "VEC3",
            Some(ARRAY_BUFFER),
            None,
        );
        let uv_acc = bb.accessor(
            &f32s(mesh.uvs.iter().flatten().copied()),
            FLOAT,
            n,
            "VEC2",
            Some(ARRAY_BUFFER),
            None,
        );
        let mut jb = Vec::with_capacity(n * 8);
        let mut wb = Vec::with_capacity(n * 16);
        for (ji, wi) in mesh.joints.iter().zip(&mesh.weights) {
            let mut infl = rig.vertex_influences(ji, wi);
            infl.truncate(4);
            infl.resize(4, (0, 0.0));
            for (b, w) in infl {
                let b = u16::try_from(b).map_err(|_| bad_scene("joint index above 65535"))?;
                jb.extend_from_slice(&b.to_le_bytes());
                wb.extend_from_slice(&(w as f32).to_le_bytes());
            }
        }
        let j_acc = bb.accessor(&jb, USHORT, n, "VEC4", Some(ARRAY_BUFFER), None);
        let w_acc = bb.accessor(&wb, FLOAT, n, "VEC4", Some(ARRAY_BUFFER), None);
        let mut attrs = Map::new();
        attrs.insert("POSITION".into(), json!(pos_acc));
        attrs.insert("NORMAL".into(), json!(nrm_acc));
        attrs.insert("TEXCOORD_0".into(), json!(uv_acc));
        attrs.insert("JOINTS_0".into(), json!(j_acc));
        attrs.insert("WEIGHTS_0".into(), json!(w_acc));
        if let Some(colors) = mesh.colors.as_ref().filter(|_| opts.include_colors) {
            let cb = f32s(colors.iter().flatten().copied());
            let acc = bb.accessor(&cb, FLOAT, n, "VEC4", Some(ARRAY_BUFFER), None);
            attrs.insert("COLOR_0".into(), json!(acc));
        }
        let ib: Vec<u8> = mesh
            .indices
            .iter()
            .flat_map(|&i| (i as u16).to_le_bytes())
            .collect();
        let idx_acc = bb.accessor(
            &ib,
            USHORT,
            mesh.indices.len(),
            "SCALAR",
            Some(ELEMENT_ARRAY_BUFFER),
            None,
        );
        let name = safe_id(&mesh.name);
        meshes.push(json!({"name": name, "primitives": [
            {"attributes": attrs, "indices": idx_acc, "material": mat_index[&mesh.material], "mode": 4}]}));
        let mut node = Map::new();
        node.insert("name".into(), json!(name));
        node.insert("mesh".into(), json!(meshes.len() - 1));
        node.insert("skin".into(), json!(0));
        nodes.push(node);
        scene_nodes.push(nodes.len() - 1);
    }

    // animations
    if opts.include_animations {
        for anim in &avatar.animations {
            let frames = anim.frame_count as usize;
            let fps = anim.rate();
            if frames == 0 {
                continue;
            }
            let times: Vec<f64> = (0..frames).map(|f| f as f64 / fps).collect();
            let nb = rig.bones.len();
            let mut per_t = vec![Vec::with_capacity(frames); nb];
            let mut per_r = vec![Vec::with_capacity(frames); nb];
            let mut per_s = vec![Vec::with_capacity(frames); nb];
            for f in 0..frames {
                for (bi, m) in clip_frame_locals(avatar, rig, anim, f)?.iter().enumerate() {
                    let r = mat_rot3(m);
                    let sc = [0, 1, 2].map(|c| vlen(&[r[0][c], r[1][c], r[2][c]]));
                    let rn = [0, 1, 2]
                        .map(|i| [0, 1, 2].map(|c| r[i][c] / if sc[c] == 0.0 { 1.0 } else { sc[c] }));
                    per_t[bi].push(mat_trans(m));
                    per_r[bi].push(r3_to_quat(&rn));
                    per_s[bi].push(sc);
                }
            }
            let t_acc = bb.accessor(
                &f32s(times.iter().copied()),
                FLOAT,
                frames,
                "SCALAR",
                None,
                Some((vec![times[0]], vec![times[frames - 1]])),
            );
            let mut channels = Vec::new();
            let mut samplers = Vec::new();
            for bi in 0..nb {
                let qs = &mut per_r[bi];
                for k in 1..qs.len() {
                    let dot = psum(qs[k].iter().zip(&qs[k - 1]).map(|(a, b)| a * b));
                    if dot < 0.0 {
                        qs[k] = qs[k].map(|x| -x);
                    }
                }
                let tacc = bb.accessor(
                    &f32s(per_t[bi].iter().flatten().copied()),
                    FLOAT,
                    frames,
                    "VEC3",
                    None,
                    None,
                );
                let racc = bb.accessor(
                    &f32s(per_r[bi].iter().flatten().copied()),
                    FLOAT,
                    frames,
                    "VEC4",
                    None,
                    None,
                );
                let sacc = bb.accessor(
                    &f32s(per_s[bi].iter().flatten().copied()),
                    FLOAT,
                    frames,
                    "VEC3",
                    None,
                    None,
                );
                for (path, acc) in [("translation", tacc), ("rotation", racc), ("scale", sacc)] {
                    samplers.push(json!({"input": t_acc, "output": acc, "interpolation": "LINEAR"}));
                    channels.push(json!({"sampler": samplers.len() - 1,
                                         "target": {"node": joint_nodes[bi], "path": path}}));
                }
            }
            let face = match &anim.face {
                Some(f) => serde_json::to_value(f)?,
                None => json!({}),
            };
            animations.push(json!({"name": safe_id(&anim.name), "channels": channels,
                                   "samplers": samplers, "extras": {"face": face, "fps": fps}}));
        }
    }

    let blob = std::mem::take(&mut bb.blob);
    let mut gltf = Map::new();
    gltf.insert(
        "asset".into(),
        json!({"version": "2.0", "generator": "ReXGlue Avatar Export"}),
    );
    gltf.insert("scene".into(), json!(0));
    gltf.insert(
        "scenes".into(),
        json!([{"nodes": scene_nodes, "extras": {
            "avatar": serde_json::to_value(&avatar.info)?,
            "face": serde_json::to_value(&avatar.face.slots)?,
        }}]),
    );
    gltf.insert("nodes".into(), json!(nodes));
    gltf.insert("meshes".into(), json!(meshes));
    gltf.insert("materials".into(), json!(materials));
    gltf.insert("textures".into(), json!(textures));
    gltf.insert("images".into(), json!(images));
    gltf.insert(
        "samplers".into(),
        json!([{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]),
    );
    gltf.insert("skins".into(), json!([skin]));
    gltf.insert("accessors".into(), json!(bb.accessors));
    gltf.insert("bufferViews".into(), json!(bb.views));
    gltf.insert("buffers".into(), json!([{"byteLength": blob.len()}]));
    if !animations.is_empty() {
        gltf.insert("animations".into(), json!(animations));
    }
    let mut js = serde_json::to_vec(&Value::Object(gltf))?;
    js.resize(js.len() + (4 - js.len() % 4) % 4, b' ');
    let total = 12 + 8 + js.len() + 8 + blob.len();
    let too_big = || bad_scene("glb larger than 4 GiB");
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(total).map_err(|_| too_big())?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(js.len()).map_err(|_| too_big())?.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&u32::try_from(blob.len()).map_err(|_| too_big())?.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&blob);
    std::fs::write(out_path, out).map_err(io_err(out_path))?;
    Ok(out_path.to_path_buf())
}
