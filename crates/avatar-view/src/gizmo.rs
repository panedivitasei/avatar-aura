// Free-pose bone gizmo after bones.js: joint marker sprites plus TransformControls rotate rings and move arrows.
// Geometry is rebuilt in world space every frame, scaled by camera distance so handles keep a constant on-screen size.

use std::collections::BTreeMap;

use avatar_export::math::M4;
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};

use crate::camera::Camera;
use crate::gpu::OverlayVertex;

/// Joints bones.js lists, by rig name, with their labels.
pub const JOINTS: [(&str, &str); 16] = [
    ("BASE", "Whole avatar"),
    ("BACKB", "Torso"),
    ("NECK", "Neck"),
    ("HEAD", "Head"),
    ("LF_S", "Left shoulder"),
    ("LF_E", "Left elbow"),
    ("LF_W", "Left wrist"),
    ("RT_S", "Right shoulder"),
    ("RT_E", "Right elbow"),
    ("RT_W", "Right wrist"),
    ("LF_H", "Left hip"),
    ("LF_K", "Left knee"),
    ("LF_A", "Left ankle"),
    ("RT_H", "Right hip"),
    ("RT_K", "Right knee"),
    ("RT_A", "Right ankle"),
];
/// The only joint that accepts Move.
pub const ROOT_JOINT: &str = "BASE";
/// Joint selected when free pose turns on.
pub const DEFAULT_JOINT: &str = "HEAD";
/// Marker sprite edge in world units, `marker.scale.setScalar(.042)`.
pub const MARKER_SIZE: f32 = 0.042;
/// Screen radius within which a press picks the nearest marker.
pub const MARKER_PICK_PIXELS: f32 = 18.0;
/// `TransformControls.setSize(.8)`.
pub const GIZMO_SIZE: f32 = 0.8;

const MARKER_FILL: [u8; 3] = [0x77, 0xaa, 0x35];
const MARKER_SELECTED: [u8; 3] = [0xe8, 0x8a, 0x1a];
const AXIS_COLORS: [[u8; 3]; 3] = [[0xff, 0, 0], [0, 0xff, 0], [0, 0, 0xff]];
const ACTIVE_COLOR: [u8; 3] = [0xff, 0xff, 0];
const FREE_COLOR: [u8; 3] = [0x78, 0x78, 0x78];

// Handle dimensions in TransformControls gizmo units.
const RING_RADIUS: f32 = 0.5;
const RING_PICK: f32 = 0.1;
const FREE_ROTATE_PICK: f32 = 0.25;
const FREE_MOVE_PICK: f32 = 0.2;
const ARROW_LENGTH: f32 = 0.5;
const ARROW_HEAD_LENGTH: f32 = 0.1;
const ARROW_HEAD_RADIUS: f32 = 0.04;
const ARROW_PICK_LENGTH: f32 = 0.6;
const ARROW_PICK: f32 = 0.12;
const MIN_PICK_PIXELS: f32 = 6.0;
const LINE_PIXELS: f32 = 2.5;
const RING_SEGMENTS: usize = 64;
/// Below this ray/plane cosine a ring drag falls back to the TransformControls linear mapping.
const OBLIQUE: f32 = 0.15;
const ROTATION_SPEED: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GizmoMode {
    #[default]
    Rotate,
    Translate,
}

/// A gizmo handle: one local axis, or the centre for free rotation and view-plane moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    X,
    Y,
    Z,
    Free,
}

impl Handle {
    pub const AXES: [Handle; 3] = [Handle::X, Handle::Y, Handle::Z];

    fn axis(self) -> Option<usize> {
        match self {
            Handle::X => Some(0),
            Handle::Y => Some(1),
            Handle::Z => Some(2),
            Handle::Free => None,
        }
    }
}

/// One joint's free-pose values as the X/Y/Z fields show them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseEdit {
    pub joint: usize,
    /// Local rotation in degrees, three.js Euler order XYZ, `bone.rotation` in bones.js.
    pub rotation_degrees: Vec3,
    /// Local position in metres, `bone.position`.
    pub translation: Vec3,
}

impl PoseEdit {
    pub fn from_local(joint: usize, rotation: Quat, translation: Vec3) -> Self {
        let (x, y, z) = rotation.to_euler(EulerRot::XYZ);
        Self {
            joint,
            rotation_degrees: Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI),
            translation,
        }
    }

    pub fn rotation(&self) -> Quat {
        let r = self.rotation_degrees * (std::f32::consts::PI / 180.0);
        Quat::from_euler(EulerRot::XYZ, r.x, r.y, r.z)
    }
}

