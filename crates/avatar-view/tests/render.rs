// Headless render of the golden avatar: rest pose and Cheer frame 10.

use std::path::PathBuf;

use avatar_export::scene::Scene;
use avatar_view::Viewer;
use image::RgbaImage;

const SIZE: u32 = 384;

fn fixture() -> Option<PathBuf> {
    let root = std::env::var_os("AVATAR_AURA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\edward\Documents\ReXGlue\avatar-aura-rust-fixtures"));
    let path = root.join("golden").join("avatar").join("avatar.json");
    path.is_file().then_some(path)
}

fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    // D3D12 on Windows; WGPU_BACKEND, WGPU_DX12_COMPILER and WGPU_DEBUG/WGPU_VALIDATION override.
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        desc.backends = wgpu::Backends::DX12;
    }
    let instance = wgpu::Instance::new(desc.with_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok()?;
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((device, queue, format!("{} ({:?})", info.name, info.backend)))
}

/// Pixels differing from the background render by more than a small tolerance, and their bounding box.
fn coverage(image: &RgbaImage, background: &RgbaImage) -> (f64, Option<[u32; 4]>) {
    let mut count = 0usize;
    let mut bbox: Option<[u32; 4]> = None;
    for (x, y, px) in image.enumerate_pixels() {
        let bg = background.get_pixel(x, y);
        let diff = (0..3)
            .map(|c| (i32::from(px[c]) - i32::from(bg[c])).abs())
            .max()
            .unwrap_or(0);
        if diff > 6 {
            count += 1;
            bbox = Some(match bbox {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            });
        }
    }
    (count as f64 / f64::from(image.width() * image.height()), bbox)
}

fn check_framed(name: &str, image: &RgbaImage, background: &RgbaImage) {
    let (fraction, bbox) = coverage(image, background);
    let [x0, y0, x1, y1] = bbox.expect("avatar pixels");
    let cx = f64::from(x0 + x1) / 2.0 / f64::from(SIZE);
    let cy = f64::from(y0 + y1) / 2.0 / f64::from(SIZE);
    println!("{name}: coverage {fraction:.3}, bbox {x0},{y0}-{x1},{y1}, centre {cx:.3},{cy:.3}");
    assert!(fraction > 0.15, "{name}: only {fraction:.3} of pixels differ");
    assert!((cx - 0.5).abs() < 0.2, "{name}: centre x {cx:.3}");
    assert!((cy - 0.5).abs() < 0.2, "{name}: centre y {cy:.3}");
}

#[test]
fn renders_rest_and_cheer() {
    let Some(path) = fixture() else {
        println!("skipped: golden avatar fixture not found");
        return;
    };
    let Some((device, queue, adapter)) = device() else {
        println!("skipped: no wgpu adapter");
        return;
    };
    println!("adapter: {adapter}");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let empty = Viewer::new(&device, &queue, format, true);
    let background = empty.render_to_rgba(&device, &queue, SIZE, SIZE).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let scene = Scene::from_json(&text).unwrap();
    let mut viewer = Viewer::new(&device, &queue, format, true);
    viewer.load_scene(&scene, path.parent().unwrap()).unwrap();

    viewer.set_rest_pose();
    let rest = viewer.render_to_rgba(&device, &queue, SIZE, SIZE).unwrap();
    let cheer = viewer.clip_index("Cheer").expect("Cheer clip");
    viewer.set_clip_frame(cheer, 10.0).unwrap();
    let posed = viewer.render_to_rgba(&device, &queue, SIZE, SIZE).unwrap();

    let dir = std::env::temp_dir();
    rest.save(dir.join("avatar_view_rest.png")).unwrap();
    posed.save(dir.join("avatar_view_cheer10.png")).unwrap();
    println!("wrote {}", dir.join("avatar_view_rest.png").display());

    check_framed("rest", &rest, &background);
    check_framed("cheer", &posed, &background);
    let changed = rest.pixels().zip(posed.pixels()).filter(|(a, b)| a != b).count();
    println!("rest vs cheer: {changed} pixels differ");
    assert!(changed > 500, "rest and Cheer frame 10 renders match");

    let bones = viewer.bone_screen_positions();
    assert_eq!(bones.len(), viewer.joint_names().len());
}
