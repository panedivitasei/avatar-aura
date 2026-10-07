// Tests for modes 1 (STFS package), --extract, 6 (--closet-import), 6b (--closet-icons), 7 (--fix-outfits),
// --refit-female and DetectItemBodies. No real STFS package or raw archive exists on the test machine, so
// packages are synthesized here and archives are hand-built directory trees.

mod common;

use common::*;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use avatarextract::closet_import::detect_item_bodies;

// ---------------------------------------------------------------------------
// Synthetic STFS package
// ---------------------------------------------------------------------------

const OFF_HEADER_SIZE: usize = 0x340;
const OFF_DESCRIPTOR: usize = 0x379;

/// Read-only LIVE package with a flat file table in block 0 and each file in consecutive blocks.
fn synth_package(files: &[(&str, &[u8])]) -> Vec<u8> {
    let bs = stfs::BLOCK_SIZE as usize;
    let header = stfs::HEADER_STRUCT_SIZE as u32;
    let layout = stfs::Layout::new(header, true);
    let mut starts = Vec::new();
    let mut next = 1u32;
    for (_, data) in files {
        starts.push(next);
        next += data.len().div_ceil(bs).max(1) as u32;
    }
    let total = next;
    let end = layout.block_to_offset(total - 1) as usize + bs;
    let mut d = vec![0u8; end];
    d[..4].copy_from_slice(b"LIVE");
    d[OFF_HEADER_SIZE..OFF_HEADER_SIZE + 4].copy_from_slice(&header.to_be_bytes());
    let ds = OFF_DESCRIPTOR;
    d[ds] = 0x24;
    d[ds + 2] = 1;
    d[ds + 3] = 1;
    d[ds + 0x1C..ds + 0x20].copy_from_slice(&total.to_be_bytes());
    let h0 = layout.hash_block_offset(0, 0) as usize;
    let link = |d: &mut Vec<u8>, b: u32, next: u32| {
        let o = h0 + b as usize * 0x18 + 0x14;
        d[o..o + 4].copy_from_slice(&(0x8000_0000 | next).to_be_bytes());
    };
    link(&mut d, 0, stfs::END_OF_CHAIN);
    let ft = layout.block_to_offset(0) as usize;
    for (i, ((name, data), &start)) in files.iter().zip(&starts).enumerate() {
        let blocks = data.len().div_ceil(bs).max(1) as u32;
        for b in 0..blocks {
            let blk = start + b;
            let nxt = if b + 1 == blocks {
                stfs::END_OF_CHAIN
            } else {
                blk + 1
            };
            link(&mut d, blk, nxt);
        }
        let e = ft + i * 0x40;
        d[e..e + name.len()].copy_from_slice(name.as_bytes());
        d[e + 0x28] = name.len() as u8;
        d[e + 0x2C..e + 0x2F].copy_from_slice(&blocks.to_le_bytes()[..3]);
        d[e + 0x2F..e + 0x32].copy_from_slice(&start.to_le_bytes()[..3]);
        d[e + 0x32..e + 0x34].copy_from_slice(&0xFFFFu16.to_be_bytes());
        d[e + 0x34..e + 0x38].copy_from_slice(&(data.len() as u32).to_be_bytes());
        for (b, chunk) in data.chunks(bs).enumerate() {
            let o = layout.block_to_offset(start + b as u32) as usize;
            d[o..o + chunk.len()].copy_from_slice(chunk);
        }
    }
    d
}

