// Port of avatar_aura/ae_convert.py `Mesh` and `Avatar`: the baked scene widened to f64 for the writers.
// Vertex and track arrays are split per element; metadata stays in the typed scene structs.

use std::path::{Path, PathBuf};

use crate::error::{bad_scene, io_err, Result};
use crate::math::{widen, widen3, widen4, V3};
use crate::scene::{AvatarInfo, Component, Face, FaceTrack, Joint as SceneJoint, Material, Scene};

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    pub parent: i32,
    pub bind_world: V3,
    pub rest_world: V3,
    pub rest_world_rot: [f64; 4],
    pub rest_local: V3,
    pub scale: V3,
}

impl Joint {
    fn from_scene(j: &SceneJoint) -> Self {
        Self {
            name: j.name.clone(),
            parent: j.parent,
            bind_world: widen3(j.bind_world),
            rest_world: widen3(j.rest_world),
            rest_world_rot: widen4(j.rest_world_rot),
            rest_local: widen3(j.rest_local),
            scale: widen3(j.scale),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub component: u32,
    pub material: usize,
    pub is_prop: bool,
    pub positions: Vec<V3>,
    pub normals: Vec<V3>,
    pub uvs: Vec<[f64; 2]>,
    /// RGBA in 0..1, absent when the scene carries no colours.
    pub colors: Option<Vec<[f64; 4]>>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f64; 4]>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub t: Vec<V3>,
    pub r: Vec<[f64; 4]>,
    pub s: Vec<V3>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    pub fps: f64,
    pub frame_count: u32,
    /// Indexed by position, matching the skeleton order.
    pub tracks: Vec<Track>,
    pub face: Option<FaceTrack>,
}

impl Clip {
    /// `fps or 30.0`.
    pub fn rate(&self) -> f64 {
        if self.fps == 0.0 {
            30.0
        } else {
            self.fps
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Avatar {
    /// Folder holding avatar.json and the PNGs it names.
    pub dir: PathBuf,
    pub info: AvatarInfo,
    pub skeleton: Vec<Joint>,
    pub components: Vec<Component>,
    pub materials: Vec<Material>,
    pub meshes: Vec<Mesh>,
    pub animations: Vec<Clip>,
    pub prop_skeleton: Option<Vec<Joint>>,
    pub face: Face,
}

fn chunks<const N: usize, T: Copy + Default, U>(
    data: &[T],
    n: usize,
    what: &str,
    mesh: &str,
    map: impl Fn(T) -> U,
) -> Result<Vec<[U; N]>> {
    if data.len() < n * N {
        return Err(bad_scene(format!(
            "mesh {mesh}: {what} has {} values, expected {}",
            data.len(),
            n * N
        )));
    }
    Ok(data[..n * N]
        .as_chunks::<N>()
        .0
        .iter()
        .map(|c| c.map(&map))
        .collect())
}

impl Scene {
    /// Reads and parses an avatar.json.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(io_err(path))?;
        Ok(Scene::from_json(&text)?)
    }
}

impl Avatar {
    /// Loads avatar.json; textures resolve against its folder.
    pub fn load(json_path: &Path) -> Result<Self> {
        let scene = Scene::load(json_path)?;
        let abs = std::path::absolute(json_path).map_err(io_err(json_path))?;
        let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
        Self::from_scene(&scene, dir)
    }

    pub fn from_scene(scene: &Scene, dir: PathBuf) -> Result<Self> {
        let mut meshes = Vec::with_capacity(scene.meshes.len());
        for m in &scene.meshes {
            let n = m.vertex_count as usize;
            let name = m.name.as_str();
            let material = m.material as usize;
            if material >= scene.materials.len() {
                return Err(bad_scene(format!(
                    "mesh {name}: material {material} out of range"
                )));
            }
            let colors = if m.colors.is_empty() {
                None
            } else {
                Some(chunks::<4, _, _>(&m.colors, n, "colors", name, |c| {
                    f64::from(c) / 255.0
                })?)
            };
            meshes.push(Mesh {
                name: m.name.clone(),
                component: m.component,
                material,
                is_prop: m.is_prop,
                positions: chunks::<3, _, _>(&m.positions, n, "positions", name, widen)?,
                normals: chunks::<3, _, _>(&m.normals, n, "normals", name, widen)?,
                uvs: chunks::<2, _, _>(&m.uv, n, "uv", name, widen)?,
                colors,
                joints: chunks::<4, _, _>(&m.joints, n, "joints", name, |j| j)?,
                weights: chunks::<4, _, _>(&m.weights, n, "weights", name, widen)?,
                indices: m.indices.clone(),
            });
        }
        let animations = scene
            .animations
            .iter()
            .map(|a| Clip {
                name: a.name.clone(),
                fps: widen(a.fps),
                frame_count: a.frame_count,
                tracks: a
                    .tracks
                    .iter()
                    .map(|t| Track {
                        t: t.t.iter().copied().map(widen3).collect(),
                        r: t.r.iter().copied().map(widen4).collect(),
                        s: t.s.iter().copied().map(widen3).collect(),
                    })
                    .collect(),
                face: a.face.clone(),
            })
            .collect();
        Ok(Self {
            dir,
            info: scene.avatar.clone(),
            skeleton: scene.skeleton.joints.iter().map(Joint::from_scene).collect(),
            components: scene.components.clone(),
            materials: scene.materials.clone(),
            meshes,
            animations,
            prop_skeleton: scene
                .prop_skeleton
                .as_ref()
                .map(|s| s.joints.iter().map(Joint::from_scene).collect()),
            face: scene.face.clone().unwrap_or_default(),
        })
    }

    pub fn joint_names(&self) -> Vec<&str> {
        self.skeleton.iter().map(|j| j.name.as_str()).collect()
    }

    /// Vertical extent of the non-prop meshes in metres, at least 1 cm.
    pub fn height_m(&self) -> f64 {
        let mut top: f64 = 0.0;
        let mut bottom: f64 = 1e9;
        for m in self.meshes.iter().filter(|m| !m.is_prop) {
            for p in &m.positions {
                top = top.max(p[1]);
                bottom = bottom.min(p[1]);
            }
        }
        (top - bottom).max(0.01)
    }

    /// Path of a file named by the scene.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.animations.iter().find(|a| a.name == name)
    }
}
