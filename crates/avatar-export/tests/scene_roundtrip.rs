// The golden avatar.json parses into the scene types and survives a round trip.

use std::path::PathBuf;

use avatar_export::scene::Scene;

fn fixtures() -> Option<PathBuf> {
    let root = std::env::var("AVATAR_AURA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\edward\Documents\ReXGlue\avatar-aura-rust-fixtures"));
    root.is_dir().then_some(root)
}

#[test]
fn golden_avatar_json_round_trips() {
    let Some(root) = fixtures() else {
        eprintln!("fixtures missing, skipped");
        return;
    };
    let text = std::fs::read_to_string(root.join("golden/avatar/avatar.json")).unwrap();
    let scene = Scene::from_json(&text).unwrap();
    assert_eq!(scene.format, avatar_export::scene::FORMAT);
    assert_eq!(scene.skeleton.joints.len(), 71);
    assert_eq!(scene.meshes[0].vertex_count, 1137);
    assert_eq!(scene.meshes[0].positions.len(), 3411);
    assert_eq!(scene.animations.len(), 44);
    let again = Scene::from_json(&scene.to_json().unwrap()).unwrap();
    assert_eq!(scene, again);
}
