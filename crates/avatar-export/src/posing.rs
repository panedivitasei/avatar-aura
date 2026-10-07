// Port of avatar_aura/posing.py: integer-frame sampling and linear blend skinning for the unchanged mesh writers.

use std::collections::HashMap;

use crate::avatar::{Avatar, Clip};
use crate::error::{invalid, Error, Result};
use crate::faces::{texture_files, Expression};
use crate::math::*;
use crate::rig::{
    anim_local_matrices, build_source_rig, check_parents, fk_world, retarget_frame_to_rig,
    rig_locals_from_worlds, Rig,
};

/// One bone of a free pose, addressed by its safe id.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseBone {
    pub name: String,
    pub position: V3,
    /// Quaternion x, y, z, w; normalised on use.
    pub rotation: [f64; 4],
}

/// Rig whose bones carry the caller's local transforms.
pub fn from_local_pose(avatar: &Avatar, transforms: &[PoseBone]) -> Result<Rig> {
    let mut rig = build_source_rig(avatar, FRAME_XBOX, 1.0)?;
    if transforms.len() != rig.bones.len() {
        return Err(invalid("Free pose must contain every avatar bone."));
    }
    let by_name: HashMap<&str, &PoseBone> = transforms.iter().map(|e| (e.name.as_str(), e)).collect();
    if by_name.len() != rig.bones.len() {
        return Err(invalid("Free pose contains duplicate bones."));
    }
    for i in 0..rig.bones.len() {
        let bone = &rig.bones[i];
        let entry = by_name
            .get(safe_id(&bone.name).as_str())
            .ok_or_else(|| invalid(format!("Free pose is missing bone {}.", bone.name)))?;
        if !entry
            .position
            .iter()
            .chain(&entry.rotation)
            .all(|v| v.is_finite())
        {
            return Err(invalid("Bone transforms must contain finite numbers."));
        }
        let length = psum(entry.rotation.iter().map(|v| v * v)).sqrt();
        if length < 1e-8 {
            return Err(invalid("Bone rotation cannot be zero."));
        }
        let local = mat_from_rt(&quat_to_r3(&entry.rotation.map(|v| v / length)), &entry.position);
        let world = match usize::try_from(bone.parent) {
            Ok(p) if p < i => mat_mul(&rig.bones[p].world, &local),
            Ok(_) => return Err(Error::Scene("bone parent does not precede it".into())),
            Err(_) => local,
        };
        rig.bones[i].local = local;
        rig.bones[i].world = world;
    }
    Ok(rig)
}

/// The rig at one integer frame of a clip.
pub fn sample(avatar: &Avatar, clip: &Clip, frame: i64) -> Result<Rig> {
    if frame < 0 || frame >= i64::from(clip.frame_count) {
        return Err(invalid("Animation frame is outside the clip."));
    }
    let mut rig = build_source_rig(avatar, FRAME_XBOX, 1.0)?;
    check_parents(&rig)?;
    let local = anim_local_matrices(avatar, clip, frame as usize);
    let worlds = retarget_frame_to_rig(avatar, &rig, &fk_world(avatar, &local)?)?;
    let locals = rig_locals_from_worlds(&rig, &worlds);
    for ((bone, world), local) in rig.bones.iter_mut().zip(worlds).zip(locals) {
        bone.world = world;
        bone.local = local;
    }
    Ok(rig)
}

/// [`sample`] with the clip looked up by name.
pub fn sample_named(avatar: &Avatar, clip: &str, frame: i64) -> Result<Rig> {
    let c = avatar
        .clip(clip)
        .ok_or_else(|| invalid(format!("Unknown animation: {clip}")))?;
    sample(avatar, c, frame)
}

/// Bind reference for posed GLBs: frame 1 of the standing clip when present, else the rest rig.
pub fn glb_bind_rig(avatar: &Avatar) -> Result<Rig> {
    match avatar.clip("Animation Generic Stand 1") {
        Some(clip) => sample(avatar, clip, 1.min(i64::from(clip.frame_count) - 1)),
        None => build_source_rig(avatar, FRAME_XBOX, 1.0),
    }
}

/// Keeps skin bind matrices stable while the GLB nodes carry the visible pose.
pub fn glb_pose_rig(avatar: &Avatar, pose: &Rig) -> Result<Rig> {
    let mut rig = pose.clone();
    let bind = glb_bind_rig(avatar)?;
    for (bone, rest) in rig.bones.iter_mut().zip(&bind.bones) {
        bone.world = rest.world;
    }
    Ok(rig)
}

