// Port of main() of avatarextract/main.cpp: flag parsing and mode dispatch.
//! avatarextract: Xbox 360 avatar packages, asset packs and closet items to OBJ/MTL/PNG.
//!
//! Usage:
//!   avatarextract <input> [<out_dir>]            auto-detect input type
//!   avatarextract --pack <toc> [<out_dir>] [--index N]
//!   avatarextract <asset_v2.bin> [<out_dir>]     inspect a raw STRB/YTGR blob
//!   avatarextract --closet-import <raw_root> <closet_dir> [--icon-cat=<hex>]
//!   avatarextract --closet-icons <raw_root> <closet_dir> [--icon-cat=<hex>]
//!       (raw_root: an avataritems_raw or Avatar_Items marketplace archive;
//!        --icon-cat limits icon import to a category mask, e.g. 1000 = props)
//!   avatarextract --scan-closet <closet_dir>
//!   avatarextract --gen-icons <closet_dir> [--toc <pack.toc>] [--filter s] [--limit N] [--force]
//!   avatarextract --fix-outfits <toc> <outfits_dir>
//!   avatarextract --refit-female <item.bin> <out.bin> --toc <pack.toc>
//!   avatarextract --avatar <avatar_manifest.bin> <out_dir> [...]   saved-avatar export
//!   avatarextract --avatar-info <avatar_manifest.bin>              summary JSON
//!   avatarextract --prop-icons ...                                  closet carryable icons

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::{Path, PathBuf};

use avatarextract::{blob, closet_import, outfits, pack, package, refit};

/// The three avatar-TOC version guids (kAvatarTOCv1/v2/v3 of asset_pack.cpp).
const TOC_GUIDS: [[u8; 16]; 3] = [
    [
        0x9A, 0xD6, 0xEB, 0xCE, 0x62, 0x62, 0x4E, 0xEB, 0x8A, 0x82, 0xA3, 0xF7, 0x0B, 0x81, 0x73, 0x69,
    ],
    [
        0x8A, 0x76, 0x2D, 0xF4, 0xC0, 0x67, 0x4B, 0x66, 0xB4, 0xEE, 0x56, 0x46, 0x6B, 0x1B, 0x82, 0x80,
    ],
    [
        0x58, 0x0A, 0x07, 0xD6, 0x4B, 0xCD, 0x40, 0xBE, 0xBF, 0x37, 0x25, 0x4B, 0xE8, 0x26, 0xFB, 0x0B,
    ],
];

