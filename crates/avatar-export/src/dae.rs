// Port of avatar_aura/ae_convert.py `write_dae`: COLLADA 1.4.1, one skinned geometry per mesh plus the rig.
// Text is assembled line by line in the same order and number format as the Python writer.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::avatar::{Avatar, Clip};
use crate::error::{bad_scene, io_err, Error, Result};
use crate::math::{fmt, mat_inverse, mat_text, safe_id};
use crate::rig::{check_parents, clip_frame_locals, Rig};

#[derive(Clone, Debug)]
pub struct DaeOptions<'a> {
    /// Metres per unit; 0.01 is written as centimetres.
    pub unit_meter: f64,
    /// "Y_UP" or "Z_UP".
    pub up_axis: String,
    /// One clip baked as matrix channels on every bone.
    pub animation: Option<&'a Clip>,
    /// Folder of the `<Material>_Diffuse.png` files relative to the .dae; empty for beside it.
    pub texture_dir: String,
    pub copy_textures: bool,
    pub include_colors: bool,
    pub flip_v: bool,
    pub armature_name: String,
    pub scene_name: String,
    pub material_suffix: String,
    /// `<created>`/`<modified>` stamp; the current UTC time when `None`.
    pub timestamp: Option<String>,
}

impl Default for DaeOptions<'_> {
    fn default() -> Self {
        Self {
            unit_meter: 1.0,
            up_axis: "Y_UP".into(),
            animation: None,
            texture_dir: String::new(),
            copy_textures: true,
            include_colors: true,
            flip_v: true,
            armature_name: "Armature".into(),
            scene_name: "Scene".into(),
            material_suffix: String::new(),
            timestamp: None,
        }
    }
}

/// `xml.sax.saxutils.escape`: ampersand, less-than, greater-than.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// ISO-8601 UTC seconds, `%Y-%m-%dT%H:%M:%S`.
pub fn utc_now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(crate) fn abs_path(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

pub(crate) fn parent_dir(out_path: &Path) -> PathBuf {
    abs_path(out_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

pub(crate) fn create_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))
}

/// Copies unless source and destination are the same file.
pub(crate) fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if abs_path(src) == abs_path(dst) {
        return Ok(());
    }
    std::fs::copy(src, dst).map_err(io_err(dst))?;
    Ok(())
}

pub(crate) fn write_lines(out_path: &Path, lines: &[String]) -> Result<()> {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(out_path, text).map_err(io_err(out_path))
}

