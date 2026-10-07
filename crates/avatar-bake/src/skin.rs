//! Ports the vertex decode and skeleton math of avatar_export.cpp: DecodeNormal, UnpackSkin, BuildJointXforms,
//! JointName, the rest-pose skinning in bake_component and the mesh hand-off into `scene::Mesh`.

use avatar_export::scene;
use avatar_formats::model::Vertex;
use avatar_formats::skeleton::INVALID_INDEX;
use avatar_formats::Skeleton;
use glam::{Vec2, Vec3};

use crate::glmath::{clamp3, length3, Mat4, Quat};

/// rex::xenos_half_to_float without denormal preservation: no infinities, denormals flush to zero.
pub fn xenos_half_to_float(value: u16) -> f32 {
    let value = u32::from(value);
    let mut mantissa = value & 0x3FF;
    let mut exponent = (value >> 10) & 0x1F;
    if exponent == 0 {
        mantissa = 0;
        exponent = (-112i32) as u32;
    }
    let bits = ((value & 0x8000) << 16) | (exponent.wrapping_add(112) << 23) | (mantissa << 13);
    f32::from_bits(bits)
}

/// XMHENDN3: 11-bit signed x (bits 0-10), 11-bit signed y (11-21), 10-bit signed z.
pub fn decode_normal(packed: u32) -> Vec3 {
    let sext = |v: u32, bits: u32| -> i32 {
        let sign = 1u32 << (bits - 1);
        (v ^ sign).wrapping_sub(sign) as i32
    };
    let x = sext(packed & 0x7FF, 11);
    let y = sext((packed >> 11) & 0x7FF, 11);
    let z = sext((packed >> 22) & 0x3FF, 10);
    let n = clamp3(
        Vec3::new(x as f32 / 1023.0, y as f32 / 1023.0, z as f32 / 511.0),
        -1.0,
        1.0,
    );
    let len = length3(n);
    if len > 1e-6 {
        n / len
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkinVertex {
    /// Rest-posed on the scaled skeleton.
    pub position: Vec3,
    pub normal: Vec3,
    /// As authored (bind pose), for re-posing.
    pub orig_position: Vec3,
    pub orig_normal: Vec3,
    pub weights: [f32; 4],
    pub joints: [u8; 4],
    /// ARGB.
    pub color: u32,
    pub uvs: [Vec2; 6],
}

/// UnpackSkin: missing UV layers repeat layer 0.
pub fn unpack_skin(v: &Vertex, uv_count: u32) -> SkinVertex {
    let position = v.position;
    let normal = decode_normal(v.normal);
    let mut out = SkinVertex {
        position,
        normal,
        orig_position: position,
        orig_normal: normal,
        color: v.color,
        ..Default::default()
    };
    for k in 0..4 {
        out.weights[k] = ((v.blend_weight >> (8 * k)) & 0xFF) as f32 / 255.0;
        out.joints[k] = ((v.blend_indices >> (8 * k)) & 0xFF) as u8;
    }
    for j in 0..6 {
        out.uvs[j] = if j < v.uvs.len() && (j as u32) < uv_count {
            Vec2::new(xenos_half_to_float(v.uvs[j].x), xenos_half_to_float(v.uvs[j].y))
        } else if j > 0 {
            out.uvs[0]
        } else {
            Vec2::ZERO
        };
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointXform {
    /// Stored bind pose (world).
    pub bind_world: Mat4,
    /// Scaled rest pose (world), the exported bind pose.
    pub rest_world: Mat4,
    /// `rest_world * inverse(bind_world)`.
    pub skin: Mat4,
    pub rest_local_t: Vec3,
    pub rest_local_r: Quat,
    pub scale: Vec3,
    pub parent_world_scale: Vec3,
}

impl Default for JointXform {
    fn default() -> Self {
        JointXform {
            bind_world: Mat4::IDENTITY,
            rest_world: Mat4::IDENTITY,
            skin: Mat4::IDENTITY,
            rest_local_t: Vec3::ZERO,
            rest_local_r: Quat::IDENTITY,
            scale: Vec3::ONE,
            parent_world_scale: Vec3::ONE,
        }
    }
}

const JOINT_NAMES: [&str; 71] = [
    "BASE",
    "BACKA",
    "LF_H",
    "RT_H",
    "SC_BASE",
    "BACKB",
    "LF_K",
    "LF_SC_H",
    "RT_K",
    "RT_SC_H",
    "SC_BACKA",
    "LF_A",
    "LF_C",
    "LF_SC_K",
    "NECK",
    "RT_A",
    "RT_C",
    "RT_SC_K",
    "SC_BACKB",
    "HEAD",
    "LF_S",
    "LF_T",
    "RT_S",
    "RT_T",
    "SC_NECK",
    "LF_E",
    "LF_SC_S",
    "LF_SC_TWIST_S",
    "RT_E",
    "RT_SC_S",
    "RT_SC_TWIST_S",
    "LF_E_TWIST",
    "LF_SC_E",
    "LF_W",
    "RT_E_TWIST",
    "RT_SC_E",
    "RT_W",
    "LF_FINGA",
    "LF_FINGB",
    "LF_FINGC",
    "LF_FINGD",
    "LF_PROP",
    "LF_SPECIAL",
    "LF_THUMB",
    "RT_FINGA",
    "RT_FINGB",
    "RT_FINGC",
    "RT_FINGD",
    "RT_PROP",
    "RT_SPECIAL",
    "RT_THUMB",
    "LF_FINGA1",
    "LF_FINGB1",
    "LF_FINGC1",
    "LF_FINGD1",
    "LF_THUMB1",
    "RT_FINGA1",
    "RT_FINGB1",
    "RT_FINGC1",
    "RT_FINGD1",
    "RT_THUMB1",
    "LF_FINGA2",
    "LF_FINGB2",
    "LF_FINGC2",
    "LF_FINGD2",
    "LF_THUMB2",
    "RT_FINGA2",
    "RT_FINGB2",
    "RT_FINGC2",
    "RT_FINGD2",
    "RT_THUMB2",
];

pub fn joint_name(i: usize, count: usize) -> String {
    if count == 71 && i < 71 {
        JOINT_NAMES[i].to_owned()
    } else {
        format!("joint_{i}")
    }
}

/// True when the joint's parent precedes it, the breadth-first order the skeletons use.
pub fn has_parent(parent_index: u8, i: usize) -> bool {
    parent_index != INVALID_INDEX && usize::from(parent_index) < i
}

pub fn build_joint_xforms(skel: &Skeleton, apply_scale: bool) -> Vec<JointXform> {
    let mut out = vec![JointXform::default(); skel.joints.len()];
    for (x, j) in out.iter_mut().zip(skel.joints.iter()) {
        let bq = Quat::from_glam(j.bindpose.rotation);
        x.bind_world = Mat4::IDENTITY
            .translate(j.bindpose.position)
            .mul(&bq.normalize().to_mat4());
        x.scale = if apply_scale { j.pose.scale } else { Vec3::ONE };
    }
    for i in 0..skel.joints.len() {
        let parent = skel.joints[i].parent_index;
        let with_parent = has_parent(parent, i);
        let p = usize::from(parent);
        let local = if with_parent {
            out[p].bind_world.inverse().mul(&out[i].bind_world)
        } else {
            out[i].bind_world
        };
        out[i].rest_local_t = local.col3(3);
        out[i].rest_local_r = local.to_mat3().to_quat().normalize();
        let posed_local = local.mul(&Mat4::IDENTITY.scale(out[i].scale));
        if with_parent {
            let pw = out[p].rest_world;
            out[i].rest_world = pw.mul(&posed_local);
            out[i].parent_world_scale =
                Vec3::new(length3(pw.col3(0)), length3(pw.col3(1)), length3(pw.col3(2)));
        } else {
            out[i].rest_world = posed_local;
        }
        out[i].skin = out[i].rest_world.mul(&out[i].bind_world.inverse());
    }
    out
}

/// Weighted blend of `m * p` and the inverse-transpose normal over the vertex's four influences.
pub fn blend_skin(v: &SkinVertex, position: Vec3, normal: Vec3, mats: &[Mat4]) -> Option<(Vec3, Vec3)> {
    let mut p = Vec3::ZERO;
    let mut n = Vec3::ZERO;
    let mut wsum = 0.0f32;
    for k in 0..4 {
        let w = v.weights[k];
        let ji = usize::from(v.joints[k]);
        if w <= 0.0 || ji >= mats.len() {
            continue;
        }
        let m = &mats[ji];
        p += m.mul_vec4(position.extend(1.0)).truncate() * w;
        let nm = m.to_mat3().inverse().transpose();
        n += nm.mul_vec3(normal) * w;
        wsum += w;
    }
    (wsum > 0.0).then_some((p / wsum, n))
}

/// The bake-time skinning in bake_component: rest pose onto the scaled skeleton, in place.
pub fn skin_to_rest(verts: &mut [SkinVertex], xforms: &[JointXform]) {
    let mats: Vec<Mat4> = xforms.iter().map(|x| x.skin).collect();
    for v in verts.iter_mut() {
        if let Some((p, n)) = blend_skin(v, v.position, v.normal, &mats) {
            v.position = p;
            let len = length3(n);
            v.normal = if len > 1e-6 { n / len } else { v.normal };
        }
    }
}

/// One baked batch: posed vertices plus the material it draws with.
#[derive(Clone, Debug, Default)]
pub struct BakedMesh {
    pub name: String,
    pub component: i32,
    pub material: i32,
    pub uv_count: u32,
    pub verts: Vec<SkinVertex>,
    pub indices: Vec<u16>,
    pub is_prop: bool,
}

/// The avatar.json mesh record; `out_uv` is the material's UV layer.
pub fn to_scene_mesh(m: &BakedMesh, out_uv: usize) -> scene::Mesh {
    let out_uv = out_uv.min(5);
    let mut mesh = scene::Mesh {
        name: m.name.clone(),
        component: m.component as u32,
        material: m.material as u32,
        is_prop: m.is_prop,
        vertex_count: m.verts.len() as u32,
        triangle_count: (m.indices.len() / 3) as u32,
        uv_count: m.uv_count,
        ..Default::default()
    };
    for v in &m.verts {
        mesh.positions
            .extend_from_slice(&[v.position.x, v.position.y, v.position.z]);
        mesh.normals
            .extend_from_slice(&[v.normal.x, v.normal.y, v.normal.z]);
        mesh.uv.extend_from_slice(&[v.uvs[out_uv].x, v.uvs[out_uv].y]);
        mesh.colors.extend_from_slice(&[
            ((v.color >> 16) & 0xFF) as u8,
            ((v.color >> 8) & 0xFF) as u8,
            (v.color & 0xFF) as u8,
            ((v.color >> 24) & 0xFF) as u8,
        ]);
        mesh.joints.extend(v.joints.iter().map(|&j| u16::from(j)));
        mesh.weights.extend_from_slice(&v.weights);
    }
    if m.uv_count > 1 {
        for l in 0..(m.uv_count as usize).min(6) {
            mesh.uv_layers
                .push(m.verts.iter().flat_map(|v| [v.uvs[l].x, v.uvs[l].y]).collect());
        }
    }
    mesh.indices = m.indices.iter().map(|&i| u32::from(i)).collect();
    mesh
}

/// Orthonormalized rotation of a world matrix, as the joint records export it.
pub fn world_rotation(m: &Mat4) -> Quat {
    let mut m3 = m.to_mat3();
    for c in m3.0.iter_mut() {
        *c = crate::glmath::normalize3(*c);
    }
    m3.to_quat().normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        assert_eq!(xenos_half_to_float(0x3C00), 1.0);
        assert_eq!(xenos_half_to_float(0xBC00), -1.0);
        assert_eq!(xenos_half_to_float(0x0001), 0.0);
        assert_eq!(xenos_half_to_float(0x3800), 0.5);
        assert_eq!(xenos_half_to_float(0x7C00), 65536.0);
    }

    #[test]
    fn normals() {
        assert_eq!(decode_normal(0), Vec3::new(0.0, 1.0, 0.0));
        let up = decode_normal(1023 << 11);
        assert!((up.y - 1.0).abs() < 1e-6);
        let down = decode_normal(1024 << 11);
        assert!(down.y < 0.0);
    }
}
