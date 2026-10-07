// Port of avatar_aura/ae_convert.py `Bone`, `Rig`, `build_source_rig` and the FK/retarget helpers.
// Bone i is source joint i, so mesh joint indices address the rig directly.

use std::collections::HashMap;

use crate::avatar::{Avatar, Clip, Joint};
use crate::error::{bad_scene, Result};
use crate::math::*;

pub use crate::math::{FRAME_SOURCE, FRAME_XBOX};

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Parent index, negative for roots.
    pub parent: i32,
    /// World matrix in the rig's frame and units.
    pub world: M4,
    /// Matrix relative to the parent.
    pub local: M4,
    /// Armature-space tail offset for the Blender extras.
    pub tip: Option<V3>,
    pub roll: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    /// Parents before children.
    pub bones: Vec<Bone>,
    /// Rotation applied to source positions and normals.
    pub frame: M3,
    /// Unit scale applied after the frame.
    pub scale: f64,
}

/// One (bone, weight) pair of a vertex, weights normalised.
pub type Influence = (usize, f64);

impl Rig {
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    pub fn xform_point(&self, p: &V3) -> V3 {
        vscale(&r3_vec(&self.frame, p), self.scale)
    }

    pub fn xform_dir(&self, n: &V3) -> V3 {
        vnorm(&r3_vec(&self.frame, n))
    }

