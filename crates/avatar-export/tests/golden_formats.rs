// Golden comparison against avatar_aura/ae_convert.py output: the Python exporter's files for golden/formats/Avatar.
// Text formats compare line by line, the GLB JSON chunk as values within 1e-5 and its binary chunk byte for byte.

use std::path::{Path, PathBuf};

use avatar_export::dae::{write_dae, DaeOptions};
use avatar_export::export::{copy_face_assets, SMD_SCALE};
use avatar_export::glb::{write_glb, GlbOptions};
use avatar_export::math::{safe_id, FRAME_SOURCE, FRAME_XBOX};
use avatar_export::obj::write_obj;
use avatar_export::rig::build_source_rig;
use avatar_export::smd::{write_smd_animation, write_smd_reference};
use avatar_export::Avatar;
use serde_json::Value;

fn golden() -> Option<PathBuf> {
    let root = std::env::var("AVATAR_AURA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\edward\Documents\ReXGlue\avatar-aura-rust-fixtures"));
    let dir = root.join("golden").join("formats").join("Avatar");
    dir.join("source").join("avatar.json").is_file().then_some(dir)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("avatar-export-golden-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn load(golden: &Path) -> Avatar {
    Avatar::load(&golden.join("source").join("avatar.json")).unwrap()
}

/// Line-equal after trimming trailing whitespace; `skip` drops lines that carry timestamps.
fn assert_text_eq(ours: &Path, theirs: &Path, skip: &[&str]) {
    let a = std::fs::read_to_string(ours).unwrap();
    let b = std::fs::read_to_string(theirs).unwrap();
    let keep = |l: &&str| !skip.iter().any(|s| l.trim_start().starts_with(s));
    let la: Vec<&str> = a.lines().filter(keep).map(str::trim_end).collect();
    let lb: Vec<&str> = b.lines().filter(keep).map(str::trim_end).collect();
    let mut diffs = 0;
    let mut first = None;
    for (i, (x, y)) in la.iter().zip(&lb).enumerate() {
        if x != y {
            diffs += 1;
            if first.is_none() {
                let (x, y) = first_token_diff(x, y);
                first = Some(format!("line {}: ours `{x}` theirs `{y}`", i + 1));
            }
        }
    }
    assert!(
        diffs == 0 && la.len() == lb.len(),
        "{}: {diffs} differing lines, {} vs {} lines; first {:?}",
        ours.display(),
        la.len(),
        lb.len(),
        first
    );
}

fn first_token_diff<'a>(x: &'a str, y: &'a str) -> (&'a str, &'a str) {
    for (a, b) in x.split(' ').zip(y.split(' ')) {
        if a != b {
            return (a, b);
        }
    }
    (x, y)
}

fn json_close(a: &Value, b: &Value, path: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() <= 1e-5 {
                Ok(())
            } else {
                Err(format!("{path}: {x} vs {y}"))
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!("{path}: length {} vs {}", x.len(), y.len()));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                json_close(p, q, &format!("{path}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Object(x), Value::Object(y)) => {
            let kx: Vec<_> = x.keys().collect();
            let ky: Vec<_> = y.keys().collect();
            let mut sx = kx.clone();
            let mut sy = ky.clone();
            sx.sort();
            sy.sort();
            if sx != sy {
                return Err(format!("{path}: keys {sx:?} vs {sy:?}"));
            }
            for (k, v) in x {
                json_close(v, &y[k], &format!("{path}.{k}"))?;
            }
            Ok(())
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{path}: {a} vs {b}")),
    }
}

fn glb_chunks(path: &Path) -> (Value, Vec<u8>) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(&data[..4], b"glTF");
    let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as usize;
    let js_len = u32_at(12);
    let js: Value = serde_json::from_slice(&data[20..20 + js_len]).unwrap();
    let bin_off = 20 + js_len;
    let bin_len = u32_at(bin_off);
    assert_eq!(&data[bin_off + 4..bin_off + 8], b"BIN\0");
    (js, data[bin_off + 8..bin_off + 8 + bin_len].to_vec())
}

