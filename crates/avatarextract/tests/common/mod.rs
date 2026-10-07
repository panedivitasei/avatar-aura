// Shared helpers for the tests of every avatarextract mode (golden comparisons against the C++ main.cpp output).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixture root from `AVATAR_AURA_FIXTURES` or the default checkout; `None` when absent.
pub fn fixtures() -> Option<PathBuf> {
    let dir = std::env::var_os("AVATAR_AURA_FIXTURES").map_or_else(
        || PathBuf::from(r"C:\Users\edward\Documents\ReXGlue\avatar-aura-rust-fixtures"),
        PathBuf::from,
    );
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("fixtures not found at {}, skipping", dir.display());
        None
    }
}

/// Fresh scratch directory under the system temp dir.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("avatarextract-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Runs the built binary; returns exit code and stdout.
pub fn run_bin(args: &[&std::ffi::OsStr]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_avatarextract"))
        .args(args)
        .output()
        .expect("run avatarextract");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Line-by-line text comparison with trailing whitespace trimmed; panics on the first difference.
pub fn assert_text_eq(got: &str, want: &str, what: &str) {
    let g: Vec<&str> = got.lines().map(str::trim_end).collect();
    let w: Vec<&str> = want.lines().map(str::trim_end).collect();
    for (i, (a, b)) in g.iter().zip(&w).enumerate() {
        assert_eq!(a, b, "{what}: line {} differs", i + 1);
    }
    assert_eq!(g.len(), w.len(), "{what}: line count differs");
}

pub fn assert_files_text_eq(got: &Path, want: &Path) {
    let g = std::fs::read_to_string(got).unwrap_or_else(|e| panic!("{}: {e}", got.display()));
    let w = std::fs::read_to_string(want).unwrap_or_else(|e| panic!("{}: {e}", want.display()));
    assert_text_eq(&g, &w, &want.display().to_string());
}

/// Pixel-exact PNG comparison.
pub fn assert_png_eq(got: &Path, want: &Path) {
    let g = image::open(got)
        .unwrap_or_else(|e| panic!("{}: {e}", got.display()))
        .to_rgba8();
    let w = image::open(want)
        .unwrap_or_else(|e| panic!("{}: {e}", want.display()))
        .to_rgba8();
    assert_eq!(g.dimensions(), w.dimensions(), "{}", want.display());
    assert!(g.as_raw() == w.as_raw(), "{}: pixels differ", want.display());
}

/// Sorted file names in a directory, filtered by extension.
pub fn files_with_ext(dir: &Path, exts: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| exts.iter().any(|x| n.ends_with(x)))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Compares every obj/mtl/png in `golden` against `out`, and that `out` has no extra ones.
pub fn assert_model_dir_eq(out: &Path, golden: &Path) {
    let exts = [".obj", ".mtl", ".png"];
    assert_eq!(
        files_with_ext(out, &exts),
        files_with_ext(golden, &exts),
        "file set differs for {}",
        golden.display()
    );
    for name in files_with_ext(golden, &exts) {
        if name.ends_with(".png") {
            assert_png_eq(&out.join(&name), &golden.join(&name));
        } else {
            assert_files_text_eq(&out.join(&name), &golden.join(&name));
        }
    }
}

/// Value of the `  input : ` / `  output: ` header lines.
pub fn header_value<'a>(stdout: &'a str, key: &str) -> &'a str {
    stdout.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("")
}

/// Compares stdout against the golden after mapping this run's input/output paths onto the golden's.
pub fn assert_stdout_eq(got: &str, golden_stdout: &Path) {
    let want = std::fs::read_to_string(golden_stdout).expect("golden stdout");
    let mut got = got.to_string();
    for key in ["  output: ", "  input : "] {
        let (mine, theirs) = (header_value(&got, key).to_string(), header_value(&want, key));
        if !mine.is_empty() {
            got = got.replace(&mine, theirs);
        }
    }
    assert_text_eq(&got, &want, &golden_stdout.display().to_string());
}
