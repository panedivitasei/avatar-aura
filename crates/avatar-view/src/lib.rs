// wgpu viewport for baked avatar scenes, replacing the three.js viewport of the Python app.
// Pure wgpu: the host supplies device, queue and target; GPU skinning reads a per-joint palette.

#![forbid(unsafe_code)]

pub mod camera;
pub mod gizmo;
mod gpu;
pub mod pose;

use std::path::{Path, PathBuf};

use avatar_export::avatar::Avatar;
use avatar_export::math::{mat_trans, safe_id, M4};
use avatar_export::posing::{from_local_pose, PoseBone};
use avatar_export::scene::Scene;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use image::RgbaImage;

pub use camera::Camera;
pub use gizmo::{Gizmo, GizmoMode, Handle, PoseEdit};
use gizmo::{JointFrame, View};
use gpu::{GpuMaterial, GpuMesh, OverlayVertex, Pipelines, Targets};
use pose::{clip_locals_at, from_mat4, to_mat4, PoseState};

#[derive(Debug, thiserror::Error)]
pub enum ViewError {
    #[error(transparent)]
    Scene(#[from] avatar_export::Error),
    #[error("{path}: {source}")]
    Texture {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("mesh {mesh}: {reason}")]
    Mesh { mesh: String, reason: String },
    #[error("no scene is loaded")]
    NoScene,
    #[error("clip index {0} is out of range")]
    ClipIndex(usize),
    #[error("material index {0} is out of range")]
    MaterialIndex(usize),
    #[error("expected {expected} joint matrices, got {got}")]
    JointCount { expected: usize, got: usize },
    #[error("readback of {0:?} targets is not supported")]
    Format(wgpu::TextureFormat),
    #[error("readback failed: {0}")]
    Readback(String),
}

pub type Result<T> = std::result::Result<T, ViewError>;

/// Clip whose frame 1 the Python preview sends as the initial pose.
pub const LOAD_POSE_CLIP: &str = "Animation Generic Stand 1";

/// Axis-aligned box in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

pub struct Viewer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    color_format: wgpu::TextureFormat,
    pipelines: Pipelines,
    frame_buffer: wgpu::Buffer,
    palette_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    meshes: Vec<GpuMesh>,
    materials: Vec<GpuMaterial>,
    pose: Option<PoseState>,
    palette: Vec<Mat4>,
    camera: Camera,
    /// Floor contact blob: centre x, floor y, centre z, avatar height; zero height hides it.
    contact: Vec4,
    targets: Option<Targets>,
    last_size: (u32, u32),
    gizmo: Gizmo,
}

