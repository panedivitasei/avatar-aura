// Orbit camera with the viewport.js PerspectiveCamera(32, aspect, 0.01, 100) projection.

use glam::camera::rh::{proj::directx::perspective, view::look_at_mat4};
use glam::{Mat4, Vec3};

/// Vertical field of view of the three.js camera, degrees.
pub const FOV_Y_DEGREES: f32 = 32.0;
const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 1e-3;

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    /// Radians around +Y; zero looks down -Z from the avatar's front.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    pub distance: f32,
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.9, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            distance: 4.0,
            fov_y_degrees: FOV_Y_DEGREES,
            near: 0.01,
            far: 100.0,
        }
    }
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + self.distance * Vec3::new(cp * sy, sp, cp * cy)
    }

    /// Places the eye at `eye` looking at `target`.
    pub fn look_from(&mut self, eye: Vec3, target: Vec3) {
        let d = eye - target;
        self.target = target;
        self.distance = d.length().max(1e-4);
        self.yaw = d.x.atan2(d.z);
        self.pitch = (d.y / self.distance)
            .clamp(-1.0, 1.0)
            .asin()
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    pub fn view(&self) -> Mat4 {
        look_at_mat4(self.eye(), self.target, Vec3::Y)
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        perspective(
            self.fov_y_degrees.to_radians(),
            aspect.max(1e-4),
            self.near,
            self.far,
        )
    }

    pub fn view_projection(&self, width: u32, height: u32) -> Mat4 {
        self.projection(width.max(1) as f32 / height.max(1) as f32) * self.view()
    }

    /// Rotates around the target; positive `dyaw` swings the eye to the right.
    pub fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Multiplies the distance; values below one move closer.
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance * factor).clamp(self.near * 10.0, self.far * 0.5);
        }
    }

    /// Moves the target in the view plane by a pixel delta, OrbitControls-style: one viewport height spans the visible height at the target.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        let span = 2.0 * self.distance * (self.fov_y_degrees.to_radians() * 0.5).tan();
        let scale = span / viewport_height.max(1.0);
        let view = self.view();
        let right = view.row(0).truncate();
        let up = view.row(1).truncate();
        self.target += (-dx * right + dy * up) * scale;
    }
}
