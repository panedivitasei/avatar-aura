// Headless free-pose gizmo checks on the golden avatar: ring colours, marker press and drag, root move.

use std::path::PathBuf;

use avatar_export::scene::Scene;
use avatar_view::{GizmoMode, Handle, Viewer};
use glam::{Vec2, Vec3};
use image::RgbaImage;

const SIZE: u32 = 384;

fn fixture() -> Option<PathBuf> {
    let root = std::env::var_os("AVATAR_AURA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\edward\Documents\ReXGlue\avatar-aura-rust-fixtures"));
    let path = root.join("golden").join("avatar").join("avatar.json");
    path.is_file().then_some(path)
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
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
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

struct Setup {
    device: wgpu::Device,
    queue: wgpu::Queue,
    viewer: Viewer,
}

fn setup() -> Option<Setup> {
    let Some(path) = fixture() else {
        println!("skipped: golden avatar fixture not found");
        return None;
    };
    let Some((device, queue)) = device() else {
        println!("skipped: no wgpu adapter");
        return None;
    };
    let scene = Scene::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut viewer = Viewer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, true);
    viewer.load_scene(&scene, path.parent().unwrap()).unwrap();
    viewer.set_viewport_size(SIZE, SIZE);
    Some(Setup {
        device,
        queue,
        viewer,
    })
}

fn joint(viewer: &Viewer, name: &str) -> usize {
    viewer.joint_names().iter().position(|n| *n == name).unwrap()
}

/// Pixels within `radius` of `at` close to a pure primary, per channel.
fn primaries(image: &RgbaImage, at: Vec2, radius: f32) -> [usize; 3] {
    let mut counts = [0; 3];
    for (x, y, px) in image.enumerate_pixels() {
        if Vec2::new(x as f32, y as f32).distance(at) > radius {
            continue;
        }
        for (c, count) in counts.iter_mut().enumerate() {
            let others = (0..3).filter(|&o| o != c).all(|o| px[o] < 90);
            if px[c] > 170 && others {
                *count += 1;
            }
        }
    }
    counts
}

#[test]
fn rotate_rings_render_around_selected_joint() {
    let Some(Setup {
        device,
        queue,
        mut viewer,
    }) = setup()
    else {
        return;
    };
    let head = joint(&viewer, "HEAD");
    let at = viewer.bone_screen_positions()[head].expect("head on screen");
    let plain = viewer.render_to_rgba(&device, &queue, SIZE, SIZE).unwrap();
    viewer.set_gizmo_enabled(true);
    assert_eq!(viewer.gizmo().selected(), Some(head));
    assert_eq!(viewer.gizmo().mode(), GizmoMode::Rotate);
    let posed = viewer.render_to_rgba(&device, &queue, SIZE, SIZE).unwrap();
    posed
        .save(std::env::temp_dir().join("avatar_view_gizmo.png"))
        .unwrap();
    let before = primaries(&plain, at, 60.0);
    let after = primaries(&posed, at, 60.0);
    println!("head at {at}, primaries without {before:?}, with {after:?}");
    for c in 0..3 {
        assert!(after[c] >= before[c] + 20, "channel {c}: {before:?} -> {after:?}");
    }
}

#[test]
fn marker_press_and_drag_rotates_joint() {
    let Some(Setup { mut viewer, .. }) = setup() else {
        return;
    };
    viewer.set_gizmo_enabled(true);
    let elbow = joint(&viewer, "LF_E");
    let wrist = joint(&viewer, "LF_W");
    let start = viewer.bone_screen_positions();
    let worlds = viewer.joint_worlds();
    let base = viewer.joint_pose(elbow).unwrap();
    let at = start[elbow].expect("elbow on screen");
    assert_eq!(viewer.pick_joint(at.x, at.y), Some(elbow));
    assert!(
        !viewer.gizmo_press(2.0, 2.0),
        "empty corner must leave the press to the camera"
    );
    assert!(viewer.gizmo_press(at.x, at.y));
    assert_eq!(viewer.gizmo().selected(), Some(elbow));
    for step in 1..=4 {
        viewer.gizmo_drag(at.x + 10.0 * step as f32, at.y);
    }
    viewer.gizmo_release();
    assert!(!viewer.gizmo().is_dragging());
    let edit = viewer
        .pose_edits()
        .into_iter()
        .find(|e| e.joint == elbow)
        .expect("elbow edit");
    let delta = edit.rotation().angle_between(base.rotation()).to_degrees();
    println!(
        "elbow {:?} -> {:?}, {delta:.2} deg",
        base.rotation_degrees, edit.rotation_degrees
    );
    assert!(delta > 1.0);
    let moved = viewer.bone_screen_positions();
    let shift = moved[wrist].unwrap().distance(start[wrist].unwrap());
    println!("wrist marker moved {shift:.2} px");
    assert!(shift > 1.0);
    assert!(moved[elbow].unwrap().distance(at) < 0.5);

    viewer.reset_pose().unwrap();
    assert!(viewer.pose_edits().is_empty());
    for (a, b) in viewer.joint_worlds().iter().zip(&worlds) {
        assert!(a.abs_diff_eq(*b, 1e-4), "reset pose differs");
    }

    let mut values = base;
    values.rotation_degrees.x += 30.0;
    viewer.set_pose_edit(elbow, values).unwrap();
    let shown = viewer.joint_pose(elbow).unwrap();
    assert!(shown.rotation().angle_between(values.rotation()) < 1e-3);
    viewer.reset_joint(elbow).unwrap();
    assert!(
        viewer
            .joint_pose(elbow)
            .unwrap()
            .rotation()
            .angle_between(base.rotation())
            < 1e-3
    );
}

#[test]
fn translate_arrow_moves_root() {
    let Some(Setup { mut viewer, .. }) = setup() else {
        return;
    };
    viewer.set_gizmo_enabled(true);
    let root = viewer.gizmo().root_joint().expect("BASE joint");
    assert!(
        !viewer.gizmo_mut().set_mode(GizmoMode::Translate),
        "Move is root-only"
    );
    assert!(viewer.gizmo_mut().select(root));
    assert!(viewer.gizmo_mut().set_mode(GizmoMode::Translate));
    let center = viewer.gizmo_handle_position(Handle::Free).unwrap();
    let (handle, at) = Handle::AXES
        .into_iter()
        .filter_map(|h| Some((h, viewer.gizmo_handle_position(h)?)))
        .max_by(|a, b| a.1.distance(center).total_cmp(&b.1.distance(center)))
        .unwrap();
    let before = viewer.joint_worlds()[0].w_axis.truncate();
    assert!(viewer.gizmo_hover(at.x, at.y));
    assert_eq!(viewer.gizmo().hovered(), Some(handle));
    assert!(viewer.gizmo_press(at.x, at.y));
    assert_eq!(viewer.gizmo().active(), Some(handle));
    let dir = (at - center).normalize();
    viewer.gizmo_drag(at.x + dir.x * 40.0, at.y + dir.y * 40.0);
    viewer.gizmo_release();
    let after = viewer.joint_worlds()[0].w_axis.truncate();
    println!("root {before} -> {after} along {handle:?}");
    assert!(after.distance(before) > 1e-3);
    let edit = viewer.pose_edits().into_iter().find(|e| e.joint == root).unwrap();
    assert!(edit.translation.distance(Vec3::ZERO) > 0.0);
}
