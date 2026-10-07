// Ports of avatar_aura test_posing.py, test_faces.py and test_face_animation.py on the same two-bone fixture.

use std::path::{Path, PathBuf};

use avatar_export::avatar::{Avatar, Clip, Joint, Mesh, Track};
use avatar_export::face_animation::prepare;
use avatar_export::faces::{texture_files, Expression, Selection};
use avatar_export::math::{mat_inverse, mat_mul, mat_trans, mat_vec, FRAME_XBOX};
use avatar_export::posing::{
    bake_posed, from_local_pose, glb_bind_rig, glb_pose_rig, rebind_posed, sample, sample_named, PoseBone,
};
use avatar_export::rig::build_source_rig;
use avatar_export::scene::{Face, FaceFile, FaceTrack, Material};
use image::RgbaImage;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("avatar-export-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn joint(name: &str, parent: i32, at: [f64; 3]) -> Joint {
    Joint {
        name: name.into(),
        parent,
        bind_world: at,
        rest_world: at,
        rest_world_rot: [0.0, 0.0, 0.0, 1.0],
        rest_local: at,
        scale: [1.0; 3],
    }
}

fn fixture(dir: &Path) -> Avatar {
    let h = 0.5f64.sqrt();
    let track = |t: [[f64; 3]; 2], r: [[f64; 4]; 2]| Track {
        t: t.to_vec(),
        r: r.to_vec(),
        s: vec![[1.0; 3]; 2],
    };
    Avatar {
        dir: dir.to_path_buf(),
        info: Default::default(),
        skeleton: vec![joint("Root", -1, [0.0; 3]), joint("Child", 0, [1.0, 0.0, 0.0])],
        components: vec![],
        materials: vec![Material {
            name: "Head".into(),
            diffuse: "base.png".into(),
            ..Default::default()
        }],
        meshes: vec![Mesh {
            name: "Head".into(),
            component: 0,
            material: 0,
            is_prop: false,
            positions: vec![[2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![[1.0, 0.0, 0.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            colors: None,
            joints: vec![[0, 1, 0, 0]; 3],
            weights: vec![[0.5, 0.5, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
        }],
        animations: vec![Clip {
            name: "Turn".into(),
            fps: 30.0,
            frame_count: 2,
            tracks: vec![
                track([[0.0; 3], [2.0, 0.0, 0.0]], [[0.0, 0.0, 0.0, 1.0]; 2]),
                track([[1.0, 0.0, 0.0]; 2], [[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, h, h]]),
            ],
            face: None,
        }],
        prop_skeleton: None,
        face: Face {
            head_materials: vec!["Head".into()],
            composite_files: vec![FaceFile {
                channel: "eyes".into(),
                frame: 1,
                file: "face/Head_eyes_01.png".into(),
            }],
            ..Default::default()
        },
    }
}

fn close(a: &[f64], b: &[f64]) {
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() < 1e-7, "{a:?} vs {b:?}");
    }
}

#[test]
fn skinning_uses_sampled_parent_chain_and_normalized_weights() {
    let avatar = fixture(Path::new("."));
    let rig = sample_named(&avatar, "Turn", 1).unwrap();
    assert_eq!(mat_trans(&rig.bones[0].world), [2.0, 0.0, 0.0]);
    assert_eq!(mat_trans(&rig.bones[1].world), [3.0, 0.0, 0.0]);
    let posed = bake_posed(&avatar, &rig, &Expression::Named("eyes:1".into())).unwrap();
    close(&posed.meshes[0].positions[0], &[3.5, 0.5, 0.0]);
    let h = 0.5f64.sqrt();
    close(&posed.meshes[0].normals[0], &[h, h, 0.0]);
    assert_eq!(posed.materials[0].diffuse, "face/Head_eyes_01.png");
    assert!(posed.animations.is_empty());
    assert_eq!(avatar.meshes[0].positions[0], [2.0, 0.0, 0.0]);
    let rebuilt = build_source_rig(&posed, FRAME_XBOX, 1.0).unwrap();
    for (a, b) in rebuilt.bones.iter().zip(&rig.bones) {
        for (ra, rb) in a.world.iter().zip(&b.world) {
            close(ra, rb);
        }
    }
}

#[test]
fn invalid_frames_and_expressions_are_rejected() {
    let avatar = fixture(Path::new("."));
    for frame in [-1, 2] {
        assert!(sample_named(&avatar, "Turn", frame).is_err());
    }
    let rest = sample_named(&avatar, "Turn", 0).unwrap();
    assert!(bake_posed(&avatar, &rest, &Expression::Named("missing:7".into())).is_err());
}

#[test]
fn rest_frame_preserves_geometry() {
    let avatar = fixture(Path::new("."));
    let posed = bake_posed(
        &avatar,
        &sample_named(&avatar, "Turn", 0).unwrap(),
        &Expression::neutral(),
    )
    .unwrap();
    assert_eq!(posed.meshes[0].positions, avatar.meshes[0].positions);
}

#[test]
fn standing_bind_preserves_a_different_visible_pose() {
    let mut avatar = fixture(Path::new("."));
    let mut standing = avatar.animations[0].clone();
    standing.name = "Animation Generic Stand 1".into();
    avatar.animations.push(standing.clone());
    let reference = sample(&avatar, &standing, 1).unwrap();
    let bind = glb_bind_rig(&avatar).unwrap();
    let worlds = |r: &avatar_export::Rig| r.bones.iter().map(|b| b.world).collect::<Vec<_>>();
    assert_eq!(worlds(&bind), worlds(&reference));
    let pose = sample_named(&avatar, "Turn", 0).unwrap();
    let visible = bake_posed(&avatar, &pose, &Expression::neutral()).unwrap();
    let rebound = rebind_posed(&visible, &pose, &bind).unwrap();
    let matrices: Vec<_> = pose
        .bones
        .iter()
        .zip(&bind.bones)
        .map(|(p, b)| mat_mul(&p.world, &mat_inverse(&b.world)))
        .collect();
    for (mesh, expected) in rebound.meshes.iter().zip(&visible.meshes) {
        for (i, point) in mesh.positions.iter().enumerate() {
            let mut out = [0.0; 3];
            for (j, w) in pose.vertex_influences(&mesh.joints[i], &mesh.weights[i]) {
                let p = mat_vec(&matrices[j], point, 1.0);
                for k in 0..3 {
                    out[k] += w * p[k];
                }
            }
            close(&out, &expected.positions[i]);
        }
    }
}

#[test]
fn glb_bind_pose_stays_fixed_when_visible_pose_changes() {
    let avatar = fixture(Path::new("."));
    let bind = build_source_rig(&avatar, FRAME_XBOX, 1.0).unwrap();
    let first = sample_named(&avatar, "Turn", 0).unwrap();
    let second = sample_named(&avatar, "Turn", 1).unwrap();
    let a = glb_pose_rig(&avatar, &first).unwrap();
    let b = glb_pose_rig(&avatar, &second).unwrap();
    let locals = |r: &avatar_export::Rig| r.bones.iter().map(|b| b.local).collect::<Vec<_>>();
    let worlds = |r: &avatar_export::Rig| r.bones.iter().map(|b| b.world).collect::<Vec<_>>();
    assert_ne!(locals(&a), locals(&b));
    assert_eq!(worlds(&a), worlds(&bind));
    assert_eq!(worlds(&b), worlds(&bind));
    assert_eq!(locals(&b), locals(&second));
}

#[test]
fn free_pose_matches_parent_chain_and_rejects_invalid_input() {
    let avatar = fixture(Path::new("."));
    let h = 0.5f64.sqrt();
    let mut transforms = vec![
        PoseBone {
            name: "Root".into(),
            position: [2.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        },
        PoseBone {
            name: "Child".into(),
            position: [1.0, 0.0, 0.0],
            rotation: [0.0, 0.0, h, h],
        },
    ];
    let rig = from_local_pose(&avatar, &transforms).unwrap();
    let posed = bake_posed(&avatar, &rig, &Expression::neutral()).unwrap();
    close(&posed.meshes[0].positions[0], &[3.5, 0.5, 0.0]);
    assert!(from_local_pose(&avatar, &transforms[..1]).is_err());
    let doubled = vec![transforms[0].clone(), transforms[0].clone()];
    assert!(from_local_pose(&avatar, &doubled).is_err());
    transforms[0].position[0] = f64::NAN;
    assert!(from_local_pose(&avatar, &transforms).is_err());
}

fn save(path: &Path, w: u32, h: u32, pixels: &[[u8; 4]]) {
    let bytes = pixels.iter().flatten().copied().collect();
    RgbaImage::from_raw(w, h, bytes).unwrap().save(path).unwrap();
}

fn pixels(path: &Path) -> Vec<u8> {
    image::open(path).unwrap().to_rgba8().into_raw()
}

#[test]
fn mouth_and_eyes_preserve_independent_pixels() {
    let root = temp_dir("faces");
    std::fs::create_dir_all(root.join("face")).unwrap();
    let mut avatar = fixture(&root);
    avatar.face.composite_files.clear();
    let neutral = [[100, 80, 60, 255]; 2];
    let changed = [20, 30, 40, 255];
    for channel in ["mouth", "eyes"] {
        for frame in [0u32, 1] {
            let name = format!("face/Head_{channel}_{frame:02}.png");
            let mut px = neutral;
            if frame == 1 {
                px[if channel == "mouth" { 0 } else { 1 }] = changed;
            }
            save(&root.join(&name), 2, 1, &px);
            avatar.face.composite_files.push(FaceFile {
                channel: channel.into(),
                frame,
                file: name,
            });
        }
    }
    save(&root.join("base.png"), 4, 2, &[[90, 70, 50, 255]; 8]);
    let sel = |pairs: &[(&str, Option<u32>)]| {
        Expression::Mix(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<Selection>(),
        )
    };
    let files = texture_files(&avatar, &sel(&[("mouth", None), ("eyes", None)])).unwrap();
    assert_eq!(files.get(&0).map(String::as_str), Some("base.png"));
    let cases = [
        (sel(&[("mouth", Some(1)), ("eyes", None)]), [changed, neutral[1]]),
        (sel(&[("mouth", None), ("eyes", Some(1))]), [neutral[0], changed]),
        (sel(&[("mouth", Some(1)), ("eyes", Some(1))]), [changed, changed]),
    ];
    for (selection, expected) in cases {
        let file = texture_files(&avatar, &selection).unwrap()[&0].clone();
        assert!(file.starts_with("face/mix_v2_0_"));
        assert_eq!(pixels(&root.join(file)), expected.concat());
    }
    assert!(texture_files(&avatar, &sel(&[("mouth", Some(9))])).is_err());
    assert!(texture_files(&avatar, &sel(&[("nose", Some(1))])).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn mix_file_names_match_python_digest() {
    // hashlib.sha256(json.dumps({"eyes": 1, "mouth": None}, sort_keys=True).encode()).hexdigest()
    assert_eq!(
        avatar_export::faces::sha256_hex(br#"{"eyes": 1, "mouth": null}"#),
        "1bec39be0796b25b4e6975a0386982ca70e86150d6f8f40ca2644aafc7992794"
    );
}

#[test]
fn texture_tracks_keep_timing_and_independent_eyes() {
    let root = temp_dir("face-anim");
    let mut avatar = fixture(&root);
    avatar.face.composite_files.clear();
    for channel in ["mouth", "eyes", "brows"] {
        for frame in [0u32, 1] {
            let file = format!("face/Head_{channel}_{frame:02}.png");
            std::fs::create_dir_all(root.join("face")).unwrap();
            save(&root.join(&file), 1, 1, &[[frame as u8, 0, 0, 255]]);
            avatar.face.composite_files.push(FaceFile {
                channel: channel.into(),
                frame,
                file,
            });
        }
    }
    avatar.animations[0] = Clip {
        name: "Wink".into(),
        fps: 20.0,
        frame_count: 4,
        tracks: vec![],
        face: Some(FaceTrack {
            mouth: vec![0, 1, 1, 0],
            eye_left: vec![0, 1, 1, 99],
            eye_right: vec![0, 0, 0, 0],
            ..Default::default()
        }),
    };
    let result = prepare(&avatar, 0).unwrap();
    assert_eq!(result.fps, 20.0);
    assert_eq!(result.sequence, vec![0, 1, 1, 0]);
    assert_eq!(result.frames, 4);
    let Expression::Mix(id) = &result.entries[1].id else {
        panic!("mixed entry expected");
    };
    assert_eq!(id["eye_left"], Some(1));
    assert_eq!(id["eye_right"], Some(0));
    assert!(prepare(&avatar, 5).is_err());
    let _ = std::fs::remove_dir_all(root);
}