/// First `n` bytes of a file (fewer when it is short), `None` when it cannot be opened.
fn read_head(path: &Path, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(n);
    f.take(n as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// `atoi`: leading whitespace, optional sign, digits; anything else stops the parse.
fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        v = (v * 10 + i64::from(c - b'0')).min(i64::from(u32::MAX) + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `strtoul(s, nullptr, 16)` truncated to 32 bits: optional 0x prefix, hex digits until the first non-digit.
fn strtoul_hex(s: &str) -> u32 {
    let t = s.trim_start();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let mut v: u64 = 0;
    for c in t.chars().map_while(|c| c.to_digit(16)) {
        v = v.saturating_mul(16).saturating_add(u64::from(c));
    }
    // strtoul saturates to ULONG_MAX (32-bit on Windows) on overflow.
    v.min(u64::from(u32::MAX)) as u32
}

fn run(args: &[String], log: &mut dyn Write) -> i32 {
    for (i, a) in args.iter().enumerate() {
        match a.as_str() {
            "--prop-icons" | "--gen-icons" | "--avatar" | "--avatar-info" => {
                let _ = log.flush();
                return match avatar_bake::cli::run(args) {
                    Ok(code) => code,
                    Err(e) => {
                        say(log, &format!("ERROR: {e:#}\n"));
                        1
                    }
                };
            }
            "--refit-female" => {
                let rest: Vec<String> = args
                    .iter()
                    .enumerate()
                    .filter(|&(k, _)| k != i)
                    .map(|(_, s)| s.clone())
                    .collect();
                return refit::run_refit_args(&rest, log);
            }
            _ => {}
        }
    }

    let mut force_pack = false;
    // --extract: dump every STFS file raw (generic container extraction, e.g. DLC packages).
    let mut extract_all = false;
    let mut closet_import = false;
    let mut closet_icons = false;
    let mut closet_scan = false;
    let mut fix_outfits = false;
    let mut icon_cat_mask = u32::MAX;
    // -1 = auto-pick the base body.
    let mut want_index = -1i32;
    let mut positionals: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--pack" => force_pack = true,
            "--extract" => extract_all = true,
            "--closet-import" => closet_import = true,
            "--closet-icons" => closet_icons = true,
            "--scan-closet" => closet_scan = true,
            "--fix-outfits" => fix_outfits = true,
            "--index" if i + 1 < args.len() => {
                i += 1;
                want_index = atoi(&args[i]);
            }
            _ => {
                if let Some(hex) = arg.strip_prefix("--icon-cat=") {
                    icon_cat_mask = strtoul_hex(hex);
                } else if let Some(n) = arg.strip_prefix("--index=") {
                    want_index = atoi(n);
                } else {
                    positionals.push(arg);
                }
            }
        }
        i += 1;
    }
    let Some(&first) = positionals.first() else {
        say(
            log,
            "usage: avatarextract <input> [<out_dir>] [options]\nsee the mode list at the top of main().\n",
        );
        return 1;
    };
    let in_path = PathBuf::from(first);
    let out_dir = match positionals.get(1) {
        Some(o) => PathBuf::from(o),
        None => in_path.parent().unwrap_or(Path::new("")).join("extracted"),
    };
    say(
        log,
        &format!(
            "avatarextract\n  input : {}\n  output: {}\n\n",
            in_path.display(),
            out_dir.display()
        ),
    );

    // STFS starts with a known 4CC, asset packs with an avatar-TOC version guid; .toc or --pack force pack mode.
    let mut is_pack = force_pack || in_path.extension().is_some_and(|e| e.eq_ignore_ascii_case("toc"));
    if !is_pack {
        if let Some(magic) = read_head(&in_path, 16) {
            let is_stfs = stfs::PackageKind::from_magic(&magic).is_some();
            if !is_stfs && magic.len() == 16 && TOC_GUIDS.iter().any(|g| magic[..] == g[..]) {
                is_pack = true;
            }
        }
    }

    if closet_scan {
        say(log, "(mode: closet scan)\n\n");
        return closet_import::run_closet_scan(&in_path, log);
    }
    if closet_import {
        say(log, "(mode: closet import)\n\n");
        return closet_import::run_closet_import(&in_path, &out_dir, icon_cat_mask, log);
    }
    if closet_icons {
        say(log, "(mode: closet icon backfill)\n\n");
        return closet_import::run_closet_icons(&in_path, &out_dir, icon_cat_mask, log);
    }
    if fix_outfits {
        say(log, "(mode: fix saved outfits)\n\n");
        return outfits::run_fix_outfits(&in_path, &out_dir, log);
    }
    if extract_all {
        say(log, "(mode: generic STFS extraction)\n\n");
        return package::run_stfs_extract_all(&in_path, &out_dir, log);
    }
    if is_pack {
        say(log, "(mode: avatar asset pack)\n\n");
        return pack::run_pack(&in_path, &out_dir, want_index, log);
    }
    if let Some(magic) = read_head(&in_path, 4) {
        if magic == b"YTGR" || magic == b"STRB" {
            say(log, "(mode: raw STRB/YTGR blob)\n\n");
            return blob::run_raw_blob(&in_path, &out_dir, log);
        }
    }
    say(log, "(mode: marketplace STFS package)\n\n");
    package::run_stfs(&in_path, &out_dir, log)
}

fn say(log: &mut dyn Write, s: &str) {
    let _ = log.write_all(s.as_bytes());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = std::io::stdout();
    let code = run(&args, &mut out);
    let _ = out.flush();
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_number_parsing() {
        assert_eq!(atoi("12"), 12);
        assert_eq!(atoi(" -3x"), -3);
        assert_eq!(atoi("abc"), 0);
        assert_eq!(strtoul_hex("1000"), 0x1000);
        assert_eq!(strtoul_hex("0xFFzz"), 0xFF);
        assert_eq!(strtoul_hex("zz"), 0);
        assert_eq!(strtoul_hex("123456789"), u32::MAX);
    }

    #[test]
    fn no_arguments_prints_usage() {
        let mut log = Vec::new();
        assert_eq!(run(&[], &mut log), 1);
        assert!(String::from_utf8(log)
            .unwrap()
            .starts_with("usage: avatarextract"));
    }
}
