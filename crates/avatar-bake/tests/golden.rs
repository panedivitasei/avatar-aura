//! Golden checks for the RunFromArgs `--avatar` export against avatarextract.exe output in the fixtures tree.
//! Fixtures come from AVATAR_AURA_FIXTURES (default ../avatar-aura-rust-fixtures); tests skip when absent.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

fn fixtures() -> Option<PathBuf> {
    let dir = std::env::var_os("AVATAR_AURA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../avatar-aura-rust-fixtures"));
    dir.join("golden/avatar/avatar.json").exists().then_some(dir)
}

/// The closet the golden was baked with: AVATAR_AURA_CLOSET, else ae-sub/assets/closet beside the fixtures.
fn closet(fixtures: &Path) -> Option<PathBuf> {
    let dir = std::env::var_os("AVATAR_AURA_CLOSET")
        .map(PathBuf::from)
        .unwrap_or_else(|| fixtures.join("../ae-sub/assets/closet"));
    dir.join("closet_index.tsv").exists().then_some(dir)
}

struct Run {
    golden: PathBuf,
    out: PathBuf,
}

/// One bake shared by every test, with the arguments recorded in golden/avatar/stdout.txt.
fn run() -> Option<&'static Run> {
    static RUN: OnceLock<Option<Run>> = OnceLock::new();
    RUN.get_or_init(|| {
        let Some(fx) = fixtures() else {
            eprintln!("skipping: fixtures not found");
            return None;
        };
        let Some(closet) = closet(&fx) else {
            eprintln!("skipping: closet not found");
            return None;
        };
        let out = std::env::temp_dir().join("avatar-bake-golden");
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let args: Vec<String> = vec![
            "--avatar".into(),
            s(&fx.join("inputs/avatar_manifest.bin")),
            s(&out),
            "--toc".into(),
            s(&fx.join("inputs/AvatarAssetPack.toc")),
            "--closet".into(),
            s(&closet),
            "--anim-dir".into(),
            s(&fx.join("inputs/clips")),
            "--pack-anims".into(),
            "--face-frames".into(),
            "512".into(),
            "--preview-dir".into(),
            s(&out),
            "--preview-frame".into(),
            "0.35".into(),
            "--preview-size".into(),
            "192".into(),
        ];
        let code = avatar_bake::cli::run(&args).expect("bake");
        assert_eq!(code, 0, "bake exit code");
        Some(Run {
            golden: fx.join("golden/avatar"),
            out,
        })
    })
    .as_ref()
}

fn compare_json(a: &Value, b: &Value, path: &str, errors: &mut Vec<String>) {
    if errors.len() > 20 || path == "/source" {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().filter(|k| !y.contains_key(*k)) {
                errors.push(format!("{path}/{k}: missing in output"));
            }
            for k in y.keys().filter(|k| !x.contains_key(*k)) {
                errors.push(format!("{path}/{k}: not in golden"));
            }
            for (k, v) in x {
                if let Some(w) = y.get(k) {
                    compare_json(v, w, &format!("{path}/{k}"), errors);
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                errors.push(format!("{path}: length {} vs {}", x.len(), y.len()));
                return;
            }
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                compare_json(v, w, &format!("{path}/{i}"), errors);
            }
        }
        (Value::Number(x), Value::Number(y)) => {
            let ints = (x.is_i64() || x.is_u64()) && (y.is_i64() || y.is_u64());
            if ints {
                if x != y {
                    errors.push(format!("{path}: {x} vs {y}"));
                }
            } else {
                let (u, v) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
                if (u - v).abs().is_nan() || (u - v).abs() > 1e-4 {
                    errors.push(format!("{path}: {u} vs {v}"));
                }
            }
        }
        _ => {
            if a != b {
                errors.push(format!("{path}: {a} vs {b}"));
            }
        }
    }
}

#[test]
fn avatar_json_matches_golden() {
    let Some(r) = run() else {
        return;
    };
    let read = |p: &Path| -> Value { serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap() };
    let golden = read(&r.golden.join("avatar.json"));
    let out = read(&r.out.join("avatar.json"));
    let mut errors = Vec::new();
    compare_json(&golden, &out, "", &mut errors);
    assert!(errors.is_empty(), "avatar.json differs:\n{}", errors.join("\n"));
    // The typed schema reads the output too.
    let text = std::fs::read_to_string(r.out.join("avatar.json")).unwrap();
    avatar_export::scene::Scene::from_json(&text).expect("scene parses");
}

fn pngs(dir: &Path, filter: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "png") && filter(&p.file_name().unwrap().to_string_lossy())
        })
        .collect();
    v.sort();
    v
}

/// Differing pixel count and the count differing by more than 8 in some channel.
fn diff_png(golden: &Path, out: &Path) -> Result<(usize, usize, usize), String> {
    let g = image::open(golden)
        .map_err(|e| format!("{}: {e}", golden.display()))?
        .to_rgba8();
    let o = image::open(out)
        .map_err(|e| format!("{}: {e}", out.display()))?
        .to_rgba8();
    if g.dimensions() != o.dimensions() {
        return Err(format!(
            "{}: size {:?} vs {:?}",
            golden.display(),
            g.dimensions(),
            o.dimensions()
        ));
    }
    let (mut any, mut big) = (0, 0);
    for (a, b) in g.pixels().zip(o.pixels()) {
        let d =
            a.0.iter()
                .zip(b.0.iter())
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
        any += usize::from(d > 0);
        big += usize::from(d > 8);
    }
    Ok((any, big, g.pixels().len()))
}

fn assert_exact(dir_rel: &str, filter: impl Fn(&str) -> bool, expect_min: usize) {
    let Some(r) = run() else {
        return;
    };
    let files = pngs(&r.golden.join(dir_rel), filter);
    assert!(files.len() >= expect_min, "only {} golden PNGs", files.len());
    let mut errors = Vec::new();
    for g in &files {
        let o = r.out.join(dir_rel).join(g.file_name().unwrap());
        match diff_png(g, &o) {
            Ok((0, _, _)) => {}
            Ok((any, big, n)) => {
                errors.push(format!("{}: {any}/{n} pixels differ ({big} by >8)", g.display()))
            }
            Err(e) => errors.push(e),
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn material_textures_match_golden() {
    assert_exact(".", |n| n.ends_with("_Diffuse.png"), 8);
}

#[test]
fn face_layers_and_composites_match_golden() {
    assert_exact("face", |_| true, 99);
}

/// Previews come from the software rasterizer: exact is expected, 0.5% of pixels off by more than 8 is tolerated.
#[test]
fn previews_match_golden() {
    let Some(r) = run() else {
        return;
    };
    let files = pngs(&r.golden, |n| n.starts_with("preview_"));
    assert_eq!(files.len(), 45);
    let mut errors = Vec::new();
    let mut inexact = Vec::new();
    for g in &files {
        let o = r.out.join(g.file_name().unwrap());
        match diff_png(g, &o) {
            Ok((0, _, _)) => {}
            Ok((any, big, n)) => {
                inexact.push(format!("{}: {any}/{n} differ ({big} by >8)", g.display()));
                if big * 200 > n {
                    errors.push(format!("{}: {big}/{n} pixels off by >8", g.display()));
                }
            }
            Err(e) => errors.push(e),
        }
    }
    if !inexact.is_empty() {
        eprintln!("previews within tolerance but not exact:\n{}", inexact.join("\n"));
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
