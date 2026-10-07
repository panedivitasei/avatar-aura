// Joint state for the viewport: rig-local matrices, forward kinematics and the skinning palette.
// Bind matrices are the rest rig from `avatar_export::rig::build_source_rig`, matching the preview GLB skin.

use avatar_export::avatar::{Avatar, Clip};
use avatar_export::math::{
    mat_from_rt, mat_inverse, mat_mul, mat_rot3, mat_trans, quat_to_r3, r3_to_quat, M4,
};
use avatar_export::rig::{build_source_rig, clip_frame_locals, Rig, FRAME_XBOX};
use glam::{DQuat, DVec3, Mat4};

use crate::Result;

/// Row-major `avatar_export` matrix to a column-major glam matrix.
pub fn to_mat4(m: &M4) -> Mat4 {
    Mat4::from_cols_array(&std::array::from_fn(|k| m[k % 4][k / 4] as f32))
}

/// Column-major glam matrix to a row-major `avatar_export` matrix.
pub fn from_mat4(m: &Mat4) -> M4 {
    let cols = m.to_cols_array_2d();
    std::array::from_fn(|r| std::array::from_fn(|c| f64::from(cols[c][r])))
}

/// `world * inverse(bind)` per joint; the identity everywhere in the rest pose.
pub fn skin_palette(rig: &Rig, worlds: &[M4]) -> Vec<Mat4> {
    rig.bones
        .iter()
        .zip(worlds)
        .map(|(bone, world)| to_mat4(&mat_mul(world, &mat_inverse(&bone.world))))
        .collect()
}

/// World matrices from rig-local ones, parents first.
pub fn forward_kinematics(rig: &Rig, locals: &[M4]) -> Vec<M4> {
    let mut worlds: Vec<M4> = Vec::with_capacity(locals.len());
    for (i, local) in locals.iter().enumerate() {
        let world = match usize::try_from(rig.bones[i].parent) {
            Ok(p) if p < i => mat_mul(&worlds[p], local),
            _ => *local,
        };
        worlds.push(world);
    }
    worlds
}

struct Trs {
    t: DVec3,
    r: DQuat,
    s: DVec3,
}

fn decompose(m: &M4) -> Trs {
    let r = mat_rot3(m);
    let s = [0, 1, 2].map(|c| (r[0][c] * r[0][c] + r[1][c] * r[1][c] + r[2][c] * r[2][c]).sqrt());
    let n = [0, 1, 2].map(|i| [0, 1, 2].map(|c| r[i][c] / if s[c] == 0.0 { 1.0 } else { s[c] }));
    let [x, y, z, w] = r3_to_quat(&n);
    Trs {
        t: DVec3::from(mat_trans(m)),
        r: DQuat::from_xyzw(x, y, z, w),
        s: DVec3::from(s),
    }
}

fn compose(p: &Trs) -> M4 {
    let r = quat_to_r3(&[p.r.x, p.r.y, p.r.z, p.r.w]);
    let s = p.s.to_array();
    let rs = [0, 1, 2].map(|i| [0, 1, 2].map(|c| r[i][c] * s[c]));
    mat_from_rt(&rs, &p.t.to_array())
}

/// Rig-local matrices at a fractional frame: linear translation and scale, slerped rotation, as the GLB sampler plays.
pub fn clip_locals_at(avatar: &Avatar, rig: &Rig, clip: &Clip, frame: f32) -> Result<Vec<M4>> {
    let last = clip.frame_count.saturating_sub(1) as usize;
    let f = f64::from(frame).clamp(0.0, last as f64);
    let f0 = f.floor() as usize;
    let f1 = (f0 + 1).min(last);
    let a = f - f0 as f64;
    let l0 = clip_frame_locals(avatar, rig, clip, f0)?;
    if f1 == f0 || a <= 0.0 {
        return Ok(l0);
    }
    let l1 = clip_frame_locals(avatar, rig, clip, f1)?;
    Ok(l0
        .iter()
        .zip(&l1)
        .map(|(m0, m1)| {
            let (p0, p1) = (decompose(m0), decompose(m1));
            compose(&Trs {
                t: p0.t.lerp(p1.t, a),
                r: p0.r.slerp(p1.r, a),
                s: p0.s.lerp(p1.s, a),
            })
        })
        .collect())
}

/// The loaded skeleton and its current pose.
pub struct PoseState {
    pub avatar: Avatar,
    pub rig: Rig,
    pub locals: Vec<M4>,
    pub worlds: Vec<M4>,
}

impl PoseState {
    pub fn new(avatar: Avatar) -> Result<Self> {
        let rig = build_source_rig(&avatar, FRAME_XBOX, 1.0)?;
        let locals: Vec<M4> = rig.bones.iter().map(|b| b.local).collect();
        let worlds = forward_kinematics(&rig, &locals);
        Ok(Self {
            avatar,
            rig,
            locals,
            worlds,
        })
    }

    pub fn set_locals(&mut self, locals: Vec<M4>) {
        self.worlds = forward_kinematics(&self.rig, &locals);
        self.locals = locals;
    }

    pub fn rest(&mut self) {
        let locals = self.rig.bones.iter().map(|b| b.local).collect();
        self.set_locals(locals);
    }

    pub fn palette(&self) -> Vec<Mat4> {
        skin_palette(&self.rig, &self.worlds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avatar_export::scene::{Joint, Scene, Skeleton};

    fn joint(name: &str, parent: i32, pos: [f32; 3], rot: [f32; 4]) -> Joint {
        Joint {
            name: name.into(),
            parent,
            bind_world: pos,
            rest_world: pos,
            rest_world_rot: rot,
            rest_local: pos,
            scale: [1.0; 3],
        }
    }

    #[test]
    fn rest_palette_matches_rig_bind() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let scene = Scene {
            skeleton: Skeleton {
                version: 1,
                joints: vec![
                    joint("BASE", -1, [0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0]),
                    joint("BACKA", 0, [0.0, 1.1, 0.05], [0.0, h, 0.0, h]),
                    joint("NECK", 1, [0.1, 1.4, 0.0], [h, 0.0, 0.0, h]),
                ],
            },
            ..Scene::default()
        };
        let avatar = Avatar::from_scene(&scene, std::path::PathBuf::new()).unwrap();
        let state = PoseState::new(avatar.clone()).unwrap();
        let rig = build_source_rig(&avatar, FRAME_XBOX, 1.0).unwrap();
        let expected: Vec<Mat4> = rig
            .bones
            .iter()
            .map(|b| to_mat4(&mat_mul(&b.world, &mat_inverse(&b.world))))
            .collect();
        let palette = state.palette();
        assert_eq!(palette.len(), expected.len());
        for (got, want) in palette.iter().zip(&expected) {
            assert!(got.abs_diff_eq(*want, 1e-5), "{got:?} != {want:?}");
            assert!(got.abs_diff_eq(Mat4::IDENTITY, 1e-5));
        }
    }

    #[test]
    fn matrix_round_trip() {
        let m: M4 = [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let g = to_mat4(&m);
        assert_eq!(g.w_axis.x, 4.0);
        assert_eq!(from_mat4(&g), m);
    }
}
