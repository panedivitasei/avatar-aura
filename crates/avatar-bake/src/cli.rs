//! Ports RunFromArgs plus the main.cpp dispatch of `--avatar`, `--avatar-info`, `--prop-icons` and `--gen-icons`.

use std::path::{Path, PathBuf};

use anyhow::bail;

use crate::args::{self, Parsed};

/// RunFromArgs: option parsing, the userdata defaults for the pack and closet, then the export.
pub fn run_from_args(argv: &[String]) -> anyhow::Result<i32> {
    let (mut a, positionals) = match args::parse(argv) {
        Parsed::Help => {
            args::print_usage();
            return Ok(0);
        }
        Parsed::Run(a, p) => (a, p),
    };
    let Some(manifest) = positionals.first() else {
        args::print_usage();
        return Ok(1);
    };
    a.manifest = PathBuf::from(manifest);
    let manifest_dir = a.manifest.parent().unwrap_or(Path::new("")).to_path_buf();
    a.out_dir = match positionals.get(1) {
        Some(out) => PathBuf::from(out),
        None => manifest_dir.join("export"),
    };
    // Defaults follow the shared userdata layout: ...\userdata\avatars\avatar_manifest.bin next to
    // ...\userdata\avatarpack\AvatarAssetPack.toc.
    if a.toc.as_os_str().is_empty() {
        let userdata = manifest_dir.parent().unwrap_or(Path::new(""));
        let cand = userdata.join("avatarpack").join("AvatarAssetPack.toc");
        if cand.exists() {
            a.toc = cand;
        }
    }
    if a.toc.as_os_str().is_empty() || !a.toc.exists() {
        println!("ERROR: asset pack not found; pass --toc <AvatarAssetPack.toc>");
        return Ok(1);
    }
    let pack_dir = a.toc.parent().unwrap_or(Path::new("")).to_path_buf();
    if a.legacy_toc.as_os_str().is_empty() {
        let cand = pack_dir.join("AvatarAssetPackLegacyV1.toc");
        if cand.exists() {
            a.legacy_toc = cand;
        }
    }
    if a.closet.as_os_str().is_empty() {
        let cand = pack_dir.join("closet");
        if cand.is_dir() {
            a.closet = cand;
        }
    }
    crate::bake::run_export(&a)
}

/// The bake modes of the avatarextract command line; `args` excludes the program name and the first mode flag
/// picks the mode. Errors when no mode flag is present, so the caller can try its other modes.
pub fn run(args: &[String]) -> anyhow::Result<i32> {
    for (i, a) in args.iter().enumerate() {
        let rest = || -> Vec<String> {
            args.iter()
                .enumerate()
                .filter(|&(k, _)| k != i)
                .map(|(_, s)| s.clone())
                .collect()
        };
        match a.as_str() {
            "--prop-icons" => return crate::icons::run_prop_icons(&rest()),
            "--gen-icons" => return crate::icons::run_gen_icons(&rest()),
            "--avatar" => return run_from_args(&rest()),
            "--avatar-info" => {
                let mut argv = vec!["--avatar-info".to_owned()];
                argv.extend(rest());
                return run_from_args(&argv);
            }
            _ => {}
        }
    }
    bail!("no bake mode flag (--avatar, --avatar-info, --prop-icons, --gen-icons)")
}
