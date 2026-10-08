// egui host for `avatar_view::Viewer`: renders into an offscreen texture that egui samples, with
// OrbitControls-style mouse input and the free-pose bone markers drawn on top.

use std::path::Path;
use std::sync::Arc;

use avatar_export::scene::Scene;
use avatar_view::Viewer;
use eframe::egui::{self, Color32, Rect, Sense, Stroke, Vec2};

use crate::widgets::{self, Paint};
use eframe::egui_wgpu;
use eframe::wgpu;

/// Colour format of every viewer target.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Device and queue a worker needs to build a viewer off the UI thread.
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// A viewer with `scene` uploaded and framed.
    pub fn build_viewer(&self, scene: &Scene, dir: &Path) -> anyhow::Result<Viewer> {
        let mut viewer = Viewer::new(&self.device, &self.queue, FORMAT, true);
        viewer.load_scene(scene, dir)?;
        Ok(viewer)
    }
}

struct Target {
    view: wgpu::TextureView,
    size: [u32; 2],
    id: egui::TextureId,
}

/// One marker drawn over the viewport.
pub struct Marker {
    pub joint: usize,
    pub selected: bool,
}

#[derive(Default)]
pub struct ViewportResponse {
    /// Joint whose marker was clicked.
    pub picked: Option<usize>,
}

pub struct Viewport {
    gpu: Gpu,
    renderer: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    pub viewer: Option<Viewer>,
    target: Option<Target>,
}

impl Viewport {
    pub fn new(state: &egui_wgpu::RenderState) -> Self {
        Self {
            gpu: Gpu {
                device: state.device.clone(),
                queue: state.queue.clone(),
            },
            renderer: state.renderer.clone(),
            viewer: None,
            target: None,
        }
    }

    pub fn gpu(&self) -> Gpu {
        self.gpu.clone()
    }

    /// Swaps in a rebuilt viewer, keeping the camera when `keep_camera` is set.
    pub fn replace(&mut self, mut viewer: Viewer, keep_camera: bool) {
        if keep_camera {
            if let Some(old) = &self.viewer {
                *viewer.camera_mut() = old.camera().clone();
            }
        }
        self.viewer = Some(viewer);
    }

    fn ensure_target(&mut self, size: [u32; 2]) -> Option<(&wgpu::TextureView, egui::TextureId)> {
        let stale = self.target.as_ref().is_none_or(|t| t.size != size);
        if stale {
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("aura viewport"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut renderer = self.renderer.write();
            let id = match &self.target {
                Some(old) => {
                    renderer.update_egui_texture_from_wgpu_texture(
                        &self.gpu.device,
                        &view,
                        wgpu::FilterMode::Linear,
                        old.id,
                    );
                    old.id
                }
                None => renderer.register_native_texture(&self.gpu.device, &view, wgpu::FilterMode::Linear),
            };
            self.target = Some(Target { view, size, id });
        }
        self.target.as_ref().map(|t| (&t.view, t.id))
    }

    /// Draws the viewport into `rect`, handling orbit (left drag), pan (right or middle drag) and zoom (wheel).
    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect, markers: &[Marker]) -> ViewportResponse {
        let id = ui.id().with("viewport");
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let painter = ui.painter_at(rect.expand(1.0));
        let mut out = ViewportResponse::default();
        let Some(viewer) = self.viewer.as_mut() else {
            stage_frame(&painter, rect, None);
            return out;
        };
        let height = rect.height().max(1.0);
        if response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            let tau = std::f32::consts::TAU;
            viewer.camera_mut().orbit(-tau * d.x / height, tau * d.y / height);
        } else if response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            let d = response.drag_delta();
            viewer.camera_mut().pan_pixels(d.x, d.y, height);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                viewer.camera_mut().zoom((-scroll * 0.002).exp());
            }
        }

        let ppp = ui.ctx().pixels_per_point();
        let size = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        let gpu = self.gpu.clone();
        let Some((view, tex)) = self.ensure_target(size) else {
            return out;
        };
        let view = view.clone();
        let Some(viewer) = self.viewer.as_mut() else {
            return out;
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("aura viewport"),
            });
        viewer.render(&mut encoder, &view, size[0], size[1]);
        gpu.queue.submit([encoder.finish()]);
        stage_frame(&painter, rect, Some(tex));

        if !markers.is_empty() {
            let positions = viewer.bone_screen_positions();
            let to_ui = |p: glam::Vec2| rect.min + Vec2::new(p.x, p.y) / ppp;
            let click = response
                .clicked()
                .then(|| response.interact_pointer_pos())
                .flatten();
            let mut nearest: Option<(usize, f32)> = None;
            for marker in markers {
                let Some(Some(p)) = positions.get(marker.joint) else {
                    continue;
                };
                let at = to_ui(*p);
                if !rect.contains(at) {
                    continue;
                }
                let fill = if marker.selected {
                    Color32::from_rgb(0xE8, 0x8A, 0x1A)
                } else {
                    Color32::from_rgb(0x77, 0xAA, 0x35)
                };
                painter.circle(
                    at,
                    6.0,
                    fill,
                    Stroke::new(2.0, Color32::from_rgb(0xF5, 0xFF, 0xE9)),
                );
                if let Some(c) = click {
                    let d = c.distance(at);
                    if d <= 18.0 && nearest.is_none_or(|(_, best)| d < best) {
                        nearest = Some((marker.joint, d));
                    }
                }
            }
            out.picked = nearest.map(|(j, _)| j);
        }
        out
    }
}

/// `.viewport`: radial #fff to #e0e8d9 stage (or the rendered frame) inside a 1px #cbd9be border, 10px radius.
fn stage_frame(painter: &egui::Painter, rect: Rect, frame: Option<egui::TextureId>) {
    match frame {
        Some(tex) => widgets::image_rounded(painter, rect, 10.0, tex),
        None => widgets::fill(
            painter,
            rect,
            10.0,
            &Paint::Radial(
                Vec2::new(0.5, 0.35),
                &[(0.0, Color32::WHITE), (1.0, widgets::rgb(0xE0E8D9))],
            ),
        ),
    }
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, widgets::rgb(0xCBD9BE)),
        egui::StrokeKind::Inside,
    );
}

impl Drop for Viewport {
    fn drop(&mut self) {
        if let Some(target) = self.target.take() {
            self.renderer.write().free_texture(&target.id);
        }
    }
}