#[test]
fn obj_matches_python() {
    let Some(g) = golden() else {
        eprintln!("fixtures missing, skipped");
        return;
    };
    let av = load(&g);
    let rig = build_source_rig(&av, FRAME_XBOX, 1.0).unwrap();
    let out = temp_dir("obj");
    write_obj(&av, &rig, &out.join("Avatar.obj"), true, true).unwrap();
    assert_text_eq(&out.join("Avatar.obj"), &g.join("obj/Avatar.obj"), &[]);
    assert_text_eq(&out.join("Avatar.mtl"), &g.join("obj/Avatar.mtl"), &[]);
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn dae_matches_python() {
    let Some(g) = golden() else {
        eprintln!("fixtures missing, skipped");
        return;
    };
    let av = load(&g);
    let rig = build_source_rig(&av, FRAME_XBOX, 1.0).unwrap();
    let out = temp_dir("dae");
    let stamp = ["<created>", "<modified>"];
    write_dae(&av, &rig, &out.join("Avatar.dae"), &DaeOptions::default()).unwrap();
    assert_text_eq(&out.join("Avatar.dae"), &g.join("dae/Avatar.dae"), &stamp);
    let clips = g.join("dae/clips");
    if clips.is_dir() {
        for a in &av.animations {
            let file = format!("Avatar_{}.dae", safe_id(&a.name));
            if !clips.join(&file).is_file() {
                continue;
            }
            let opts = DaeOptions {
                animation: Some(a),
                ..DaeOptions::default()
            };
            write_dae(&av, &rig, &out.join(&file), &opts).unwrap();
            assert_text_eq(&out.join(&file), &clips.join(&file), &stamp);
        }
    }
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn smd_matches_python() {
    let Some(g) = golden() else {
        eprintln!("fixtures missing, skipped");
        return;
    };
    let reference = g.join("smd/Avatar_reference.smd");
    if !reference.is_file() {
        eprintln!("smd goldens missing, skipped");
        return;
    }
    let av = load(&g);
    let rig = build_source_rig(&av, FRAME_SOURCE, SMD_SCALE).unwrap();
    let out = temp_dir("smd");
    write_smd_reference(&av, &rig, &out.join("Avatar_reference.smd"), true).unwrap();
    assert_text_eq(&out.join("Avatar_reference.smd"), &reference, &[]);
    for a in &av.animations {
        let file = format!("{}.smd", safe_id(&a.name));
        let theirs = g.join("smd/anims").join(&file);
        if !theirs.is_file() {
            continue;
        }
        write_smd_animation(&av, &rig, a, &out.join(&file)).unwrap();
        assert_text_eq(&out.join(&file), &theirs, &[]);
    }
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn glb_matches_python() {
    let Some(g) = golden() else {
        eprintln!("fixtures missing, skipped");
        return;
    };
    let av = load(&g);
    let rig = build_source_rig(&av, FRAME_XBOX, 1.0).unwrap();
    let out = temp_dir("glb");
    let ours = out.join("Avatar.glb");
    write_glb(&av, &rig, &ours, &GlbOptions::default()).unwrap();
    let (ja, ba) = glb_chunks(&ours);
    let (jb, bb) = glb_chunks(&g.join("glb/Avatar.glb"));
    if let Err(e) = json_close(&ja, &jb, "$") {
        panic!("glb json differs at {e}");
    }
    let differing = ba.iter().zip(&bb).filter(|(x, y)| x != y).count();
    let first = ba.iter().zip(&bb).position(|(x, y)| x != y);
    assert!(
        ba.len() == bb.len() && differing == 0,
        "bin chunk: {} vs {} bytes, {differing} differ, first at {first:?}",
        ba.len(),
        bb.len()
    );
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn face_index_matches_python() {
    let Some(g) = golden() else {
        eprintln!("fixtures missing, skipped");
        return;
    };
    let theirs = g.join("face/face_index.json");
    if !theirs.is_file() {
        eprintln!("face goldens missing, skipped");
        return;
    }
    let av = load(&g);
    let out = temp_dir("face");
    let written = copy_face_assets(&av, &out).unwrap();
    let a: Value = serde_json::from_slice(&std::fs::read(out.join("face/face_index.json")).unwrap()).unwrap();
    let b: Value = serde_json::from_slice(&std::fs::read(&theirs).unwrap()).unwrap();
    if let Err(e) = json_close(&a, &b, "$") {
        panic!("face_index differs at {e}");
    }
    for p in &written {
        let rel = p.strip_prefix(&out).unwrap();
        let golden_file = g.join(rel);
        if golden_file.is_file() && rel.extension().is_some_and(|e| e == "png") {
            assert_eq!(std::fs::read(p).unwrap(), std::fs::read(&golden_file).unwrap());
        }
    }
    let _ = std::fs::remove_dir_all(out);
}
