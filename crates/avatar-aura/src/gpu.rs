// Backend choice: D3D12 on Windows, the primary backends elsewhere; `WGPU_BACKEND` overrides for diagnosis.

use eframe::wgpu;


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