#[test]
fn stfs_extract_all_and_bad_blob() {
    let dir = scratch("stfs-junk");
    let junk: Vec<u8> = (0..5000u32).map(|i| (i * 13) as u8).collect();
    let pkg = synth_package(&[("asset.bin", &junk), ("icon.png", b"PNGDATA")]);
    let path = dir.join("item.pkg");
    fs::write(&path, pkg).unwrap();

    let out = dir.join("x");
    let (code, stdout) = run_bin(&[OsStr::new("--extract"), path.as_os_str(), out.as_os_str()]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(fs::read(out.join("asset.bin")).unwrap(), junk);
    assert_eq!(fs::read(out.join("icon.png")).unwrap(), b"PNGDATA");
    assert!(stdout.contains("extracted 2 file(s)"), "{stdout}");

    let (code, stdout) = run_bin(&[path.as_os_str(), dir.join("m").as_os_str()]);
    assert_eq!(code, 4, "{stdout}");
    assert!(stdout.contains("(mode: marketplace STFS package)"));
    assert!(stdout.contains("ERROR: asset blob is not an STRB/YTGR container (first4: 00 0D 1A 27)."));

    let bad = dir.join("bad.pkg");
    fs::write(&bad, b"LIVE but far too short").unwrap();
    let (code, stdout) = run_bin(&[bad.as_os_str(), dir.join("m").as_os_str()]);
    assert_eq!(code, 1);
    assert!(stdout.contains("ERROR: not a valid STFS package"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stfs_package_matches_raw_blob_golden() {
    let Some(fx) = fixtures() else { return };
    let guid = "00000008-204f-4251-ceca-b43857520802";
    let item = fs::read(fx.join("inputs/closet").join(format!("{guid}.bin"))).unwrap();
    let decoy = vec![0u8; item.len() * 2];
    let dir = scratch("stfs-item");
    // asset_v2.bin wins over a larger .bin.
    let pkg = synth_package(&[
        ("big.bin", &decoy),
        ("asset_v2.bin", &item),
        ("icon.png", b"ICON"),
    ]);
    let path = dir.join("item.pkg");
    fs::write(&path, pkg).unwrap();
    let out = dir.join("out");
    let (code, stdout) = run_bin(&[path.as_os_str(), out.as_os_str()]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains(&format!(
        "Using avatar asset blob: asset_v2.bin ({} bytes)",
        item.len()
    )));
    assert!(stdout.contains("Wrote model with 1 batches"), "{stdout}");
    assert_eq!(fs::read(out.join("icon.png")).unwrap(), b"ICON");

    // Mode 1 and mode 5 share WriteModel; only the prefix differs.
    let golden = fx.join("golden").join(format!("item_{guid}"));
    let want_obj = fs::read_to_string(golden.join("item.obj"))
        .unwrap()
        .replace("mtllib item.mtl", "mtllib model.mtl");
    let want_mtl = fs::read_to_string(golden.join("item.mtl"))
        .unwrap()
        .replace("item_texture", "model_texture");
    assert_text_eq(
        &fs::read_to_string(out.join("model.obj")).unwrap(),
        &want_obj,
        "model.obj",
    );
    assert_text_eq(
        &fs::read_to_string(out.join("model.mtl")).unwrap(),
        &want_mtl,
        "model.mtl",
    );
    for png in files_with_ext(&golden, &[".png"]) {
        assert_png_eq(&out.join(png.replace("item_", "model_")), &golden.join(&png));
    }
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Hand-built STRB items
// ---------------------------------------------------------------------------

/// Bare little-endian STRB, 4-byte ids and sizes, no alignment, entry size 1.
fn strb(blocks: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut s = b"STRB".to_vec();
    s.extend_from_slice(&[0, 1]);
    s.extend_from_slice(&[0; 16]);
    s.extend_from_slice(&[4, 4, 0, 0]);
    for (id, data) in blocks {
        s.extend_from_slice(&id.to_le_bytes());
        s.extend_from_slice(&(data.len() as u32).to_le_bytes());
        s.extend_from_slice(&1u32.to_le_bytes());
        s.extend_from_slice(data);
    }
    s
}

/// Raw shape header: count, buffer size, target id {a, b, c, d}.
fn shape(count: u32, a: u32, c: u16) -> (u32, Vec<u8>) {
    let mut v = Vec::new();
    v.extend_from_slice(&count.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&a.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&c.to_le_bytes());
    v.extend_from_slice(&[0; 8]);
    (4, v)
}

/// kModel block whose single chunk header claims `compressed` bytes.
fn model_chunk(compressed: u32, payload: usize) -> (u32, Vec<u8>) {
    let mut v = Vec::new();
    v.extend_from_slice(&compressed.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&0x100u32.to_le_bytes());
    v.resize(12 + payload, 0);
    (3, v)
}

fn bodies(bytes: &[u8]) -> (u32, String) {
    let mut log = Vec::new();
    let v = detect_item_bodies(bytes, &mut log);
    (v, String::from_utf8(log).unwrap())
}

#[test]
fn detect_item_bodies_verdicts() {
    assert_eq!(bodies(&strb(&[shape(3, 2, 1)])).0, 1);
    assert_eq!(bodies(&strb(&[shape(3, 2, 2)])).0, 2);
    assert_eq!(bodies(&strb(&[])).0, 3);
    // A head-targeting shape says nothing; the body shape after it decides.
    assert_eq!(bodies(&strb(&[shape(1, 1, 1), shape(1, 2, 2)])).0, 2);
    assert_eq!(bodies(&strb(&[shape(1, 1, 1)])).0, 3);
    assert_eq!(bodies(&strb(&[model_chunk(8, 8), shape(1, 2, 1)])).0, 1);
    // Malformed: oversized index count, first chunk past the block, short compressed block.
    assert_eq!(bodies(&strb(&[shape(9000, 2, 1)])).0, 0);
    assert_eq!(bodies(&strb(&[model_chunk(100, 8)])).0, 0);
    assert_eq!(bodies(&strb(&[(3, vec![0; 8])])).0, 0);
    // A truncated shape block faults and is logged.
    let mut cut = strb(&[shape(1, 2, 1)]);
    cut.truncate(cut.len() - 12);
    let (v, log) = bodies(&cut);
    assert_eq!(v, 0);
    assert!(log.starts_with("  (item parse fault"), "{log}");
    // Not an STRB at all: no blocks, no shape.
    assert_eq!(bodies(b"nothing here").0, 3);
}

fn set_mtime(path: &Path, t: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

const G1: &str = "00000040-aaaa-0001-c1c8-f109a19cb2e0";
const G2: &str = "00001000-bbbb-0002-c1c8-f109a19cb2e0";
const G3: &str = "00001000-cccc-0003-c1c8-f109a19cb2e0";

/// Layout (a) male hat with ID.TXT, layout (b) dump with a female prop and a malformed prop, and an unpaired blob.
fn build_archive(raw: &Path) {
    let hat = raw.join("Male Hat");
    fs::create_dir_all(&hat).unwrap();
    fs::write(hat.join("asset_v2.bin"), strb(&[shape(1, 2, 1)])).unwrap();
    fs::write(hat.join("ID.TXT"), format!("{}\r\n", G1.to_uppercase())).unwrap();
    fs::write(hat.join("ICON.PNG"), b"hat icon").unwrap();

    let dump = raw.join("Dump");
    fs::create_dir_all(&dump).unwrap();
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let b2 = dump.join(format!("{G2}.bin"));
    let b3 = dump.join(format!("{G3}.bin"));
    let art = dump.join("thumbm.png");
    fs::write(&b2, strb(&[shape(1, 2, 2)])).unwrap();
    fs::write(&b3, strb(&[model_chunk(100, 8)])).unwrap();
    fs::write(&art, b"prop art").unwrap();
    set_mtime(&b2, t0);
    set_mtime(&art, t0 + Duration::from_secs(10));
    set_mtime(&b3, t0 + Duration::from_secs(20));

    let noid = raw.join("NoId");
    fs::create_dir_all(&noid).unwrap();
    fs::write(noid.join("asset_v2.bin"), strb(&[])).unwrap();
}

#[test]
fn closet_import_and_resume() {
    let dir = scratch("closet-import");
    let raw = dir.join("raw");
    let closet = dir.join("closet");
    build_archive(&raw);

    let args = [
        OsStr::new("--closet-import"),
        raw.as_os_str(),
        closet.as_os_str(),
        OsStr::new("--icon-cat=1000"),
    ];
    let (code, stdout) = run_bin(&args);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("(mode: closet import)"));
    assert!(stdout.contains("  skipping malformed item: "), "{stdout}");
    assert!(
        stdout.contains(
            "closet import: scanned=4 imported=2 (icons=1, no-icon=0) dupes=0 failed=2 (unpaired=1) poisoned=0"
        ),
        "{stdout}"
    );
    let index = fs::read_to_string(closet.join("closet_index.tsv")).unwrap();
    assert_eq!(
        index,
        format!("{G1}\t00000040\t1\tMale Hat\n{G2}\t00001000\t2\tDump\n")
    );
    assert_eq!(
        fs::read(closet.join(format!("{G2}.bin"))).unwrap(),
        strb(&[shape(1, 2, 2)])
    );
    assert_eq!(
        fs::read(closet.join("icons").join(format!("{G2}.png"))).unwrap(),
        b"prop art"
    );
    assert!(!closet.join("icons").join(format!("{G1}.png")).exists());
    assert!(!closet.join(format!("{G3}.bin")).exists());

    // The malformed guid was journaled but never indexed, so the rerun poisons it.
    let (code, stdout) = run_bin(&args);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("  resuming: 2 indexed, 1 poison-skipped"),
        "{stdout}"
    );
    assert!(
        stdout.contains("scanned=4 imported=0 (icons=0, no-icon=0) dupes=2 failed=1 (unpaired=1) poisoned=1"),
        "{stdout}"
    );
    assert!(closet.join("closet_index.tsv.bak").exists());

    // Backfill: the hat gets its art, the generated stand-in for the prop yields to store art.
    let gen = closet.join("icons_generated.tsv");
    fs::write(
        &gen,
        format!("{G2}\tprop\nffffffff-0000-0000-0000-000000000000\tkeep\n"),
    )
    .unwrap();
    fs::write(closet.join("icons").join(format!("{G2}.png")), b"generated").unwrap();
    let (code, stdout) = run_bin(&[OsStr::new("--closet-icons"), raw.as_os_str(), closet.as_os_str()]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("  generated stand-ins replaced by store art: 1"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "closet icons: scanned=4 copied=2 already-present=0 no-icon=0 not-in-index=2 category-filtered=0"
        ),
        "{stdout}"
    );
    assert_eq!(
        fs::read(closet.join("icons").join(format!("{G1}.png"))).unwrap(),
        b"hat icon"
    );
    assert_eq!(
        fs::read(closet.join("icons").join(format!("{G2}.png"))).unwrap(),
        b"prop art"
    );
    assert_eq!(
        fs::read_to_string(&gen).unwrap(),
        "ffffffff-0000-0000-0000-000000000000\tkeep\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn closet_icons_needs_an_index() {
    let dir = scratch("closet-icons-empty");
    let (code, stdout) = run_bin(&[
        OsStr::new("--closet-icons"),
        dir.as_os_str(),
        dir.join("closet").as_os_str(),
    ]);
    assert_eq!(code, 1);
    assert!(stdout.contains("ERROR: no closet_index.tsv (or empty)"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn closet_scan_flags_bad_items() {
    let dir = scratch("closet-scan");
    fs::write(dir.join("good.bin"), strb(&[shape(1, 2, 1)])).unwrap();
    fs::write(dir.join("bad.bin"), strb(&[model_chunk(100, 8)])).unwrap();
    fs::write(dir.join("empty.bin"), b"").unwrap();
    fs::write(dir.join("notes.txt"), b"x").unwrap();
    let (code, stdout) = run_bin(&[OsStr::new("--scan-closet"), dir.as_os_str()]);
    assert_eq!(code, 2);
    assert!(stdout.contains("BAD bad.bin"));
    assert!(stdout.contains("BAD empty.bin"));
    assert!(stdout.contains("closet scan: scanned=3 bad=2"), "{stdout}");
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Mode 7 and --refit-female (need the asset pack fixture)
// ---------------------------------------------------------------------------

const STOCK_TAIL: [u8; 8] = [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0];

#[test]
fn fix_outfits_matches_cpp() {
    let Some(fx) = fixtures() else { return };
    let dir = scratch("outfits");
    let mut buf = vec![0u8; 1200];
    let offs: Vec<usize> = (0..13)
        .map(|i| 0x160 + 0x20 * i)
        .chain([0x300, 0x320, 0x340, 0x360])
        .collect();
    let bs = [
        0u16, 1, 2, 3, 40, 100, 200, 300, 500, 700, 900, 1100, 1281, 1282, 5, 77, 0,
    ];
    for (i, (&o, &b)) in offs.iter().zip(&bs).enumerate() {
        let (a, c) = if i == 16 {
            (0u32, 0u16)
        } else {
            (1u32 << (i % 13), 7u16)
        };
        buf[o..o + 4].copy_from_slice(&a.to_be_bytes());
        buf[o + 4..o + 6].copy_from_slice(&b.to_be_bytes());
        buf[o + 6..o + 8].copy_from_slice(&c.to_be_bytes());
        buf[o + 8..o + 16].copy_from_slice(&STOCK_TAIL);
    }
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("sub/OUTFIT00_test"), &buf).unwrap();
    fs::write(dir.join("OUTFIT00_x.header"), &buf).unwrap();
    fs::write(dir.join("OUTFIT00_small"), &buf[..500]).unwrap();
    let toc = fx.join("inputs/AvatarAssetPack.toc");
    let (code, stdout) = run_bin(&[OsStr::new("--fix-outfits"), toc.as_os_str(), dir.as_os_str()]);
    assert_eq!(code, 0);
    // Captured from the C++ tool on the same input.
    let want = r"  OUTFIT00_test @0x160: 00000001-0000-0007 -> 00000001-0000-0001
  OUTFIT00_test @0x180: 00000002-0001-0007 -> 00000002-0001-0002
  OUTFIT00_test @0x1a0: 00000004-0002-0007 -> 00000004-0002-0003
  OUTFIT00_test @0x1c0: 00000008-0003-0007 -> 00000008-0003-0003
  OUTFIT00_test @0x1e0: 00000010-0028-0007 -> 00000010-0028-0003
  OUTFIT00_test @0x200: 00000020-0064-0007 -> 00000020-0064-0001
  OUTFIT00_test @0x220: 00000040-00C8-0007 -> 00000040-00C8-0001
  OUTFIT00_test @0x240: 00000080-012C-0007 -> 00000080-012C-0002
  OUTFIT00_test @0x260: 00000100-01F4-0007 -> 00000100-01F4-0003
  OUTFIT00_test @0x280: 00000200-02BC-0007 -> 00000200-02BC-0003
  OUTFIT00_test @0x2a0: 00000400-0384-0007 -> 00000208-0053-0001
  OUTFIT00_test @0x2c0: 00000800-044C-0007 -> 00000800-044C-0003
  OUTFIT00_test @0x2e0: 00001000-0501-0007 -> 00001000-0501-0002
  OUTFIT00_test @0x320: 00000002-0005-0007 -> 00000002-0005-0003
  OUTFIT00_test @0x340: 00000004-004D-0007 -> 00000004-004D-0001
fix-outfits: 1 files scanned, 1 patched (15 ids)
";
    let body: String = stdout
        .lines()
        .skip_while(|l| !l.starts_with("  OUTFIT00_"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_text_eq(&body, want, "fix-outfits stdout");
    let patched = fs::read(dir.join("sub/OUTFIT00_test")).unwrap();
    assert_eq!(&patched[0x2a0..0x2a8], &[0, 0, 0x02, 0x08, 0, 0x53, 0, 1]);
    assert_eq!(fs::read(dir.join("OUTFIT00_x.header")).unwrap(), buf);
    let _ = fs::remove_dir_all(&dir);
}

fn fnv1a64(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn refit_female_matches_cpp() {
    let Some(fx) = fixtures() else { return };
    let dir = scratch("refit");
    let toc = fx.join("inputs/AvatarAssetPack.toc");
    let item = fx.join("inputs/closet/00000008-204f-4251-ceca-b43857520802.bin");
    let out = dir.join("refit.bin");
    let (code, stdout) = run_bin(&[
        OsStr::new("--refit-female"),
        item.as_os_str(),
        out.as_os_str(),
        OsStr::new("--toc"),
        toc.as_os_str(),
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("female shape: 466 hidden tris (male had 340), 73 tucks (male had 64)"));
    assert!(stdout.contains("round-trip OK: max tuck quantization error 0.3006 mm; applies to the female body: yes (466 triangles collapsed)"));
    // Size and FNV-1a of the C++ tool's output for the same item.
    let bytes = fs::read(&out).unwrap();
    assert_eq!(bytes.len(), 35056);
    assert_eq!(fnv1a64(&bytes), 0x31eb_590e_3963_c0e8);

    // The refit item now carries a female shape.
    let (code, stdout) = run_bin(&[
        OsStr::new("--refit-female"),
        out.as_os_str(),
        dir.join("again.bin").as_os_str(),
        OsStr::new("--toc"),
        toc.as_os_str(),
    ]);
    assert_eq!(code, 0);
    assert!(stdout.contains("item already carries a female body shape; nothing to do"));

    let (code, stdout) = run_bin(&[OsStr::new("--refit-female"), item.as_os_str(), out.as_os_str()]);
    assert_eq!(code, 1);
    assert!(stdout.contains("ERROR: --toc <AvatarAssetPack.toc> is required"));
    let _ = fs::remove_dir_all(&dir);
}