fn join_fmt(values: impl IntoIterator<Item = f64>, nd: usize) -> String {
    values
        .into_iter()
        .map(|v| fmt(v, nd))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Material indices referenced by meshes, ascending.
pub(crate) fn sorted_used_materials(avatar: &Avatar) -> Vec<usize> {
    avatar
        .meshes
        .iter()
        .map(|m| m.material)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Writes `out_path` and returns it.
pub fn write_dae(avatar: &Avatar, rig: &Rig, out_path: &Path, opts: &DaeOptions) -> Result<PathBuf> {
    check_parents(rig)?;
    for mesh in &avatar.meshes {
        if mesh.indices.iter().any(|&i| i as usize >= mesh.positions.len()) {
            return Err(bad_scene(format!("mesh {}: index out of range", mesh.name)));
        }
    }
    let out_dir = parent_dir(out_path);
    create_dir(&out_dir)?;
    let now = opts.timestamp.clone().unwrap_or_else(utc_now_stamp);
    let arm = opts.armature_name.as_str();
    let mut l: Vec<String> = Vec::new();
    macro_rules! w {
        ($($t:tt)*) => { l.push(format!($($t)*)) };
    }
    w!(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    w!(
        r#"<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">"#
    );
    w!("  <asset>");
    w!("    <contributor>");
    w!("      <author>ReXGlue Avatar Export</author>");
    w!("      <authoring_tool>ReXGlue Avatar Export (avatarextract --avatar)</authoring_tool>");
    w!("    </contributor>");
    w!("    <created>{now}</created>");
    w!("    <modified>{now}</modified>");
    let uname = if (opts.unit_meter - 0.01).abs() < 1e-9 {
        "centimeter"
    } else {
        "meter"
    };
    w!(
        r#"    <unit name="{uname}" meter="{}"/>"#,
        fmt(opts.unit_meter, 4)
    );
    w!("    <up_axis>{}</up_axis>", opts.up_axis);
    w!("  </asset>");

    // materials, images, effects
    let mut mats: Vec<(usize, String, String)> = Vec::new();
    for mi in sorted_used_materials(avatar) {
        let md = &avatar.materials[mi];
        let name = safe_id(&format!("{}{}", md.name, opts.material_suffix));
        let src_png = avatar.file(&md.diffuse);
        let dst_name = format!("{name}_Diffuse.png");
        let rel_dir = opts.texture_dir.as_str();
        if opts.copy_textures && src_png.is_file() {
            let dst_dir = if rel_dir.is_empty() {
                out_dir.clone()
            } else {
                out_dir.join(rel_dir)
            };
            create_dir(&dst_dir)?;
            copy_file(&src_png, &dst_dir.join(&dst_name))?;
        }
        let init_from = if rel_dir.is_empty() {
            dst_name
        } else {
            format!("{}/{dst_name}", rel_dir.replace('\\', "/"))
        };
        mats.push((mi, name, init_from));
    }
    w!("  <library_images>");
    for (_, name, init_from) in &mats {
        w!(r#"    <image id="{name}_png" name="{name}_png">"#);
        w!("      <init_from>{}</init_from>", xml_escape(init_from));
        w!("    </image>");
    }
    w!("  </library_images>");
    w!("  <library_effects>");
    for (_, name, _) in &mats {
        w!(r#"    <effect id="{name}-effect">"#);
        w!("      <profile_COMMON>");
        w!(r#"        <newparam sid="{name}_png-surface">"#);
        w!(r#"          <surface type="2D">"#);
        w!("            <init_from>{name}_png</init_from>");
        w!("          </surface>");
        w!("        </newparam>");
        w!(r#"        <newparam sid="{name}_png-sampler">"#);
        w!("          <sampler2D>");
        w!("            <source>{name}_png-surface</source>");
        w!("          </sampler2D>");
        w!("        </newparam>");
        w!(r#"        <technique sid="common">"#);
        w!("          <lambert>");
        w!(r#"            <emission><color sid="emission">0 0 0 1</color></emission>"#);
        w!(r#"            <diffuse><texture texture="{name}_png-sampler" texcoord="UVMap"/></diffuse>"#);
        w!(r#"            <index_of_refraction><float sid="ior">1.45</float></index_of_refraction>"#);
        w!("          </lambert>");
        w!("        </technique>");
        w!("      </profile_COMMON>");
        w!("    </effect>");
    }
    w!("  </library_effects>");
    w!("  <library_materials>");
    for (_, name, _) in &mats {
        w!(r#"    <material id="{name}-material" name="{name}">"#);
        w!(r##"      <instance_effect url="#{name}-effect"/>"##);
        w!("    </material>");
    }
    w!("  </library_materials>");
    let mat_name: HashMap<usize, &str> = mats.iter().map(|(mi, n, _)| (*mi, n.as_str())).collect();

    // geometries
    w!("  <library_geometries>");
    let mut mesh_ids = Vec::with_capacity(avatar.meshes.len());
    for mesh in &avatar.meshes {
        let gid = safe_id(&mesh.name);
        let n = mesh.positions.len();
        let pos: Vec<_> = mesh.positions.iter().map(|p| rig.xform_point(p)).collect();
        let nrm: Vec<_> = mesh.normals.iter().map(|v| rig.xform_dir(v)).collect();
        w!(r#"    <geometry id="{gid}-mesh" name="{gid}">"#);
        w!("      <mesh>");
        w!(r#"        <source id="{gid}-mesh-positions">"#);
        w!(
            r#"          <float_array id="{gid}-mesh-positions-array" count="{}">{}</float_array>"#,
            3 * n,
            join_fmt(pos.iter().flatten().copied(), 4)
        );
        w!("          <technique_common>");
        w!(r##"            <accessor source="#{gid}-mesh-positions-array" count="{n}" stride="3">"##);
        w!(
            r#"              <param name="X" type="float"/><param name="Y" type="float"/><param name="Z" type="float"/>"#
        );
        w!("            </accessor>");
        w!("          </technique_common>");
        w!("        </source>");
        w!(r#"        <source id="{gid}-mesh-normals">"#);
        w!(
            r#"          <float_array id="{gid}-mesh-normals-array" count="{}">{}</float_array>"#,
            3 * n,
            join_fmt(nrm.iter().flatten().copied(), 4)
        );
        w!("          <technique_common>");
        w!(r##"            <accessor source="#{gid}-mesh-normals-array" count="{n}" stride="3">"##);
        w!(
            r#"              <param name="X" type="float"/><param name="Y" type="float"/><param name="Z" type="float"/>"#
        );
        w!("            </accessor>");
        w!("          </technique_common>");
        w!("        </source>");
        w!(r#"        <source id="{gid}-mesh-map-0">"#);
        let uvs = mesh
            .uvs
            .iter()
            .flat_map(|&[u, v]| [u, if opts.flip_v { 1.0 - v } else { v }]);
        w!(
            r#"          <float_array id="{gid}-mesh-map-0-array" count="{}">{}</float_array>"#,
            2 * n,
            join_fmt(uvs, 5)
        );
        w!("          <technique_common>");
        w!(r##"            <accessor source="#{gid}-mesh-map-0-array" count="{n}" stride="2">"##);
        w!(r#"              <param name="S" type="float"/><param name="T" type="float"/>"#);
        w!("            </accessor>");
        w!("          </technique_common>");
        w!("        </source>");
        let colors = mesh.colors.as_ref().filter(|_| opts.include_colors);
        if let Some(colors) = colors {
            w!(r#"        <source id="{gid}-mesh-colors-Col">"#);
            w!(
                r#"          <float_array id="{gid}-mesh-colors-Col-array" count="{}">{}</float_array>"#,
                4 * n,
                join_fmt(colors.iter().flatten().copied(), 4)
            );
            w!("          <technique_common>");
            w!(r##"            <accessor source="#{gid}-mesh-colors-Col-array" count="{n}" stride="4">"##);
            w!(
                r#"              <param name="R" type="float"/><param name="G" type="float"/><param name="B" type="float"/><param name="A" type="float"/>"#
            );
            w!("            </accessor>");
            w!("          </technique_common>");
            w!("        </source>");
        }
        w!(r#"        <vertices id="{gid}-mesh-vertices">"#);
        w!(r##"          <input semantic="POSITION" source="#{gid}-mesh-positions"/>"##);
        w!("        </vertices>");
        let tri_count = mesh.indices.len() / 3;
        let mname = mat_name[&mesh.material];
        w!(r#"        <triangles material="{mname}-material" count="{tri_count}">"#);
        w!(r##"          <input semantic="VERTEX" source="#{gid}-mesh-vertices" offset="0"/>"##);
        w!(r##"          <input semantic="NORMAL" source="#{gid}-mesh-normals" offset="1"/>"##);
        w!(r##"          <input semantic="TEXCOORD" source="#{gid}-mesh-map-0" offset="2" set="0"/>"##);
        if colors.is_some() {
            w!(r##"          <input semantic="COLOR" source="#{gid}-mesh-colors-Col" offset="3" set="0"/>"##);
        }
        let per = if colors.is_some() { 4 } else { 3 };
        let mut p: Vec<String> = Vec::with_capacity(mesh.indices.len() * per);
        for idx in &mesh.indices {
            let s = idx.to_string();
            for _ in 0..per {
                p.push(s.clone());
            }
        }
        w!("          <p>{}</p>", p.join(" "));
        w!("        </triangles>");
        w!("      </mesh>");
        w!("    </geometry>");
        mesh_ids.push(gid);
    }
    w!("  </library_geometries>");

    // controllers
    let bone_sids: Vec<&str> = rig.bones.iter().map(|b| b.name.as_str()).collect();
    let inv_binds: Vec<_> = rig.bones.iter().map(|b| mat_inverse(&b.world)).collect();
    let bind_text = inv_binds
        .iter()
        .map(|m| mat_text(m, 6))
        .collect::<Vec<_>>()
        .join(" ");
    w!("  <library_controllers>");
    for (mesh, gid) in avatar.meshes.iter().zip(&mesh_ids) {
        let cid = format!("{arm}_{gid}-skin");
        let n = mesh.positions.len();
        let nb = bone_sids.len();
        w!(r#"    <controller id="{cid}" name="{arm}">"#);
        w!(r##"      <skin source="#{gid}-mesh">"##);
        w!("        <bind_shape_matrix>1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</bind_shape_matrix>");
        w!(r#"        <source id="{cid}-joints">"#);
        w!(
            r#"          <Name_array id="{cid}-joints-array" count="{nb}">{}</Name_array>"#,
            bone_sids.join(" ")
        );
        w!("          <technique_common>");
        w!(r##"            <accessor source="#{cid}-joints-array" count="{nb}" stride="1">"##);
        w!(r#"              <param name="JOINT" type="name"/>"#);
        w!("            </accessor>");
        w!("          </technique_common>");
        w!("        </source>");
        w!(r#"        <source id="{cid}-bind_poses">"#);
        w!(
            r#"          <float_array id="{cid}-bind_poses-array" count="{}">{bind_text}</float_array>"#,
            16 * nb
        );
        w!("          <technique_common>");
        w!(r##"            <accessor source="#{cid}-bind_poses-array" count="{nb}" stride="16">"##);
        w!(r#"              <param name="TRANSFORM" type="float4x4"/>"#);
        w!("            </accessor>");
        w!("          </technique_common>");
        w!("        </source>");
        let mut weights: Vec<f64> = Vec::new();
        let mut vcount: Vec<String> = Vec::with_capacity(n);
        let mut v: Vec<String> = Vec::new();
        for (ji, wi) in mesh.joints.iter().zip(&mesh.weights) {
            let infl = rig.vertex_influences(ji, wi);
            vcount.push(infl.len().to_string());
            for (b, wt) in infl {
                v.push(b.to_string());
                v.push(weights.len().to_string());
                weights.push(wt);
            }
        }
        let nw = weights.len();
        w!(r#"        <source id="{cid}-weights">"#);
        w!(
            r#"          <float_array id="{cid}-weights-array" count="{nw}">{}</float_array>"#,
            join_fmt(weights, 5)
        );
        w!("          <technique_common>");
        w!(r##"            <accessor source="#{cid}-weights-array" count="{nw}" stride="1">"##);
        w!(r#"              <param name="WEIGHT" type="float"/>"#);
        w!("            </accessor>");
        w!("          </technique_common>");
        w!("        </source>");
        w!("        <joints>");
        w!(r##"          <input semantic="JOINT" source="#{cid}-joints"/>"##);
        w!(r##"          <input semantic="INV_BIND_MATRIX" source="#{cid}-bind_poses"/>"##);
        w!("        </joints>");
        w!(r#"        <vertex_weights count="{n}">"#);
        w!(r##"          <input semantic="JOINT" source="#{cid}-joints" offset="0"/>"##);
        w!(r##"          <input semantic="WEIGHT" source="#{cid}-weights" offset="1"/>"##);
        w!("          <vcount>{}</vcount>", vcount.join(" "));
        w!("          <v>{}</v>", v.join(" "));
        w!("        </vertex_weights>");
        w!("      </skin>");
        w!("    </controller>");
    }
    w!("  </library_controllers>");

    // one clip, matrix channels on every bone
    if let Some(animation) = opts.animation {
        let frames = animation.frame_count as usize;
        let fps = animation.rate();
        let times: Vec<f64> = (0..frames).map(|f| f as f64 / fps).collect();
        let times_text = join_fmt(times.iter().copied(), 5);
        let mut per_bone: Vec<Vec<String>> = vec![Vec::with_capacity(frames); rig.bones.len()];
        for f in 0..frames {
            let locs = clip_frame_locals(avatar, rig, animation, f)?;
            for (bi, m) in locs.iter().enumerate() {
                per_bone[bi].push(mat_text(m, 5));
            }
        }
        let aname = safe_id(&animation.name);
        let interp = vec!["LINEAR"; frames].join(" ");
        w!("  <library_animations>");
        w!(r#"    <animation id="action_container-{arm}" name="{aname}">"#);
        for (bi, b) in rig.bones.iter().enumerate() {
            let bid = safe_id(&b.name);
            let nid = format!("{arm}_{bid}");
            let aid = format!("{aname}_{nid}_pose_matrix");
            w!(r#"      <animation id="{aid}" name="{bid}">"#);
            w!(r#"        <source id="{aid}-input">"#);
            w!(
                r#"          <float_array id="{aid}-input-array" count="{frames}">{times_text}</float_array>"#
            );
            w!("          <technique_common>");
            w!(r##"            <accessor source="#{aid}-input-array" count="{frames}" stride="1">"##);
            w!(r#"              <param name="TIME" type="float"/>"#);
            w!("            </accessor>");
            w!("          </technique_common>");
            w!("        </source>");
            w!(r#"        <source id="{aid}-output">"#);
            w!(
                r#"          <float_array id="{aid}-output-array" count="{}">{}</float_array>"#,
                16 * frames,
                per_bone[bi].join(" ")
            );
            w!("          <technique_common>");
            w!(r##"            <accessor source="#{aid}-output-array" count="{frames}" stride="16">"##);
            w!(r#"              <param name="TRANSFORM" type="float4x4"/>"#);
            w!("            </accessor>");
            w!("          </technique_common>");
            w!("        </source>");
            w!(r#"        <source id="{aid}-interpolation">"#);
            w!(
                r#"          <Name_array id="{aid}-interpolation-array" count="{frames}">{interp}</Name_array>"#
            );
            w!("          <technique_common>");
            w!(r##"            <accessor source="#{aid}-interpolation-array" count="{frames}" stride="1">"##);
            w!(r#"              <param name="INTERPOLATION" type="name"/>"#);
            w!("            </accessor>");
            w!("          </technique_common>");
            w!("        </source>");
            w!(r#"        <sampler id="{aid}-sampler">"#);
            w!(r##"          <input semantic="INPUT" source="#{aid}-input"/>"##);
            w!(r##"          <input semantic="OUTPUT" source="#{aid}-output"/>"##);
            w!(r##"          <input semantic="INTERPOLATION" source="#{aid}-interpolation"/>"##);
            w!("        </sampler>");
            w!(r##"        <channel source="#{aid}-sampler" target="{nid}/transform"/>"##);
            w!("      </animation>");
        }
        w!("    </animation>");
        w!("  </library_animations>");
    }

    // visual scene
    let scene = opts.scene_name.as_str();
    w!("  <library_visual_scenes>");
    w!(r#"    <visual_scene id="{scene}" name="{scene}">"#);
    w!(r#"      <node id="{arm}" name="{arm}" type="NODE">"#);
    w!(r#"        <matrix sid="transform">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</matrix>"#);
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, b) in rig.bones.iter().enumerate() {
        children.entry(b.parent).or_default().push(i);
    }
    let mut stack: Vec<(usize, usize, bool)> = children
        .get(&-1)
        .map(|r| r.iter().rev().map(|&i| (i, 0, false)).collect())
        .unwrap_or_default();
    while let Some((i, depth, closing)) = stack.pop() {
        let ind = "  ".repeat(4 + depth);
        if closing {
            w!("{ind}</node>");
            continue;
        }
        let b = &rig.bones[i];
        let nid = format!("{arm}_{}", safe_id(&b.name));
        let name = &b.name;
        w!(r#"{ind}<node id="{nid}" name="{name}" sid="{name}" type="JOINT">"#);
        w!(
            r#"{ind}  <matrix sid="transform">{}</matrix>"#,
            mat_text(&b.local, 6)
        );
        let tip = b.tip.unwrap_or([0.0, 0.0, 1.0]);
        w!("{ind}  <extra>");
        w!(r#"{ind}    <technique profile="blender">"#);
        w!(r#"{ind}      <connect sid="connect" type="bool">0</connect>"#);
        w!(r#"{ind}      <layer sid="layer" type="string">0</layer>"#);
        w!(
            r#"{ind}      <roll sid="roll" type="float">{}</roll>"#,
            fmt(b.roll, 6)
        );
        w!(
            r#"{ind}      <tip_x sid="tip_x" type="float">{}</tip_x>"#,
            fmt(tip[0], 5)
        );
        w!(
            r#"{ind}      <tip_y sid="tip_y" type="float">{}</tip_y>"#,
            fmt(tip[1], 5)
        );
        w!(
            r#"{ind}      <tip_z sid="tip_z" type="float">{}</tip_z>"#,
            fmt(tip[2], 5)
        );
        w!("{ind}    </technique>");
        w!("{ind}  </extra>");
        stack.push((i, depth, true));
        let index = i32::try_from(i).map_err(|_| bad_scene("too many bones"))?;
        if let Some(kids) = children.get(&index) {
            for &c in kids.iter().rev() {
                stack.push((c, depth + 1, false));
            }
        }
    }
    w!("      </node>");
    let root_index = children.get(&-1).and_then(|r| r.first()).copied().unwrap_or(0);
    let root_bone = rig
        .bones
        .get(root_index)
        .ok_or_else(|| Error::Scene("the rig has no bones".into()))?;
    let root_id = safe_id(&root_bone.name);
    for (mesh, gid) in avatar.meshes.iter().zip(&mesh_ids) {
        let cid = format!("{arm}_{gid}-skin");
        let mname = mat_name[&mesh.material];
        w!(r#"      <node id="{gid}" name="{gid}" type="NODE">"#);
        w!(r#"        <matrix sid="transform">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</matrix>"#);
        w!(r##"        <instance_controller url="#{cid}">"##);
        w!("          <skeleton>#{arm}_{root_id}</skeleton>");
        w!("          <bind_material>");
        w!("            <technique_common>");
        w!(r##"              <instance_material symbol="{mname}-material" target="#{mname}-material">"##);
        w!(
            r#"                <bind_vertex_input semantic="UVMap" input_semantic="TEXCOORD" input_set="0"/>"#
        );
        w!("              </instance_material>");
        w!("            </technique_common>");
        w!("          </bind_material>");
        w!("        </instance_controller>");
        w!("      </node>");
    }
    w!("    </visual_scene>");
    w!("  </library_visual_scenes>");
    w!("  <scene>");
    w!(r##"    <instance_visual_scene url="#{scene}"/>"##);
    w!("  </scene>");
    w!("</COLLADA>");
    write_lines(out_path, &l)?;
    Ok(out_path.to_path_buf())
}