fn influence_key(infl: &[(usize, f64)]) -> Vec<(usize, u64)> {
    infl.iter().map(|&(j, w)| (j, w.to_bits())).collect()
}

/// Undoes the new skin transform so changing bind references preserves the visible surface.
pub fn rebind_posed(avatar: &Avatar, pose: &Rig, bind: &Rig) -> Result<Avatar> {
    let mut rebound = avatar.clone();
    let matrices: Vec<M4> = pose
        .bones
        .iter()
        .zip(&bind.bones)
        .map(|(p, b)| mat_mul(&p.world, &mat_inverse(&b.world)))
        .collect();
    let mut inverses: HashMap<Vec<(usize, u64)>, M4> = HashMap::new();
    for mesh in &mut rebound.meshes {
        let count = mesh
            .positions
            .len()
            .min(mesh.normals.len())
            .min(mesh.joints.len())
            .min(mesh.weights.len());
        for index in 0..count {
            let influences = pose.vertex_influences(&mesh.joints[index], &mesh.weights[index]);
            let key = influence_key(&influences);
            let inverse = match inverses.get(&key) {
                Some(m) => *m,
                None => {
                    let mut blended = [[0.0; 4]; 4];
                    for (row, out_row) in blended.iter_mut().enumerate() {
                        for (column, cell) in out_row.iter_mut().enumerate() {
                            let mut terms = Vec::with_capacity(influences.len());
                            for &(joint, weight) in &influences {
                                let m = matrices
                                    .get(joint)
                                    .ok_or_else(|| Error::Scene("joint outside the bind rig".into()))?;
                                terms.push(weight * m[row][column]);
                            }
                            *cell = psum(terms);
                        }
                    }
                    let inverse = mat_inverse(&blended);
                    let product = mat_mul(&blended, &inverse);
                    let collapsed = (0..4).any(|r| {
                        (0..4).any(|c| (product[r][c] - if r == c { 1.0 } else { 0.0 }).abs() > 1e-6)
                    });
                    if collapsed {
                        return Err(invalid(
                            "This pose collapses a skinned surface. Adjust its bone rotations before exporting GLB.",
                        ));
                    }
                    inverses.insert(key, inverse);
                    inverse
                }
            };
            mesh.positions[index] = mat_vec(&inverse, &mesh.positions[index], 1.0);
            mesh.normals[index] = vnorm(&mat_vec(&inverse, &mesh.normals[index], 0.0));
        }
    }
    Ok(rebound)
}

/// Skins the meshes into `rig`'s pose, makes that pose the rest skeleton and applies the expression's head textures.
pub fn bake_posed(avatar: &Avatar, rig: &Rig, expression: &Expression) -> Result<Avatar> {
    if rig.frame != FRAME_XBOX || rig.scale != 1.0 {
        return Err(invalid(
            "Pose sampling requires the avatar's native frame in metres.",
        ));
    }
    let mut posed = avatar.clone();
    let bind = build_source_rig(avatar, FRAME_XBOX, 1.0)?;
    let matrices: Vec<M4> = rig
        .bones
        .iter()
        .zip(&bind.bones)
        .map(|(bone, rest)| mat_mul(&bone.world, &mat_inverse(&rest.world)))
        .collect();
    for (original, mesh) in avatar.meshes.iter().zip(posed.meshes.iter_mut()) {
        mesh.positions.clear();
        mesh.normals.clear();
        for (((position, normal), joints), weights) in original
            .positions
            .iter()
            .zip(&original.normals)
            .zip(&original.joints)
            .zip(&original.weights)
        {
            let mut point = [0.0; 3];
            let mut direction = [0.0; 3];
            for (joint, weight) in rig.vertex_influences(joints, weights) {
                let m = matrices
                    .get(joint)
                    .ok_or_else(|| Error::Scene("joint outside the bind rig".into()))?;
                let transformed = mat_vec(m, position, 1.0);
                let rotated = mat_vec(m, normal, 0.0);
                for axis in 0..3 {
                    point[axis] += transformed[axis] * weight;
                    direction[axis] += rotated[axis] * weight;
                }
            }
            mesh.positions.push(point);
            mesh.normals.push(vnorm(&direction));
        }
    }
    for (joint, bone) in posed.skeleton.iter_mut().zip(&rig.bones) {
        joint.rest_world = mat_trans(&bone.world);
        joint.rest_world_rot = r3_to_quat(&mat_rot3(&bone.world));
        joint.rest_local = mat_trans(&bone.local);
        joint.bind_world = joint.rest_world;
    }
    for (index, file) in texture_files(avatar, expression)? {
        posed.materials[index].diffuse = file;
    }
    posed.animations.clear();
    Ok(posed)
}
