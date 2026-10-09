// Avatar Aura desktop app: imports avatar items into a closet and exports posed avatars as 3D models.

#![deny(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bake;
mod caption;
mod catalog;
mod export_tab;
mod gpu;
mod import;
mod import_tab;
mod jobs;
mod paths;
mod session;
mod settings;
mod smoke;
mod tiles;
mod viewport;
mod widgets;

use std::process::ExitCode;
use std::sync::Arc;

use eframe::{egui, egui_wgpu, wgpu};

fn icon() -> Option<egui::IconData> {
    let bytes = paths::bundled_bytes("avatar-aura.ico")?;
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)
        .ok()?
        .to_rgba8();
    Some(egui::IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    })
}

/// D3D12 first, then a discrete GPU, among the adapters that can present to the window.
fn pick_adapter(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<wgpu::Adapter, String> {
    let usable: Vec<&wgpu::Adapter> = adapters
        .iter()
        .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
        .collect();
    usable
        .into_iter()
        .min_by_key(|a| gpu::rank(&a.get_info()))
        .cloned()
        .ok_or_else(|| "no graphics adapter can draw to this window".to_string())
}

fn run_window(load_avatar: bool) -> eframe::Result {
    let settings = settings::Settings::load();
    let size = settings.window.unwrap_or([1280.0, 850.0]);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("avatar aura")
        .with_inner_size([size[0].max(940.0), size[1].max(680.0)])
        .with_min_inner_size([940.0, 680.0])
        .with_maximized(true)
        .with_drag_and_drop(true);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let (backends, note) = gpu::window_backends();
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends;
    setup.native_adapter_selector = Some(Arc::new(pick_adapter));
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "avatar aura",
        options,
        Box::new(move |cc| Ok(Box::new(app::AuraApp::new(cc, note, load_avatar)?))),
    )
}

fn main() -> ExitCode {
    if std::env::args().any(|a| a == "--smoke") {
        return match smoke::run() {
            Ok(()) => {
                println!("smoke: ok");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: {e:#}");
                ExitCode::FAILURE
            }
        };
    }
    match run_window(std::env::args().any(|a| a == "--load-avatar")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("avatar aura could not start: {e}");
            ExitCode::FAILURE
        }
    }
}