impl Viewer {
    /// `depth` enables the depth buffer; without it meshes draw in submission order.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        depth: bool,
    ) -> Self {
        let pipelines = Pipelines::new(device, color_format, depth);
        let frame_buffer = gpu::frame_buffer(device);
        let palette_buffer = gpu::palette_buffer(device, &[Mat4::IDENTITY]);
        let frame_group = pipelines.frame_group(device, &frame_buffer, &palette_buffer);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            color_format,
            pipelines,
            frame_buffer,
            palette_buffer,
            frame_group,
            meshes: Vec::new(),
            materials: Vec::new(),
            pose: None,
            palette: vec![Mat4::IDENTITY],
            camera: Camera::default(),
            contact: Vec4::ZERO,
            targets: None,
            last_size: (1, 1),
            gizmo: Gizmo::default(),
        }
    }

    /// Uploads meshes and textures, then poses and frames the avatar as viewport.js does on load.
    /// The load pose is frame 1 of "Animation Generic Stand 1" when the scene carries it, else the rest pose.
    pub fn load_scene(&mut self, scene: &Scene, dir: &Path) -> Result<()> {
        let avatar = Avatar::from_scene(scene, dir.to_path_buf())?;
        let state = PoseState::new(avatar)?;
        let mut materials = Vec::with_capacity(scene.materials.len());
        for material in &scene.materials {
            let path = dir.join(&material.diffuse);
            let image = if !material.diffuse.is_empty() && path.is_file() {
                let decoded = image::open(&path).map_err(|source| ViewError::Texture { path, source })?;
                Some(decoded.to_rgba8())
            } else {
                None
            };
            materials.push(GpuMaterial::new(
                &self.device,
                &self.queue,
                &self.pipelines,
                material,
                image.as_ref(),
            ));
        }
        let mut meshes = Vec::with_capacity(scene.meshes.len());
        for (mesh, source) in state.avatar.meshes.iter().zip(&scene.meshes) {
            if let Some(gpu) = GpuMesh::new(&self.device, &state.rig, mesh, &source.colors)? {
                meshes.push(gpu);
            }
        }
        self.materials = materials;
        self.meshes = meshes;
        self.palette_buffer = gpu::palette_buffer(&self.device, &state.palette());
        self.frame_group = self
            .pipelines
            .frame_group(&self.device, &self.frame_buffer, &self.palette_buffer);
        let names: Vec<&str> = state.rig.bones.iter().map(|b| b.name.as_str()).collect();
        self.gizmo = Gizmo::for_joints(&names);
        self.pose = Some(state);
        self.upload_pose();
        if let Some(stand) = self.clip_index(LOAD_POSE_CLIP) {
            self.set_clip_frame(stand, 1.0)?;
        }
        self.fit_to_scene();
        Ok(())
    }

    pub fn has_scene(&self) -> bool {
        self.pose.is_some()
    }

    pub fn joint_names(&self) -> Vec<&str> {
        self.pose
            .as_ref()
            .map(|p| p.rig.bones.iter().map(|b| b.name.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn clip_names(&self) -> Vec<&str> {
        self.pose
            .as_ref()
            .map(|p| p.avatar.animations.iter().map(|c| c.name.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn clip_index(&self, name: &str) -> Option<usize> {
        self.pose
            .as_ref()?
            .avatar
            .animations
            .iter()
            .position(|c| c.name == name)
    }

    /// Bind pose of the rig.
    pub fn set_rest_pose(&mut self) {
        if let Some(state) = self.pose.as_mut() {
            state.rest();
            self.upload_pose();
            self.rebase_gizmo();
        }
    }

    /// Samples a clip at a fractional frame, clamped to the clip, interpolating between neighbouring frames.
    pub fn set_clip_frame(&mut self, clip_index: usize, frame: f32) -> Result<()> {
        let state = self.pose.as_mut().ok_or(ViewError::NoScene)?;
        let clip = state
            .avatar
            .animations
            .get(clip_index)
            .ok_or(ViewError::ClipIndex(clip_index))?;
        let locals = clip_locals_at(&state.avatar, &state.rig, clip, frame)?;
        state.set_locals(locals);
        self.upload_pose();
        self.rebase_gizmo();
        Ok(())
    }

    /// Rig-local matrices for every joint, parents first, as the free-pose editor edits them.
    pub fn set_joint_locals(&mut self, locals: &[Mat4]) -> Result<()> {
        let state = self.pose.as_mut().ok_or(ViewError::NoScene)?;
        if locals.len() != state.rig.bones.len() {
            return Err(ViewError::JointCount {
                expected: state.rig.bones.len(),
                got: locals.len(),
            });
        }
        state.set_locals(locals.iter().map(from_mat4).collect());
        self.upload_pose();
        self.rebase_gizmo();
        Ok(())
    }

    /// Free pose in the serialized editor form, validated by `avatar_export::posing::from_local_pose`.
    pub fn set_free_pose(&mut self, bones: &[PoseBone]) -> Result<()> {
        let state = self.pose.as_mut().ok_or(ViewError::NoScene)?;
        let rig = from_local_pose(&state.avatar, bones)?;
        state.set_locals(rig.bones.iter().map(|b| b.local).collect());
        self.upload_pose();
        self.rebase_gizmo();
        Ok(())
    }

    pub fn joint_locals(&self) -> Vec<Mat4> {
        self.pose
            .as_ref()
            .map(|p| p.locals.iter().map(to_mat4).collect())
            .unwrap_or_default()
    }

    pub fn joint_worlds(&self) -> Vec<Mat4> {
        self.pose
            .as_ref()
            .map(|p| p.worlds.iter().map(to_mat4).collect())
            .unwrap_or_default()
    }

    /// Current skinning palette, `world * inverse(bind)` per joint.
    pub fn palette(&self) -> &[Mat4] {
        &self.palette
    }

    /// Replaces a material's diffuse map, for expressions; the original stays cached.
    pub fn set_material_texture(&mut self, material_index: usize, rgba: &RgbaImage) -> Result<()> {
        let material = self
            .materials
            .get_mut(material_index)
            .ok_or(ViewError::MaterialIndex(material_index))?;
        material.set_override(&self.device, &self.queue, &self.pipelines, rgba);
        Ok(())
    }

    pub fn reset_material_texture(&mut self, material_index: usize) -> Result<()> {
        let material = self
            .materials
            .get_mut(material_index)
            .ok_or(ViewError::MaterialIndex(material_index))?;
        material.clear_override();
        Ok(())
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Skinned bounds of every mesh in the current pose.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for mesh in &self.meshes {
            for p in mesh.skinned_positions(&self.palette) {
                min = min.min(p);
                max = max.max(p);
            }
        }
        (min.x <= max.x).then_some(Bounds { min, max })
    }

    /// viewport.js framing: target the box centre, eye on +Z at 2.1 times the larger of height and width, raised 6% of the height.
    pub fn fit_to_scene(&mut self) {
        let Some(bounds) = self.bounds() else {
            return;
        };
        let center = bounds.center();
        let size = bounds.size();
        let eye = Vec3::new(
            center.x,
            center.y + size.y * 0.06,
            center.z + size.y.max(size.x) * 2.1,
        );
        self.camera.look_from(eye, center);
        self.contact = Vec4::new(center.x, bounds.min.y - 0.01, center.z, size.y);
    }

    /// Each joint's position in pixels of the last rendered size, `None` outside the depth range.
    pub fn bone_screen_positions(&self) -> Vec<Option<Vec2>> {
        let view = self.view();
        self.joint_positions()
            .into_iter()
            .map(|p| view.project_in_depth(p))
            .collect()
    }

    /// Sets the pixel size picking and `bone_screen_positions` use; `render` sets it too.
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.last_size = (width.max(1), height.max(1));
    }

    // ---- free-pose gizmo ----

    pub fn gizmo(&self) -> &Gizmo {
        &self.gizmo
    }

    /// Selection and mode; `Gizmo::select` and `Gizmo::set_mode` enforce the bones.js rules.
    pub fn gizmo_mut(&mut self) -> &mut Gizmo {
        &mut self.gizmo
    }

    /// Turning free pose on captures the current pose as the reset target, clears edits and selects the head.
    /// Turning it off hides markers and handles and keeps the edited pose.
    pub fn set_gizmo_enabled(&mut self, enabled: bool) {
        if enabled && !self.gizmo.enabled() {
            self.gizmo.base = self.pose.as_ref().map(|p| p.locals.clone()).unwrap_or_default();
            self.gizmo.edits.clear();
        }
        if enabled != self.gizmo.enabled() {
            self.gizmo.set_enabled(enabled);
        }
    }

    /// Nearest listed joint marker under a viewport pixel, as the bones.js pointer handler picks.
    pub fn pick_joint(&self, x: f32, y: f32) -> Option<usize> {
        self.gizmo
            .pick_marker(&self.view(), &self.joint_positions(), Vec2::new(x, y))
    }

    /// Updates handle hover; true when a handle or marker is under the pointer.
    pub fn gizmo_hover(&mut self, x: f32, y: f32) -> bool {
        if !self.gizmo.enabled() {
            return false;
        }
        if self.gizmo.is_dragging() {
            return true;
        }
        let px = Vec2::new(x, y);
        let view = self.view();
        let handle = self
            .selected_frame()
            .and_then(|frame| self.gizmo.pick_handle(&view, &frame, px));
        self.gizmo.set_hovered(handle);
        handle.is_some()
            || self
                .gizmo
                .pick_marker(&view, &self.joint_positions(), px)
                .is_some()
    }

    /// Starts a handle drag, or selects the marker's joint and arms a free drag on it.
    /// False when nothing was hit, leaving the press to the camera.
    pub fn gizmo_press(&mut self, x: f32, y: f32) -> bool {
        if !self.gizmo.enabled() || self.pose.is_none() {
            return false;
        }
        let px = Vec2::new(x, y);
        let view = self.view();
        let on_handle = self
            .selected_frame()
            .and_then(|frame| self.gizmo.pick_handle(&view, &frame, px))
            .and_then(|handle| Some((self.gizmo.selected()?, handle)));
        let hit = on_handle.or_else(|| {
            let joint = self.gizmo.pick_marker(&view, &self.joint_positions(), px)?;
            Some((joint, Handle::Free))
        });
        let Some((joint, handle)) = hit else {
            return false;
        };
        if self.gizmo.selected() != Some(joint) {
            self.gizmo.select(joint);
        }
        if let (Some(frame), Some(local)) = (self.joint_frame(joint), self.joint_local(joint)) {
            self.gizmo.begin_drag(&view, joint, frame, handle, local, px);
        }
        true
    }

    /// Moves the active handle; true when the pose changed.
    pub fn gizmo_drag(&mut self, x: f32, y: f32) -> bool {
        let view = self.view();
        let Some((joint, rotation, translation)) = self.gizmo.drag_to(&view, Vec2::new(x, y)) else {
            return false;
        };
        self.ensure_gizmo_base();
        self.gizmo
            .edits
            .insert(joint, PoseEdit::from_local(joint, rotation, translation));
        self.apply_pose_edits().is_ok()
    }

    pub fn gizmo_release(&mut self) {
        self.gizmo.end_drag();
    }

    /// A pixel on a handle of the selected joint's gizmo, for hit tests and tooltips.
    pub fn gizmo_handle_position(&self, handle: Handle) -> Option<Vec2> {
        let frame = self.selected_frame()?;
        self.gizmo.handle_position(&self.view(), &frame, handle)
    }

    /// Edited joints with their current values, by joint index.
    pub fn pose_edits(&self) -> Vec<PoseEdit> {
        self.gizmo.edits.values().copied().collect()
    }

    /// Current local rotation and position of any joint, edited or not, for the X/Y/Z fields.
    pub fn joint_pose(&self, joint: usize) -> Option<PoseEdit> {
        let (rotation, translation) = self.joint_local(joint)?;
        Some(PoseEdit::from_local(joint, rotation, translation))
    }

    /// Writes the numeric fields' values for `joint` and reposes.
    pub fn set_pose_edit(&mut self, joint: usize, edit: PoseEdit) -> Result<()> {
        let count = self.pose.as_ref().ok_or(ViewError::NoScene)?.locals.len();
        if joint >= count {
            return Err(ViewError::JointCount {
                expected: count,
                got: joint + 1,
            });
        }
        self.ensure_gizmo_base();
        self.gizmo.edits.insert(joint, PoseEdit { joint, ..edit });
        self.apply_pose_edits()
    }

    /// Reset bone: the joint returns to the pose captured when free pose turned on.
    pub fn reset_joint(&mut self, joint: usize) -> Result<()> {
        self.gizmo.edits.remove(&joint);
        self.apply_pose_edits()
    }

    /// Reset pose: every joint returns to the captured pose.
    pub fn reset_pose(&mut self) -> Result<()> {
        self.gizmo.edits.clear();
        self.apply_pose_edits()
    }

    /// Base pose with the edits applied, through `from_local_pose` as `set_free_pose` applies a serialized pose.
    fn apply_pose_edits(&mut self) -> Result<()> {
        self.ensure_gizmo_base();
        let state = self.pose.as_mut().ok_or(ViewError::NoScene)?;
        let bones: Vec<PoseBone> = state
            .rig
            .bones
            .iter()
            .zip(&self.gizmo.base)
            .enumerate()
            .map(|(i, (bone, base))| {
                let (rotation, translation) = match self.gizmo.edits.get(&i) {
                    Some(edit) => (edit.rotation(), edit.translation),
                    None => {
                        let (_, r, t) = to_mat4(base).to_scale_rotation_translation();
                        (r, t)
                    }
                };
                PoseBone {
                    name: safe_id(&bone.name),
                    position: translation.to_array().map(f64::from),
                    rotation: rotation.to_array().map(f64::from),
                }
            })
            .collect();
        let rig = from_local_pose(&state.avatar, &bones)?;
        state.set_locals(rig.bones.iter().map(|b| b.local).collect());
        self.upload_pose();
        Ok(())
    }

    fn ensure_gizmo_base(&mut self) {
        if let Some(state) = self.pose.as_ref() {
            if self.gizmo.base.len() != state.locals.len() {
                self.gizmo.base = state.locals.clone();
            }
        }
    }

    /// An external pose replaces the reset target while free pose is on.
    fn rebase_gizmo(&mut self) {
        if self.gizmo.enabled() {
            self.gizmo.base = self.pose.as_ref().map(|p| p.locals.clone()).unwrap_or_default();
            self.gizmo.edits.clear();
            self.gizmo.end_drag();
        }
    }

    fn view(&self) -> View {
        View::new(&self.camera, self.last_size.0, self.last_size.1)
    }

    fn joint_positions(&self) -> Vec<Vec3> {
        let point = |m: &M4| {
            let t = mat_trans(m);
            Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32)
        };
        self.pose
            .as_ref()
            .map(|p| p.worlds.iter().map(point).collect())
            .unwrap_or_default()
    }

    fn joint_local(&self, joint: usize) -> Option<(Quat, Vec3)> {
        let local = self.pose.as_ref()?.locals.get(joint)?;
        let (_, r, t) = to_mat4(local).to_scale_rotation_translation();
        Some((r, t))
    }

    fn joint_frame(&self, joint: usize) -> Option<JointFrame> {
        let state = self.pose.as_ref()?;
        let rotation_of = |m: &M4| to_mat4(m).to_scale_rotation_translation().1;
        let world = state.worlds.get(joint)?;
        let parent_rotation = usize::try_from(state.rig.bones.get(joint)?.parent)
            .ok()
            .and_then(|p| state.worlds.get(p))
            .map_or(Quat::IDENTITY, rotation_of);
        let t = mat_trans(world);
        Some(JointFrame {
            center: Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32),
            rotation: rotation_of(world),
            parent_rotation,
        })
    }

    fn selected_frame(&self) -> Option<JointFrame> {
        self.joint_frame(self.gizmo.selected()?)
    }

    fn overlay_vertices(&self, width: u32, height: u32) -> Vec<OverlayVertex> {
        if !self.gizmo.enabled() {
            return Vec::new();
        }
        let view = View::new(&self.camera, width, height);
        self.gizmo
            .overlay(&view, &self.joint_positions(), self.selected_frame().as_ref())
    }

    /// Records the viewport into `view`, a `color_format` target of `width` x `height`.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let (width, height) = (width.max(1), height.max(1));
        self.last_size = (width, height);
        let reuse = self.targets.as_ref().is_some_and(|t| t.size == (width, height));
        if !reuse {
            self.targets = Some(Targets::new(&self.device, &self.pipelines, width, height));
        }
        self.queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(&self.frame_uniform(width, height)),
        );
        let overlay = gpu::overlay_buffer(&self.device, &self.overlay_vertices(width, height));
        if let Some(targets) = self.targets.as_ref() {
            self.encode(encoder, view, targets, &self.frame_group, overlay.as_ref());
        }
    }

    /// Renders offscreen with the current camera and pose and reads the pixels back.
    pub fn render_to_rgba(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage> {
        let bgra = match self.color_format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
            other => return Err(ViewError::Format(other)),
        };
        let (width, height) = (width.max(1), height.max(1));
        let frame_buffer = gpu::frame_buffer(device);
        queue.write_buffer(
            &frame_buffer,
            0,
            bytemuck::bytes_of(&self.frame_uniform(width, height)),
        );
        let group = self
            .pipelines
            .frame_group(device, &frame_buffer, &self.palette_buffer);
        let targets = Targets::new(device, &self.pipelines, width, height);
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("avatar-view readback"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("avatar-view offscreen"),
        });
        let overlay = gpu::overlay_buffer(device, &self.overlay_vertices(width, height));
        self.encode(&mut encoder, &view, &targets, &group, overlay.as_ref());
        let mut pixels = gpu::read_texture(device, queue, encoder, &color, width, height)?;
        if bgra {
            for px in pixels.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| ViewError::Readback("pixel buffer size mismatch".into()))
    }

    fn frame_uniform(&self, width: u32, height: u32) -> gpu::FrameUniform {
        gpu::FrameUniform {
            view_proj: self.camera.view_projection(width, height).to_cols_array_2d(),
            eye: self.camera.eye().extend(1.0).to_array(),
            params: [
                if self.color_format.is_srgb() { 0.0 } else { 1.0 },
                width as f32,
                height as f32,
                0.0,
            ],
            contact: self.contact.to_array(),
        }
    }

    fn upload_pose(&mut self) {
        if let Some(state) = self.pose.as_ref() {
            self.palette = state.palette();
            self.queue.write_buffer(
                &self.palette_buffer,
                0,
                bytemuck::cast_slice(&gpu::palette_bytes(&self.palette)),
            );
        }
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        targets: &Targets,
        frame_group: &wgpu::BindGroup,
        overlay: Option<&(wgpu::Buffer, u32)>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("avatar-view"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: targets.msaa.as_ref().map_or(view, |(msaa, _)| msaa),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: targets.depth.as_ref().map(|depth| {
                wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, frame_group, &[]);
        pass.set_pipeline(&self.pipelines.background);
        pass.draw(0..3, 0..1);
        self.draw_meshes(&mut pass, false);
        if self.contact.w > 0.0 {
            pass.set_pipeline(&self.pipelines.contact);
            pass.draw(0..6, 0..1);
        }
        self.draw_meshes(&mut pass, true);
        if let Some((buffer, count)) = overlay {
            pass.set_pipeline(&self.pipelines.overlay);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..*count, 0..1);
        }
        drop(pass);
        if let Some((_, group)) = targets.msaa.as_ref() {
            self.resolve(encoder, view, group);
        }
    }

    /// Resolves the samples into `view` with a draw; pass resolve targets read back blank on some D3D12 drivers.
    fn resolve(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("avatar-view resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipelines.resolve);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn draw_meshes(&self, pass: &mut wgpu::RenderPass<'_>, blended: bool) {
        for mesh in &self.meshes {
            let Some(material) = self.materials.get(mesh.material) else {
                continue;
            };
            if material.blended != blended {
                continue;
            }
            pass.set_pipeline(self.pipelines.mesh(blended, material.double_sided));
            pass.set_bind_group(1, material.bind_group(), &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
    }
}
