// Port of mode 1 (marketplace STFS package -> model.obj, RunStfsMode) and --extract (RunStfsExtractAll)
// of avatarextract/main.cpp.
//! STFS package extraction.

use std::io::Write;
use std::path::Path;

use avatar_formats::model::load_option as model_option;
use avatar_formats::Model;

use crate::objwrite::write_model;

fn invalid(log: &mut dyn Write, in_path: &Path) -> i32 {
    say!(
        log,
        "ERROR: not a valid STFS package (or unreadable): {}\n",
        in_path.display()
    );
    1
}

/// --extract: dumps every file of the package into `out_dir` unmodified, flat (directories by name only).
pub fn run_stfs_extract_all(in_path: &Path, out_dir: &Path, log: &mut dyn Write) -> i32 {
    let Ok(data) = std::fs::read(in_path) else {
        return invalid(log, in_path);
    };
    let Ok(pkg) = stfs::Package::parse(&data) else {
        return invalid(log, in_path);
    };
    let _ = std::fs::create_dir_all(out_dir);
    let mut written = 0usize;
    for f in pkg.entries() {
        if f.is_directory {
            let _ = std::fs::create_dir_all(out_dir.join(&f.name));
            continue;
        }
        let Ok(bytes) = pkg.read(f) else {
            say!(log, "ERROR: failed to read '{}' from STFS block chain.\n", f.name);
            return 2;
        };
        let dst = out_dir.join(&f.name);
        if std::fs::write(&dst, &bytes).is_err() {
            say!(log, "ERROR: cannot write {}\n", dst.display());
            return 3;
        }
        say!(log, "  wrote {:<44} {} bytes\n", f.name, bytes.len());
        written += 1;
    }
    say!(log, "extracted {written} file(s) to {}\n", out_dir.display());
    0
}

/// Mode 1: picks the avatar asset blob (asset_v2.bin, else asset.bin, else the largest .bin), writes
/// `model.obj`/`model.mtl`/PNGs and copies icon.png when present.
pub fn run_stfs(in_path: &Path, out_dir: &Path, log: &mut dyn Write) -> i32 {
    let Ok(data) = std::fs::read(in_path) else {
        return invalid(log, in_path);
    };
    let Ok(pkg) = stfs::Package::parse(&data) else {
        return invalid(log, in_path);
    };
    let files = pkg.entries();
    say!(log, "=== STFS contents ({} entries) ===\n", files.len());
    for f in files {
        say!(
            log,
            "  {:<44} {} size={:<9} start_block={}\n",
            f.name,
            if f.is_directory { "[DIR] " } else { "[FILE]" },
            f.size,
            f.start_block
        );
    }
    say!(log, "\n");

    let mut asset = None;
    let mut icon = None;
    for f in files.iter().filter(|f| !f.is_directory) {
        let lower = f.name.to_ascii_lowercase();
        if lower == "asset_v2.bin" || (lower == "asset.bin" && asset.is_none()) {
            asset = Some(f);
        }
        if lower == "icon.png" {
            icon = Some(f);
        }
    }
    if asset.is_none() {
        for f in files.iter().filter(|f| !f.is_directory) {
            if f.name.to_ascii_lowercase().ends_with(".bin")
                && asset.is_none_or(|b: &stfs::Entry| f.size > b.size)
            {
                asset = Some(f);
            }
        }
    }
    let Some(asset) = asset else {
        say!(
            log,
            "ERROR: no avatar asset blob (.bin) found in package. Items that only\n\
             reference asset ids resolve against an AvatarAssetPack.toc; use --pack.\n"
        );
        return 2;
    };
    say!(
        log,
        "Using avatar asset blob: {} ({} bytes)\n",
        asset.name,
        asset.size
    );

    let Ok(strb) = pkg.read(asset) else {
        say!(log, "ERROR: failed to read asset blob from STFS block chain.\n");
        return 3;
    };
    if strb.len() < 4 || (&strb[..4] != b"STRB" && &strb[..4] != b"YTGR") {
        let b = |i: usize| strb.get(i).copied().unwrap_or(0);
        say!(
            log,
            "ERROR: asset blob is not an STRB/YTGR container (first4: {:02X} {:02X} {:02X} {:02X}).\n",
            b(0),
            b(1),
            b(2),
            b(3)
        );
        return 4;
    }

    let Ok(Some(model)) = Model::load(&strb, model_option::NONE) else {
        say!(
            log,
            "ERROR: Model::Load failed (no kModel STRB block or decompress failed).\n"
        );
        return 5;
    };
    let Some(t) = write_model(&model, out_dir, "model", log) else {
        return 6;
    };

    if let Some(icon) = icon {
        if let Ok(bytes) = pkg.read(icon) {
            let _ = std::fs::write(out_dir.join("icon.png"), bytes);
        }
    }

    say!(log, "\n=== Done ===\n");
    say!(
        log,
        "Wrote model with {} batches, {} verts, {} tris.\n",
        model.triangle_batches.len(),
        t.vertices,
        t.triangles
    );
    0
}