/// Camera state for one viewport size, for projection and pointer rays.
pub(crate) struct View {
    vp: Mat4,
    inv: Mat4,
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    size: Vec2,
    tan_half: f32,
}

impl View {
    pub fn new(camera: &Camera, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let vp = camera.view_projection(width, height);
        let view = camera.view();
        Self {
            vp,
            inv: vp.inverse(),
            eye: camera.eye(),
            forward: -view.row(2).truncate(),
            right: view.row(0).truncate(),
            up: view.row(1).truncate(),
            size: Vec2::new(width as f32, height as f32),
            tan_half: (camera.fov_y_degrees.to_radians() * 0.5).tan(),
        }
    }

    /// Pixel position, `None` behind the eye.
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let clip = self.vp * p.extend(1.0);
        (clip.w > 1e-6).then(|| {
            let ndc = clip.truncate() / clip.w;
            Vec2::new(
                (ndc.x + 1.0) * 0.5 * self.size.x,
                (1.0 - ndc.y) * 0.5 * self.size.y,
            )
        })
    }

    /// Projection that also rejects points outside the depth range, as `bone_screen_positions` does.
    pub fn project_in_depth(&self, p: Vec3) -> Option<Vec2> {
        let clip = self.vp * p.extend(1.0);
        if clip.w <= 0.0 || !(0.0..=1.0).contains(&(clip.z / clip.w)) {
            return None;
        }
        self.project(p)
    }

    /// World ray through a pixel: origin and unit direction.
    fn ray(&self, px: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(px.x / self.size.x * 2.0 - 1.0, 1.0 - px.y / self.size.y * 2.0);
        let near = self.inv.project_point3(ndc.extend(0.0));
        let far = self.inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize_or(self.forward))
    }

    fn world_per_pixel(&self, p: Vec3) -> f32 {
        let depth = (p - self.eye).dot(self.forward).max(1e-4);
        2.0 * self.tan_half * depth / self.size.y
    }

    /// World length of one gizmo unit at `center`, as TransformControls scales its helper.
    fn gizmo_scale(&self, center: Vec3) -> f32 {
        (center - self.eye).length() * (1.9 * self.tan_half).min(7.0) * GIZMO_SIZE / 4.0
    }

    /// Unit vector from `p` to the eye, TransformControls `eye`.
    fn eye_dir(&self, p: Vec3) -> Vec3 {
        (self.eye - p).normalize_or(-self.forward)
    }
}

/// World placement of one joint and its parent.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JointFrame {
    pub center: Vec3,
    pub rotation: Quat,
    pub parent_rotation: Quat,
}

impl JointFrame {
    fn axis(&self, i: usize) -> Vec3 {
        self.rotation * Vec3::AXES[i]
    }
}

#[derive(Clone, Copy, Debug)]
enum DragKind {
    /// Ray/plane angle on the ring plane, unwrapped across frames.
    Ring {
        prev: Vec3,
        angle: f32,
    },
    /// Linear TransformControls mapping for a ring seen edge-on.
    RingLinear,
    FreeRotate,
    Arrow {
        s0: f32,
    },
    FreeMove,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    joint: usize,
    handle: Handle,
    kind: DragKind,
    frame: JointFrame,
    start_rotation: Quat,
    start_translation: Vec3,
    /// Press point on the camera-facing plane through the joint.
    start_hit: Vec3,
}

