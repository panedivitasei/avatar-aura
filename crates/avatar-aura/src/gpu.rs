// Backend choice: D3D12 first, unless the adapter fails a mesh self-test, in which case Vulkan.
// Some D3D12 drivers (Radeon RX Vega M here) drop the viewer's mesh draws once a target exceeds about 32k pixels.

use avatar_export::scene::{Joint, Material, Mesh, Scene, Skeleton};
use eframe::wgpu;

use crate::viewport::{Gpu, FORMAT};

/// Side of the self-test frame; large enough to cross the failing size.
const PROBE_SIZE: u32 = 256;

/// D3D12 first, then a discrete GPU.
pub fn rank(info: &wgpu::AdapterInfo) -> (bool, bool) {
    (
        info.backend != wgpu::Backend::Dx12,
        info.device_type != wgpu::DeviceType::DiscreteGpu,
    )
}

/// One joint and one double-sided quad facing the default camera.
fn probe_scene() -> Scene {
    let joint = Joint {
        name: "BASE".into(),
        parent: -1,
        rest_world_rot: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
        ..Joint::default()
    };
    let positions = vec![-0.4, 0.1, 0.0, 0.4, 0.1, 0.0, 0.4, 1.7, 0.0, -0.4, 1.7, 0.0];
    Scene {
        format: avatar_export::scene::FORMAT.into(),
        version: avatar_export::scene::VERSION,
        skeleton: Skeleton {
            version: 2,
            joints: vec![joint],
        },
        materials: vec![Material {
            name: "Probe".into(),
            double_sided: true,
            ..Material::default()
        }],
        meshes: vec![Mesh {
            name: "Probe".into(),
            vertex_count: 4,
            triangle_count: 2,
            uv_count: 1,
            positions,
            normals: [0.0, 0.0, 1.0].repeat(4),
            uv: vec![0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            colors: [180, 60, 40, 255].repeat(4),
            joints: vec![0; 16],
            weights: [1.0, 0.0, 0.0, 0.0].repeat(4),
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Mesh::default()
        }],
        ..Scene::default()
    }
}

/// True when a mesh draw reaches a frame of `PROBE_SIZE` pixels.
pub fn meshes_render(gpu: &Gpu) -> bool {
    let empty = avatar_view::Viewer::new(&gpu.device, &gpu.queue, FORMAT, true);
    let Ok(background) = empty.render_to_rgba(&gpu.device, &gpu.queue, PROBE_SIZE, PROBE_SIZE) else {
        return false;
    };
    let Ok(viewer) = gpu.build_viewer(&probe_scene(), std::path::Path::new("")) else {
        return false;
    };
    let Ok(frame) = viewer.render_to_rgba(&gpu.device, &gpu.queue, PROBE_SIZE, PROBE_SIZE) else {
        return false;
    };
    let changed = frame
        .pixels()
        .zip(background.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(x, y)| x.abs_diff(*y) > 6))
        .count();
    changed > (PROBE_SIZE * PROBE_SIZE / 50) as usize
}

fn device_on(backends: wgpu::Backends) -> Option<(Gpu, wgpu::AdapterInfo)> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends;
    let instance = wgpu::Instance::new(desc);
    let mut adapters = pollster::block_on(instance.enumerate_adapters(backends));
    adapters.sort_by_key(|a| rank(&a.get_info()));
    let adapter = adapters
        .into_iter()
        .find(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)?;
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((Gpu { device, queue }, info))
}

/// The backends the window should use and a note on why.
pub fn window_backends() -> (wgpu::Backends, String) {
    if let Some(forced) = wgpu::Backends::from_env() {
        return (forced, format!("WGPU_BACKEND selects {forced:?}"));
    }
    match device_on(wgpu::Backends::DX12) {
        Some((gpu, info)) if !meshes_render(&gpu) => (
            wgpu::Backends::VULKAN | wgpu::Backends::GL,
            format!("D3D12 on {} failed the mesh self-test; using Vulkan", info.name),
        ),
        Some((_, info)) => (
            wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            format!("D3D12 on {}", info.name),
        ),
        None => (
            wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            "no D3D12 adapter".into(),
        ),
    }
}

/// A headless device on the same backend choice as the window, for `--smoke`.
pub fn headless() -> anyhow::Result<(Gpu, String)> {
    let (backends, note) = window_backends();
    let ordered = [
        backends & wgpu::Backends::DX12,
        backends & !wgpu::Backends::DX12,
        wgpu::Backends::all(),
    ];
    for b in ordered.into_iter().filter(|b| !b.is_empty()) {
        if let Some((gpu, info)) = device_on(b) {
            return Ok((gpu, format!("{} ({:?}); {note}", info.name, info.backend)));
        }
    }
    anyhow::bail!("no wgpu adapter")
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_scene_loads() {
        let scene = super::probe_scene();
        avatar_export::avatar::Avatar::from_scene(&scene, std::path::PathBuf::new()).unwrap();
    }
}
