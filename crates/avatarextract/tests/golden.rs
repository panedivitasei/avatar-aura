// Golden comparisons for modes 2 (--pack), 5 (raw blob) and 8 (--scan-closet) against the C++ avatarextract.
//! Skips when the fixture tree is absent.

mod common;

use common::*;
use std::ffi::OsStr;
use std::path::Path;

const ITEMS: [&str; 3] = [
    "00000008-0004-c172-caeb-f9c44d5308c9",
    "00000008-1965-0271-c323-1f77584111f7",
    "00000008-204f-4251-ceca-b43857520802",
];

#[test]
fn raw_blob_items_match_golden() {
    let Some(fx) = fixtures() else { return };
    for guid in ITEMS {
        let input = fx.join("inputs/closet").join(format!("{guid}.bin"));
        let golden = fx.join("golden").join(format!("item_{guid}"));
        let out = scratch(&format!("item-{guid}"));
        let (code, stdout) = run_bin(&[input.as_os_str(), out.as_os_str()]);
        assert_eq!(code, 0, "{guid}");
        assert_model_dir_eq(&out, &golden);
        assert_stdout_eq(&stdout, &golden.join("stdout.txt"));
        let _ = std::fs::remove_dir_all(&out);
    }
}

#[test]
fn pack_matches_golden() {
    let Some(fx) = fixtures() else { return };
    let input = fx.join("inputs/AvatarAssetPack.toc");
    let golden = fx.join("golden/pack");
    let out = scratch("pack");
    let (code, stdout) = run_bin(&[OsStr::new("--pack"), input.as_os_str(), out.as_os_str()]);
    assert_eq!(code, 0);
    assert_model_dir_eq(&out, &golden);
    assert_stdout_eq(&stdout, &golden.join("stdout.txt"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn scan_closet_matches_golden() {
    let Some(fx) = fixtures() else { return };
    let golden = fx.join("golden/scan/stdout.txt");
    let closet = Path::new(r"C:\Users\edward\Documents\ReXGlue\ae-sub\assets\closet");
    if closet.is_dir() {
        let (code, stdout) = run_bin(&[OsStr::new("--scan-closet"), closet.as_os_str()]);
        assert_eq!(code, 0);
        assert!(stdout.contains("closet scan: scanned=225 bad=0"), "{stdout}");
        assert_stdout_eq(&stdout, &golden);
    } else {
        eprintln!("{} absent, scanning the fixture closet only", closet.display());
    }
    let (code, stdout) = run_bin(&[OsStr::new("--scan-closet"), fx.join("inputs/closet").as_os_str()]);
    assert_eq!(code, 0);
    let bins = files_with_ext(&fx.join("inputs/closet"), &[".bin"]).len();
    assert!(
        stdout.contains(&format!("closet scan: scanned={bins} bad=0")),
        "{stdout}"
    );
}
