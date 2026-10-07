// Backend choice: D3D12 on Windows, the primary backends elsewhere; `WGPU_BACKEND` overrides for diagnosis.

use eframe::wgpu;

use crate::viewport::Gpu;

/// D3D12 first, then a discrete GPU.
pub fn rank(info: &wgpu::AdapterInfo) -> (bool, bool) {
    (
        info.backend != wgpu::Backend::Dx12,
        info.device_type != wgpu::DeviceType::DiscreteGpu,
    )
}

/// The backends the window should use and a note on why.
pub fn window_backends() -> (wgpu::Backends, String) {
    if let Some(forced) = wgpu::Backends::from_env() {
        return (forced, format!("WGPU_BACKEND selects {forced:?}"));
    }
    if cfg!(windows) {
        (wgpu::Backends::DX12, "D3D12".into())
    } else {
        (
            wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            "primary backends".into(),
        )
    }
}

/// A headless high-performance device on the window's backends, for `--smoke`.
pub fn headless() -> anyhow::Result<(Gpu, String)> {
    let (backends, note) = window_backends();
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backends;
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .map_err(|e| anyhow::anyhow!("no {note} adapter: {e}"))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    Ok((
        Gpu { device, queue },
        format!("{} ({:?}); {note}", info.name, info.backend),
    ))
}