    /// Merged, normalised influences sorted by descending weight; ties keep first-seen order.
    pub fn vertex_influences(&self, joints: &[u16; 4], weights: &[f64; 4]) -> Vec<Influence> {
        let mut acc: Vec<Influence> = Vec::with_capacity(4);
        for (&j, &w) in joints.iter().zip(weights) {
            let j = usize::from(j);
            if w <= 0.0 || j >= self.bones.len() || w.is_nan() {
                continue;
            }
            match acc.iter_mut().find(|e| e.0 == j) {
                Some(e) => e.1 += w,
                None => acc.push((j, 0.0 + w)),
            }
        }
        if acc.is_empty() {
            return vec![(0, 1.0)];
        }
        let mut total = psum(acc.iter().map(|e| e.1));
        if total == 0.0 {
            total = 1.0;
        }
        let mut out: Vec<Influence> = acc.into_iter().map(|(b, w)| (b, w / total)).collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    fn parent_index(&self, i: usize) -> Option<usize> {
        usize::try_from(self.bones[i].parent).ok()
    }
}

/// The export's own skeleton (Xbox joint names) in an arbitrary frame and unit scale.
pub fn build_source_rig(avatar: &Avatar, frame: M3, scale: f64) -> Result<Rig> {
    build_rig_from_joints(&avatar.skeleton, frame, scale)
}

/// [`build_source_rig`] over an explicit joint list, such as the prop skeleton.
pub fn build_rig_from_joints(joints: &[Joint], frame: M3, scale: f64) -> Result<Rig> {
    let mut bones: Vec<Bone> = Vec::with_capacity(joints.len());
    for (i, j) in joints.iter().enumerate() {
        let pos = vscale(&r3_vec(&frame, &j.rest_world), scale);
        let rot = r3_mul(&frame, &r3_mul(&quat_to_r3(&j.rest_world_rot), &r3_t(&frame)));
        let world = mat_from_rt(&rot, &pos);
        let local = match usize::try_from(j.parent) {
            Ok(p) if p < i => mat_mul(&mat_inverse_rigid(&bones[p].world), &world),
            Ok(p) => {
                return Err(bad_scene(format!(
                    "joint {} ({}) has parent {p} that does not precede it",
                    i, j.name
                )))
            }
            Err(_) => world,
        };
        bones.push(Bone {
            name: j.name.clone(),
            parent: j.parent,
            world,
            local,
            tip: None,
            roll: 0.0,
        });
    }
    for i in 0..bones.len() {
        let child = bones.iter().find(|c| usize::try_from(c.parent).ok() == Some(i));
        let tip = match child {
            Some(c) => vsub(&mat_trans(&c.world), &mat_trans(&bones[i].world)),
            None => [0.0, 0.0, 0.03 * scale],
        };
        bones[i].tip = Some(tip);
    }
    Ok(Rig { bones, frame, scale })
}

/// Local matrices (source frame, metres) of every source joint at a frame; rest pose where a track is missing.
pub fn anim_local_matrices(avatar: &Avatar, anim: &Clip, frame_index: usize) -> Vec<M4> {
    let mut locs = Vec::with_capacity(avatar.skeleton.len());
    for (ji, j) in avatar.skeleton.iter().enumerate() {
        let tr = anim
            .tracks
            .get(ji)
            .filter(|tr| frame_index < tr.t.len() && frame_index < tr.r.len() && frame_index < tr.s.len());
        match tr {
            Some(tr) => {
                let t = tr.t[frame_index];
                let r = quat_to_r3(&tr.r[frame_index]);
                let s = tr.s[frame_index];
                let r = [0, 1, 2].map(|i| [0, 1, 2].map(|k| r[i][k] * s[k]));
                locs.push(mat_from_rt(&r, &t));
            }
            None => locs.push(mat_from_rt(&quat_to_r3(&j.rest_world_rot), &j.rest_local)),
        }
    }
    locs
}

/// Forward kinematics over the source skeleton.
pub fn fk_world(avatar: &Avatar, locals: &[M4]) -> Result<Vec<M4>> {
    let mut worlds: Vec<M4> = Vec::with_capacity(locals.len());
    for (ji, j) in avatar.skeleton.iter().enumerate() {
        let local = locals
            .get(ji)
            .ok_or_else(|| bad_scene("fewer local matrices than joints"))?;
        let w = match usize::try_from(j.parent) {
            Ok(p) => {
                let pw = worlds.get(p).ok_or_else(|| {
                    bad_scene(format!(
                        "joint {} has parent {p} that does not precede it",
                        j.name
                    ))
                })?;
                mat_mul(pw, local)
            }
            Err(_) => *local,
        };
        worlds.push(w);
    }
    Ok(worlds)
}

/// World matrices of the rig's bones at an animated frame, carried into the rig's frame and units.
pub fn retarget_frame_to_rig(avatar: &Avatar, rig: &Rig, worlds_src: &[M4]) -> Result<Vec<M4>> {
    let mut by_name: HashMap<&str, M4> = HashMap::new();
    for (n, w) in avatar.joint_names().into_iter().zip(worlds_src) {
        let pos = vscale(&r3_vec(&rig.frame, &mat_trans(w)), rig.scale);
        let rot = r3_mul(
            &rig.frame,
            &r3_mul(&r3_normalize_columns(&mat_rot3(w)), &r3_t(&rig.frame)),
        );
        by_name.insert(n, mat_from_rt(&rot, &pos));
    }
    rig.bones
        .iter()
        .map(|b| {
            by_name
                .get(b.name.as_str())
                .copied()
                .ok_or_else(|| bad_scene(format!("rig bone {} is not an avatar joint", b.name)))
        })
        .collect()
}

pub fn rig_locals_from_worlds(rig: &Rig, worlds: &[M4]) -> Vec<M4> {
    (0..rig.bones.len())
        .map(|i| match rig.parent_index(i) {
            Some(p) => mat_mul(&mat_inverse(&worlds[p]), &worlds[i]),
            None => worlds[i],
        })
        .collect()
}

/// Rig-local matrices of every bone at one clip frame.
pub fn clip_frame_locals(avatar: &Avatar, rig: &Rig, clip: &Clip, frame: usize) -> Result<Vec<M4>> {
    let locs = anim_local_matrices(avatar, clip, frame);
    let worlds = fk_world(avatar, &locs)?;
    let rig_worlds = retarget_frame_to_rig(avatar, rig, &worlds)?;
    check_parents(rig)?;
    Ok(rig_locals_from_worlds(rig, &rig_worlds))
}

/// Every parent index precedes its child and lies inside the rig.
pub fn check_parents(rig: &Rig) -> Result<()> {
    for (i, _) in rig.bones.iter().enumerate() {
        if let Some(p) = rig.parent_index(i) {
            if p >= i {
                return Err(bad_scene(format!(
                    "bone {i} has parent {p} that does not precede it"
                )));
            }
        }
    }
    Ok(())
}
