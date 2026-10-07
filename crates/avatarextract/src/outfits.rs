// Port of mode 7 (--fix-outfits, RunFixOutfits) of avatarextract/main.cpp.
//! Normalizes the asset ids inside saved X_AVATAR_METADATA outfit files (the editor's OUTFIT00_* saves).
//! "My Outfits" shows an outfit only when every id matches the enumerated catalog exactly, so stock-tail ids
//! get .c rewritten to the pack asset's bodies mask and companion entries are remapped to their catalog partner.

use std::io::Write;
use std::path::{Path, PathBuf};

use avatar_formats::AssetPack;

/// Tail shared by every stock pack id.
pub const STOCK_TAIL: [u8; 8] = [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0];

/// components[13] at 0x160 (stride 32) and fallbacks[4] at 0x300; the display gate walks only these.
pub const ID_OFFSETS: [usize; 17] = [
    0x160, 0x180, 0x1A0, 0x1C0, 0x1E0, 0x200, 0x220, 0x240, 0x260, 0x280, 0x2A0, 0x2C0, 0x2E0, 0x300, 0x320,
    0x340, 0x360,
];

/// One rewritten id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdPatch {
    pub offset: usize,
    pub from: (u32, u32, u32),
    pub to: (u32, u32, u32),
}

/// Companion index -> catalog index, from each catalog entry's `asset_ids[0].b`.
pub fn companion_map(pack: &AssetPack) -> Vec<i32> {
    let infos = pack.asset_infos();
    let mut map = vec![-1i32; infos.len()];
    for (j, info) in infos.iter().enumerate() {
        let id0 = &info.asset_ids[0];
        let partner = usize::from(id0.b);
        if !id0.is_zero() && partner != j && partner < infos.len() {
            map[partner] = j as i32;
        }
    }
    map
}

/// Rewrites the stock ids of one outfit buffer in place and returns what changed.
pub fn patch_outfit(bytes: &mut [u8], pack: &AssetPack, companions: &[i32]) -> Vec<IdPatch> {
    let infos = pack.asset_infos();
    let mut patches = Vec::new();
    for off in ID_OFFSETS {
        let Some(id) = bytes.get_mut(off..off + 16) else {
            continue;
        };
        if id[8..16] != STOCK_TAIL {
            continue;
        }
        let a = u32::from_be_bytes([id[0], id[1], id[2], id[3]]);
        let b = u32::from(u16::from_be_bytes([id[4], id[5]]));
        let c = u32::from(u16::from_be_bytes([id[6], id[7]]));
        if a == 0 && b == 0 && c == 0 {
            continue;
        }
        if b as usize >= infos.len() {
            continue;
        }
        let (mut na, mut nb) = (a, b);
        if let Ok(partner) = u32::try_from(companions[b as usize]) {
            nb = partner;
            na = infos[nb as usize].categories;
        }
        let bodies = u32::from(infos[nb as usize].bodies);
        let nc = if bodies != 0 { bodies } else { 3 };
        if (na, nb, nc) != (a, b, c) {
            id[0..4].copy_from_slice(&na.to_be_bytes());
            id[4..6].copy_from_slice(&(nb as u16).to_be_bytes());
            id[6..8].copy_from_slice(&(nc as u16).to_be_bytes());
            patches.push(IdPatch {
                offset: off,
                from: (a, b, c),
                to: (na, nb, nc),
            });
        }
    }
    patches
}

/// Files in directory order, depth first, as recursive_directory_iterator visits them.
fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd {
        let Ok(entry) = entry else { return };
        let p = entry.path();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir {
            walk_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Mode 7: patches every OUTFIT00_* file (not .header) under `outfits_dir` in place.
pub fn run_fix_outfits(toc_path: &Path, outfits_dir: &Path, log: &mut dyn Write) -> i32 {
    let toc = match std::fs::read(toc_path) {
        Ok(b) => b,
        Err(_) => {
            say!(log, "ERROR: cannot open asset pack: {}\n", toc_path.display());
            return 1;
        }
    };
    if toc.is_empty() {
        say!(log, "ERROR: bad read on asset pack.\n");
        return 1;
    }
    let Ok(pack) = AssetPack::load(toc) else {
        say!(log, "ERROR: AssetPack::Load failed.\n");
        return 2;
    };
    let companions = companion_map(&pack);
    let (mut files, mut patched_files, mut patched_ids) = (0usize, 0usize, 0usize);
    let mut paths = Vec::new();
    walk_files(outfits_dir, &mut paths);
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.starts_with("OUTFIT00_") || path.extension().is_some_and(|e| e == "header") {
            continue;
        }
        let Ok(mut bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.len() < 1000 {
            continue;
        }
        files += 1;
        let patches = patch_outfit(&mut bytes, &pack, &companions);
        for p in &patches {
            say!(
                log,
                "  {} @{:#x}: {:08X}-{:04X}-{:04X} -> {:08X}-{:04X}-{:04X}\n",
                name,
                p.offset,
                p.from.0,
                p.from.1,
                p.from.2,
                p.to.0,
                p.to.1,
                p.to.2
            );
        }
        patched_ids += patches.len();
        if !patches.is_empty() {
            // Same size as the original, so this matches the C++ rewrite through the r+b handle.
            if std::fs::write(&path, &bytes).is_ok() {
                patched_files += 1;
            }
        }
    }
    say!(
        log,
        "fix-outfits: {files} files scanned, {patched_files} patched ({patched_ids} ids)\n"
    );
    0
}