/// Free-pose editor state: selection, mode, handle hover and drag, and the per-joint edits.
#[derive(Debug, Default)]
pub struct Gizmo {
    enabled: bool,
    selected: Option<usize>,
    mode: GizmoMode,
    hovered: Option<Handle>,
    drag: Option<Drag>,
    listed: Vec<(usize, &'static str)>,
    root: Option<usize>,
    default: Option<usize>,
    pub(crate) base: Vec<M4>,
    pub(crate) edits: BTreeMap<usize, PoseEdit>,
}

impl Gizmo {
    pub(crate) fn for_joints(names: &[&str]) -> Self {
        let listed = JOINTS
            .iter()
            .filter_map(|(name, label)| names.iter().position(|n| n == name).map(|i| (i, *label)))
            .collect();
        Self {
            listed,
            root: names.iter().position(|n| *n == ROOT_JOINT),
            default: names.iter().position(|n| *n == DEFAULT_JOINT),
            ..Self::default()
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.drag = None;
        self.hovered = None;
        if enabled {
            self.selected = self.default.or(self.listed.first().map(|(j, _)| *j));
            self.mode = GizmoMode::Rotate;
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn mode(&self) -> GizmoMode {
        self.mode
    }

    pub fn hovered(&self) -> Option<Handle> {
        self.hovered
    }

    /// Handle under an active drag.
    pub fn active(&self) -> Option<Handle> {
        self.drag.map(|d| d.handle)
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Editable joints in bones.js order: rig index and label.
    pub fn listed_joints(&self) -> &[(usize, &'static str)] {
        &self.listed
    }

    pub fn root_joint(&self) -> Option<usize> {
        self.root
    }

    pub fn is_listed(&self, joint: usize) -> bool {
        self.listed.iter().any(|(j, _)| *j == joint)
    }

    /// Selects a listed joint; anything but the root drops back to Rotate. False for unlisted joints.
    pub fn select(&mut self, joint: usize) -> bool {
        if !self.is_listed(joint) {
            return false;
        }
        self.selected = Some(joint);
        if Some(joint) != self.root {
            self.mode = GizmoMode::Rotate;
        }
        self.drag = None;
        true
    }

    /// Translate is accepted only while the root is selected.
    pub fn set_mode(&mut self, mode: GizmoMode) -> bool {
        if mode == GizmoMode::Translate && (self.selected.is_none() || self.selected != self.root) {
            return false;
        }
        self.mode = mode;
        self.drag = None;
        true
    }

    /// Whether Move is available for the current selection.
    pub fn can_translate(&self) -> bool {
        self.selected.is_some() && self.selected == self.root
    }

    /// Nearest listed joint whose marker is within 18 px of `px` or whose sprite covers it.
    pub(crate) fn pick_marker(&self, view: &View, worlds: &[Vec3], px: Vec2) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for &(joint, _) in &self.listed {
            let Some(&p) = worlds.get(joint) else {
                continue;
            };
            let Some(at) = view.project_in_depth(p) else {
                continue;
            };
            let half = 0.5 * MARKER_SIZE / view.world_per_pixel(p);
            let d = at - px;
            let distance = d.length();
            let inside = d.x.abs() <= half && d.y.abs() <= half;
            if (distance <= MARKER_PICK_PIXELS || inside) && best.is_none_or(|(_, b)| distance < b) {
                best = Some((joint, distance));
            }
        }
        best.map(|(j, _)| j)
    }

    /// Handle of the selected joint's gizmo under `px`.
    pub(crate) fn pick_handle(&self, view: &View, frame: &JointFrame, px: Vec2) -> Option<Handle> {
        let scale = view.gizmo_scale(frame.center);
        let unit_px = scale / view.world_per_pixel(frame.center);
        let (axis_pick, free_pick) = match self.mode {
            GizmoMode::Rotate => (RING_PICK, FREE_ROTATE_PICK),
            GizmoMode::Translate => (ARROW_PICK, FREE_MOVE_PICK),
        };
        let threshold = (axis_pick * unit_px).max(MIN_PICK_PIXELS);
        let mut best: Option<(Handle, f32)> = None;
        for handle in Handle::AXES {
            let segments = match self.mode {
                GizmoMode::Rotate => ring_segments(view, frame, handle, scale),
                GizmoMode::Translate => {
                    let a = frame.axis(handle.axis().unwrap_or(0)) * ARROW_PICK_LENGTH * scale;
                    vec![(frame.center - a, frame.center + a)]
                }
            };
            for (a, b) in segments {
                let (Some(pa), Some(pb)) = (view.project(a), view.project(b)) else {
                    continue;
                };
                let d = segment_distance(px, pa, pb);
                if d <= threshold && best.is_none_or(|(_, b)| d < b) {
                    best = Some((handle, d));
                }
            }
        }
        if let Some((handle, _)) = best {
            return Some(handle);
        }
        let center = view.project(frame.center)?;
        (center.distance(px) <= free_pick * unit_px).then_some(Handle::Free)
    }

    pub(crate) fn set_hovered(&mut self, handle: Option<Handle>) {
        if self.drag.is_none() {
            self.hovered = handle;
        }
    }

    /// Starts a drag on `handle` of `joint`, whose current local rotation and translation are given.
    pub(crate) fn begin_drag(
        &mut self,
        view: &View,
        joint: usize,
        frame: JointFrame,
        handle: Handle,
        local: (Quat, Vec3),
        px: Vec2,
    ) {
        let (origin, dir) = view.ray(px);
        let eye = view.eye_dir(frame.center);
        let start_hit = plane_hit(origin, dir, frame.center, eye).unwrap_or(frame.center);
        let kind = match (self.mode, handle.axis()) {
            (_, None) if self.mode == GizmoMode::Rotate => DragKind::FreeRotate,
            (_, None) => DragKind::FreeMove,
            (GizmoMode::Rotate, Some(i)) => {
                let axis = frame.axis(i);
                match plane_hit(origin, dir, frame.center, axis) {
                    Some(hit) if dir.dot(axis).abs() > OBLIQUE => {
                        let v = hit - frame.center;
                        if v.length_squared() > 1e-12 {
                            DragKind::Ring {
                                prev: v.normalize(),
                                angle: 0.0,
                            }
                        } else {
                            DragKind::RingLinear
                        }
                    }
                    _ => DragKind::RingLinear,
                }
            }
            (GizmoMode::Translate, Some(i)) => DragKind::Arrow {
                s0: line_param(origin, dir, frame.center, frame.axis(i)).unwrap_or(0.0),
            },
        };
        self.hovered = Some(handle);
        self.drag = Some(Drag {
            joint,
            handle,
            kind,
            frame,
            start_rotation: local.0,
            start_translation: local.1,
            start_hit,
        });
    }

    /// New local rotation and translation of the dragged joint for the pointer at `px`.
    pub(crate) fn drag_to(&mut self, view: &View, px: Vec2) -> Option<(usize, Quat, Vec3)> {
        let drag = self.drag.as_mut()?;
        let frame = drag.frame;
        let (origin, dir) = view.ray(px);
        let eye = view.eye_dir(frame.center);
        let speed = ROTATION_SPEED / (frame.center - view.eye).length().max(1e-4);
        let offset = || plane_hit(origin, dir, frame.center, eye).map(|hit| hit - drag.start_hit);
        let parent_inv = frame.parent_rotation.inverse();
        let mut rotation = drag.start_rotation;
        let mut translation = drag.start_translation;
        match (&mut drag.kind, drag.handle.axis()) {
            (DragKind::Ring { prev, angle }, Some(i)) => {
                let axis = frame.axis(i);
                if dir.dot(axis).abs() > 1e-3 {
                    if let Some(hit) = plane_hit(origin, dir, frame.center, axis) {
                        let v = hit - frame.center;
                        if v.length_squared() > 1e-12 {
                            let v = v.normalize();
                            *angle += prev.cross(v).dot(axis).atan2(prev.dot(v));
                            *prev = v;
                        }
                    }
                }
                rotation = drag.start_rotation * Quat::from_axis_angle(Vec3::AXES[i], *angle);
            }
            (DragKind::RingLinear, Some(i)) => {
                let tangent = frame.axis(i).cross(eye).normalize_or_zero();
                let angle = offset().map_or(0.0, |o| o.dot(tangent) * speed);
                rotation = drag.start_rotation * Quat::from_axis_angle(Vec3::AXES[i], angle);
            }
            (DragKind::FreeRotate, _) => {
                let o = offset().unwrap_or(Vec3::ZERO);
                let axis = o.cross(eye).normalize_or_zero();
                if axis != Vec3::ZERO {
                    let angle = o.dot(axis.cross(eye).normalize_or_zero()) * speed;
                    rotation = Quat::from_axis_angle(parent_inv * axis, angle) * drag.start_rotation;
                }
            }
            (DragKind::Arrow { s0 }, Some(i)) => {
                let axis = frame.axis(i);
                if let Some(s) = line_param(origin, dir, frame.center, axis) {
                    translation = drag.start_translation + parent_inv * (axis * (s - *s0));
                }
            }
            (DragKind::FreeMove, _) => {
                translation = drag.start_translation + parent_inv * offset().unwrap_or(Vec3::ZERO);
            }
            _ => {}
        }
        Some((drag.joint, rotation.normalize(), translation))
    }

    pub(crate) fn end_drag(&mut self) {
        self.drag = None;
    }

    /// A pixel on `handle` of the selected gizmo: mid-arrow, the ring point nearest the eye, or the centre.
    pub(crate) fn handle_position(&self, view: &View, frame: &JointFrame, handle: Handle) -> Option<Vec2> {
        let scale = view.gizmo_scale(frame.center);
        let Some(i) = handle.axis() else {
            return view.project(frame.center);
        };
        match self.mode {
            GizmoMode::Translate => view.project(frame.center + frame.axis(i) * 0.35 * scale),
            GizmoMode::Rotate => {
                let axis = frame.axis(i);
                let eye = view.eye_dir(frame.center);
                let toward = (eye - axis * eye.dot(axis)).normalize_or(frame.axis((i + 1) % 3));
                view.project(frame.center + toward * RING_RADIUS * scale)
            }
        }
    }

    /// Triangles for the markers and the selected joint's handles, back-to-front markers first.
    pub(crate) fn overlay(
        &self,
        view: &View,
        worlds: &[Vec3],
        frame: Option<&JointFrame>,
    ) -> Vec<OverlayVertex> {
        let mut out = Vec::new();
        if !self.enabled {
            return out;
        }
        let mut markers: Vec<(usize, Vec3)> = self
            .listed
            .iter()
            .filter_map(|&(j, _)| worlds.get(j).map(|&p| (j, p)))
            .collect();
        markers.sort_by(|a, b| {
            let da = (a.1 - view.eye).dot(view.forward);
            let db = (b.1 - view.eye).dot(view.forward);
            db.total_cmp(&da)
        });
        let half = MARKER_SIZE * 0.5;
        for (joint, p) in markers {
            let fill = if Some(joint) == self.selected {
                MARKER_SELECTED
            } else {
                MARKER_FILL
            };
            // Canvas textures carry no colour space, so three.js treats the bytes as linear.
            let color = [fill[0], fill[1], fill[2]].map(|c| f32::from(c) / 255.0);
            let corner = |u: f32, v: f32| {
                OverlayVertex::marker(
                    p + (view.right * u + view.up * v) * half,
                    [u, v],
                    [color[0], color[1], color[2], 1.0],
                )
            };
            let (a, b, c, d) = (
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            );
            out.extend([a, b, c, a, c, d]);
        }
        let Some(frame) = frame else {
            return out;
        };
        let scale = view.gizmo_scale(frame.center);
        let highlight = self.drag.map(|d| d.handle).or(self.hovered);
        let color_of = |handle: Handle, base: [u8; 3]| {
            linear_color(if highlight == Some(handle) {
                ACTIVE_COLOR
            } else {
                base
            })
        };
        match self.mode {
            GizmoMode::Rotate => {
                let free = color_of(Handle::Free, FREE_COLOR);
                let ring = circle(frame.center, view.right, view.up, RING_RADIUS * scale);
                for pair in ring.windows(2) {
                    thick_segment(&mut out, view, pair[0], pair[1], free);
                }
                for handle in Handle::AXES {
                    let color = color_of(handle, AXIS_COLORS[handle.axis().unwrap_or(0)]);
                    for (a, b) in ring_segments(view, frame, handle, scale) {
                        thick_segment(&mut out, view, a, b, color);
                    }
                }
            }
            GizmoMode::Translate => {
                for handle in Handle::AXES {
                    let i = handle.axis().unwrap_or(0);
                    let color = color_of(handle, AXIS_COLORS[i]);
                    let axis = frame.axis(i);
                    let base = frame.center + axis * ARROW_LENGTH * scale;
                    thick_segment(&mut out, view, frame.center, base, color);
                    let tip = base + axis * ARROW_HEAD_LENGTH * scale;
                    let side = axis.cross(view.eye_dir(base)).normalize_or_zero() * ARROW_HEAD_RADIUS * scale;
                    if side != Vec3::ZERO {
                        out.extend([base - side, base + side, tip].map(|p| OverlayVertex::flat(p, color)));
                    }
                }
            }
        }
        out
    }
}

fn linear_color(c: [u8; 3]) -> [f32; 4] {
    let f = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(c[0]), f(c[1]), f(c[2]), 1.0]
}

fn circle(center: Vec3, u: Vec3, v: Vec3, radius: f32) -> Vec<Vec3> {
    (0..=RING_SEGMENTS)
        .map(|k| {
            let t = k as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
            center + (u * t.cos() + v * t.sin()) * radius
        })
        .collect()
}

/// The half of a local-axis ring facing the eye, as TransformControls draws it; whole when seen face-on.
fn ring_segments(view: &View, frame: &JointFrame, handle: Handle, scale: f32) -> Vec<(Vec3, Vec3)> {
    let i = handle.axis().unwrap_or(0);
    let axis = frame.axis(i);
    let eye = view.eye_dir(frame.center);
    let points = circle(
        frame.center,
        frame.axis((i + 1) % 3),
        frame.axis((i + 2) % 3),
        RING_RADIUS * scale,
    );
    let face_on = axis.dot(eye).abs() > 0.9;
    points
        .windows(2)
        .map(|w| (w[0], w[1]))
        .filter(|(a, b)| face_on || ((*a + *b) * 0.5 - frame.center).dot(eye) >= -0.02 * scale)
        .collect()
}

/// Screen-space quad of `LINE_PIXELS` width along a world segment.
fn thick_segment(out: &mut Vec<OverlayVertex>, view: &View, a: Vec3, b: Vec3, color: [f32; 4]) {
    let mid = (a + b) * 0.5;
    let side =
        (b - a).cross(view.eye - mid).normalize_or_zero() * (LINE_PIXELS * 0.5 * view.world_per_pixel(mid));
    if side == Vec3::ZERO {
        return;
    }
    let v = |p: Vec3| OverlayVertex::flat(p, color);
    out.extend([
        v(a - side),
        v(a + side),
        v(b + side),
        v(a - side),
        v(b + side),
        v(b - side),
    ]);
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-12 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Intersection of a ray with the plane through `point` with `normal`.
fn plane_hit(origin: Vec3, dir: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = dir.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (point - origin).dot(normal) / denom;
    (t > 0.0).then(|| origin + dir * t)
}

/// Parameter along `center + axis * s` of the point closest to the ray; `None` when they are parallel.
fn line_param(origin: Vec3, dir: Vec3, center: Vec3, axis: Vec3) -> Option<f32> {
    let w = center - origin;
    let b = axis.dot(dir);
    let denom = 1.0 - b * b;
    if denom < 1e-4 {
        return None;
    }
    Some((b * w.dot(dir) - w.dot(axis)) / denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_matches_three_xyz() {
        let edit = PoseEdit {
            joint: 0,
            rotation_degrees: Vec3::new(20.0, -35.0, 50.0),
            translation: Vec3::ZERO,
        };
        let r = edit.rotation_degrees * (std::f32::consts::PI / 180.0);
        let three = Quat::from_rotation_x(r.x) * Quat::from_rotation_y(r.y) * Quat::from_rotation_z(r.z);
        assert!(edit.rotation().abs_diff_eq(three, 1e-6));
        let back = PoseEdit::from_local(0, three, Vec3::ZERO);
        assert!(back.rotation_degrees.abs_diff_eq(edit.rotation_degrees, 1e-3));
    }

    #[test]
    fn line_param_finds_closest_point() {
        let s = line_param(Vec3::new(2.0, 5.0, 0.0), Vec3::NEG_Y, Vec3::ZERO, Vec3::X).unwrap();
        assert!((s - 2.0).abs() < 1e-5);
    }

    #[test]
    fn ring_drag_angle_follows_pointer() {
        let camera = Camera {
            target: Vec3::ZERO,
            distance: 2.0,
            ..Camera::default()
        };
        let view = View::new(&camera, 400, 400);
        let frame = JointFrame {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            parent_rotation: Quat::IDENTITY,
        };
        let mut gizmo = Gizmo::for_joints(&["BASE"]);
        gizmo.set_enabled(true);
        // Diagonal points keep clear of the edge-on X and Y rings, which project onto the screen axes.
        let center = view.project(Vec3::ZERO).unwrap();
        let r = gizmo.handle_position(&view, &frame, Handle::Z).unwrap().x - center.x;
        let h = std::f32::consts::FRAC_1_SQRT_2 * r;
        let start = center + Vec2::new(h, -h);
        assert_eq!(gizmo.pick_handle(&view, &frame, start), Some(Handle::Z));
        gizmo.begin_drag(&view, 0, frame, Handle::Z, (Quat::IDENTITY, Vec3::ZERO), start);
        let quarter = center + Vec2::new(-h, -h);
        let (_, rotation, _) = gizmo.drag_to(&view, quarter).unwrap();
        let (axis, angle) = rotation.to_axis_angle();
        assert!(axis.z.abs() > 0.99, "{axis:?}");
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 0.05, "{angle}");
    }
}
