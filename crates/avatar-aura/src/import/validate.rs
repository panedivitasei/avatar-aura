// Port of avatar_aura/importing.py: the Dry Cleaner gates for STFS item containers, award packages and raw blobs.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

/// An input refused by one of the importer gates.
#[derive(Debug)]
pub struct PackageError(pub String);

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackageError {}

pub type Result<T> = std::result::Result<T, PackageError>;

fn refuse<T>(message: impl Into<String>) -> Result<T> {
    Err(PackageError(message.into()))
}

const CATEGORY_BITS: [(u32, &str); 14] = [
    (1 << 0, "Head"),
    (1 << 1, "Body"),
    (1 << 2, "Hair"),
    (1 << 3, "Top"),
    (1 << 4, "Bottom"),
    (1 << 5, "Shoes"),
    (1 << 6, "Hat"),
    (1 << 7, "Gloves"),
    (1 << 8, "Glasses"),
    (1 << 9, "Wristwear"),
    (1 << 10, "Earrings"),
    (1 << 11, "Ring"),
    (1 << 12, "Prop"),
    (1 << 22, "Animation"),
];

pub const ANIMATION_CATEGORY: u32 = 1 << 22;

fn block_name(id: u64) -> Option<&'static str> {
    Some(match id {
        1 => "kAnimation",
        2 => "kTexture",
        3 => "kModel",
        4 => "kShapeOverrides",
        5 => "kSkeleton",
        6 => "kAssetMetadataUnversioned",
        7 => "kCustomColorTable",
        8 => "kAssetMetadataVersioned",
        _ => return None,
    })
}

const BODY_NAMES: [&str; 4] = ["?", "male", "female", "both"];

pub fn category_names(mask: u32) -> String {
    let names: Vec<&str> = CATEGORY_BITS
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, n)| *n)
        .collect();
    if names.is_empty() {
        "(none)".into()
    } else {
        names.join("|")
    }
}

pub fn bodies_label(bodies: u32) -> &'static str {
    match bodies {
        1 => "Male",
        2 => "Female",
        3 => "Unisex",
        _ => "Unknown",
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Meta {
    pub bodies: u8,
    pub categories: u32,
    pub subcategory: u32,
}

/// One validated avatar item.
#[derive(Clone, Debug)]
pub struct ItemResult {
    pub guid: String,
    pub categories: u32,
    pub bodies: u32,
    pub name: String,
    pub title_id: u32,
    pub title_name: String,
    pub description: String,
    pub is_award: bool,
    pub meta: Option<Meta>,
    pub bin_bytes: Vec<u8>,
    pub icon_bytes: Option<Vec<u8>>,
    pub log: Vec<String>,
    pub warnings: Vec<String>,
}

impl Default for ItemResult {
    fn default() -> Self {
        Self {
            guid: String::new(),
            categories: 0,
            bodies: 3,
            name: String::new(),
            title_id: 0,
            title_name: String::new(),
            description: String::new(),
            is_award: false,
            meta: None,
            bin_bytes: Vec::new(),
            icon_bytes: None,
            log: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

fn be32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0)
}

fn le32(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0)
}

fn is_stfs_magic(d: &[u8]) -> bool {
    matches!(d.get(..4), Some(b"LIVE" | b"PIRS" | b"CON "))
}

/// Files of a LIVE/PIRS/CON container, read through the `stfs` crate with the Python size gates and log lines.
pub fn parse_stfs(data: &[u8], res: &mut ItemResult, expect_item: bool) -> Result<Vec<(String, Vec<u8>)>> {
    if data.len() < 0xB000 {
        return refuse("file too small to be an STFS container");
    }
    if !is_stfs_magic(data) {
        return refuse(format!(
            "not an STFS container (magic {:?})",
            String::from_utf8_lossy(&data[..4])
        ));
    }
    let magic = String::from_utf8_lossy(&data[..4]).trim().to_string();
    res.log
        .push(format!("container: {magic} STFS, {} bytes", data.len()));
    let package =
        stfs::Package::parse(data).map_err(|e| PackageError(format!("cannot read the container: {e}")))?;
    if expect_item && package.content_type != 0x9000 {
        return refuse(format!(
            "content type {:#x} is not Avatar Item (0x9000)",
            package.content_type
        ));
    }
    let name = package.display_name.trim();
    res.name = if name.is_empty() {
        "(unnamed)".into()
    } else {
        name.into()
    };
    res.title_id = package.title_id;
    res.title_name = package.title_name.trim().to_string();
    res.description = package
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    res.log.push(format!(
        "display name: {:?}  title id: {:08X}  title: {:?}",
        res.name, res.title_id, res.title_name
    ));
    let mut files = Vec::new();
    for entry in package.entries().iter().filter(|e| !e.is_directory) {
        if entry.size > 16 << 20 || entry.allocated_blocks > package.descriptor.total_block_count + 16 {
            return refuse(format!(
                "file entry {:?} claims absurd size {}",
                entry.name, entry.size
            ));
        }
        let bytes = package
            .read(entry)
            .map_err(|e| PackageError(format!("{:?}: {e}", entry.name)))?;
        if bytes.len() != entry.size as usize {
            return refuse(format!(
                "{:?}: truncated ({}/{} bytes)",
                entry.name,
                bytes.len(),
                entry.size
            ));
        }
        res.log
            .push(format!("  contains {}: {} bytes", entry.name, entry.size));
        files.push((entry.name.clone(), bytes));
    }
    if files.is_empty() {
        return refuse("no files in the container");
    }
    Ok(files)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub fn scan_signatures(blob: &[u8], label: &str) -> Result<()> {
    let patterns: [&[u8]; 6] = [
        b"MZ\x90",
        b"This program cannot",
        b"XEX2",
        b"PK\x03\x04",
        b"<script",
        b"powershell",
    ];
    for pat in patterns {
        if let Some(i) = find(blob, pat) {
            return refuse(format!(
                "{label}: executable/script signature {:?} at {i}",
                String::from_utf8_lossy(pat)
            ));
        }
    }
    Ok(())
}

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

/// Validates the STRB container and returns the bodies verdict (1, 2 or 3).
pub fn validate_strb(blob: &[u8], res: &mut ItemResult) -> Result<u32> {
    let mut buf = blob;
    if buf.starts_with(b"YTGR") {
        if buf.len() < 0x140 {
            return refuse("YTGR wrapper truncated");
        }
        buf = &buf[0x140..];
        res.log.push("  YTGR signature wrapper skipped".into());
    }
    if !buf.starts_with(b"STRB") || buf.len() < 30 {
        let head: String = buf.iter().take(4).map(|b| format!("{b:02x}")).collect();
        return refuse(format!("payload is not an STRB container (magic {head})"));
    }
    let has_align = buf[4] != 0;
    let is_le = buf[5] != 0;
    let (id_size, size_size) = (usize::from(buf[22]), usize::from(buf[23]));
    let alignment = if has_align { usize::from(buf[26]) } else { 1 };
    let widths = [1, 2, 4, 8];
    if alignment == 0 || !widths.contains(&id_size) || !widths.contains(&size_size) {
        return refuse("malformed STRB header field widths");
    }
    let rd = |off: usize, sz: usize| -> u64 {
        let bytes = &buf[off..off + sz];
        let fold = |v: u64, b: &u8| (v << 8) | u64::from(*b);
        if is_le {
            bytes.iter().rev().fold(0, fold)
        } else {
            bytes.iter().fold(0, fold)
        }
    };
    let bhs = align(id_size + size_size + size_size, alignment);
    let mut off = align(if has_align { 30 } else { 26 }, alignment);
    let mut blocks: Vec<(u64, usize, usize)> = Vec::new();
    while off + bhs <= buf.len() {
        let bid = rd(off, id_size);
        let data_size = rd(off + id_size, size_size);
        let entry_size = rd(off + id_size + size_size, size_size);
        off += bhs;
        let Some(name) = block_name(bid) else {
            return refuse(format!("unknown STRB block id {bid}"));
        };
        let payload = data_size.saturating_mul(entry_size);
        if (off as u64).saturating_add(payload) > buf.len() as u64 {
            return refuse(format!("STRB block {name} extends past container"));
        }
        blocks.push((bid, off, payload as usize));
        off = off.saturating_add(align(data_size as usize, alignment));
    }
    let mut census: BTreeMap<u64, usize> = BTreeMap::new();
    for (bid, _, _) in &blocks {
        *census.entry(*bid).or_default() += 1;
    }
    res.log.push(format!(
        "  blocks: {}",
        census
            .iter()
            .map(|(k, v)| format!("{}={v}", block_name(*k).unwrap_or("?")))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    let mut bodies = 3;
    for (bid, boff, blen) in blocks {
        let blk = &buf[boff..boff + blen];
        let name = block_name(bid).unwrap_or("?");
        match bid {
            2 | 3 => {
                if blk.len() < 12 {
                    return refuse(format!("{name} block under 12 bytes"));
                }
                let (mut o, mut total, mut chunks) = (0usize, 0u64, 0usize);
                while o + 12 <= blk.len() {
                    let cs = le32(blk, o) as usize;
                    let uo = u64::from(le32(blk, o + 4));
                    let us = u64::from(le32(blk, o + 8));
                    o += 12;
                    if cs > blk.len() - o {
                        return refuse(format!(
                            "{name} chunk {chunks}: compressed data extends past the block"
                        ));
                    }
                    if uo != total {
                        return refuse(format!("{name} chunk {chunks}: bad uncompressed offset"));
                    }
                    total += us;
                    o += cs;
                    chunks += 1;
                }
                if total > 64 << 20 {
                    return refuse(format!("{name} claims {total} uncompressed bytes"));
                }
                res.log.push(format!(
                    "  {name}: {chunks} LZX chunks, {total} bytes uncompressed, chain in bounds"
                ));
            }
            8 if blk.len() >= 15 => {
                let meta = Meta {
                    bodies: blk[1],
                    categories: le32(blk, 6),
                    subcategory: le32(blk, 10),
                };
                res.log.push(format!(
                    "  metadata: categories {:08X} = {}, bodies {}, subcategory {}",
                    meta.categories,
                    category_names(meta.categories),
                    BODY_NAMES[usize::from(meta.bodies & 3)],
                    meta.subcategory
                ));
                res.meta = Some(meta);
            }
            4 => {
                if blk.len() < 24 {
                    continue;
                }
                let count = le32(blk, 0);
                let a = le32(blk, 8);
                let c = u16::from_le_bytes([blk[14], blk[15]]);
                if count > 8192 {
                    return refuse(format!(
                        "shape index_count {count} > 8192 (the runtime parser trusts it)"
                    ));
                }
                if a == 2 && (c == 1 || c == 2) {
                    bodies = u32::from(c);
                }
            }
            _ => {}
        }
    }
    Ok(bodies)
}

/// Full PNG structural validation; true means safe to carry verbatim.
pub fn validate_png(d: &[u8], label: &str, res: &mut ItemResult) -> bool {
    if !d.starts_with(b"\x89PNG\r\n\x1a\n") {
        res.warnings.push(format!("{label}: not a PNG, dropped"));
        return false;
    }
    let mut o = 8usize;
    let mut idat = Vec::new();
    let mut meta: Option<(u32, u32, u8, u8, u8)> = None;
    let mut saw_iend = false;
    while o + 8 <= d.len() {
        let ln = be32(d, o) as usize;
        let ctype: [u8; 4] = [d[o + 4], d[o + 5], d[o + 6], d[o + 7]];
        if o.saturating_add(12).saturating_add(ln) > d.len() {
            res.warnings.push(format!("{label}: truncated chunk, dropped"));
            return false;
        }
        let body = &d[o + 8..o + 8 + ln];
        let crc = be32(d, o + 8 + ln);
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&ctype);
        hasher.update(body);
        if hasher.finalize() != crc {
            res.warnings.push(format!("{label}: chunk CRC mismatch, dropped"));
            return false;
        }
        match &ctype {
            b"IHDR" => {
                if body.len() < 13 {
                    res.warnings.push(format!("{label}: bad IHDR, dropped"));
                    return false;
                }
                let (w, h) = (be32(body, 0), be32(body, 4));
                let (depth, ctv, comp, filt, ilace) = (body[8], body[9], body[10], body[11], body[12]);
                meta = Some((w, h, depth, ctv, ilace));
                if !(0 < w && w <= 2048 && 0 < h && h <= 2048) || comp != 0 || filt != 0 {
                    res.warnings.push(format!("{label}: bad IHDR ({w}x{h}), dropped"));
                    return false;
                }
            }
            b"IDAT" => idat.extend_from_slice(body),
            b"PLTE" | b"IEND" => {}
            other if other[0] & 0x20 == 0 => {
                res.warnings.push(format!(
                    "{label}: unknown critical chunk {:?}, dropped",
                    String::from_utf8_lossy(other)
                ));
                return false;
            }
            _ => {}
        }
        o += 12 + ln;
        if &ctype == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    let Some((w, h, depth, ctv, ilace)) = meta.filter(|_| saw_iend) else {
        res.warnings.push(format!("{label}: no IEND/IHDR, dropped"));
        return false;
    };
    if o != d.len() {
        res.warnings
            .push(format!("{label}: {} trailing bytes after IEND", d.len() - o));
    }
    let Ok(raw) = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, 64 << 20) else {
        res.warnings
            .push(format!("{label}: IDAT inflate failed, dropped"));
        return false;
    };
    let channels: u64 = match ctv {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 0,
    };
    let row = (u64::from(w) * channels * u64::from(depth)).div_ceil(8);
    if ilace == 0 && channels != 0 && raw.len() as u64 != u64::from(h) * (1 + row) {
        res.warnings
            .push(format!("{label}: raster size mismatch, dropped"));
        return false;
    }
    res.log
        .push(format!("  {label}: {w}x{h} PNG validated (CRCs, raster size)"));
    true
}

/// 32-hex or dashed names to the dashed lowercase guid.
pub fn guid_from_stem(stem: &str) -> Option<String> {
    let s = stem.to_ascii_lowercase();
    let hex = |t: &str| t.bytes().all(|c| c.is_ascii_hexdigit());
    if s.len() == 32 && hex(&s) {
        return Some(format!(
            "{}-{}-{}-{}-{}",
            &s[0..8],
            &s[8..12],
            &s[12..16],
            &s[16..20],
            &s[20..32]
        ));
    }
    let b = s.as_bytes();
    if s.len() == 36 && b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-' {
        let h = s.replace('-', "");
        if h.len() == 32 && hex(&h) {
            return Some(s);
        }
    }
    None
}

fn hex_field(guid: &str, from: usize, to: usize) -> u32 {
    let h = guid.replace('-', "");
    h.get(from..to)
        .and_then(|t| u32::from_str_radix(t, 16).ok())
        .unwrap_or(0)
}

fn award_flag(guid: &str) -> bool {
    let c = guid
        .get(14..18)
        .and_then(|t| u32::from_str_radix(t, 16).ok())
        .unwrap_or(0);
    (c >> 8) & 0xF == 1
}

fn clean_text(res: &mut ItemResult) {
    for s in [&mut res.name, &mut res.title_name, &mut res.description] {
        *s = s.replace(['\t', '\n', '\r'], " ");
    }
}

fn verdict(res: &ItemResult) -> String {
    format!(
        "verdict: clean. {} item, categories {:08X} = {}",
        BODY_NAMES[res.bodies as usize & 3],
        res.categories,
        category_names(res.categories)
    )
}

fn metadata_mismatch(res: &mut ItemResult) {
    if let Some(meta) = res.meta {
        if meta.categories != res.categories {
            res.warnings.push(format!(
                "metadata says categories {:08X}, product id says {:08X}; keeping the product id's value",
                meta.categories, res.categories
            ));
        }
    }
}

fn finish_item(files: &[(String, Vec<u8>)], res: &mut ItemResult) -> Result<()> {
    let mut blob_index = files
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case("asset_v2.bin"));
    if blob_index.is_none() {
        blob_index = files
            .iter()
            .position(|(_, c)| c.starts_with(b"STRB") || c.starts_with(b"YTGR"));
        if let Some(i) = blob_index {
            res.warnings
                .push(format!("no asset_v2.bin; using {:?}", files[i].0));
        }
    }
    let Some(blob_index) = blob_index else {
        return refuse("container holds no STRB asset payload");
    };
    let blob = &files[blob_index].1;
    scan_signatures(blob, "asset blob")?;
    res.bodies = validate_strb(blob, res)?;
    res.bin_bytes = blob.clone();
    metadata_mismatch(res);
    let icon = files
        .iter()
        .enumerate()
        .find(|(i, (n, _))| *i != blob_index && n.to_ascii_lowercase().ends_with(".png"));
    if let Some((_, (name, content))) = icon {
        if validate_png(content, name, res) {
            res.icon_bytes = Some(content.clone());
        }
    }
    clean_text(res);
    res.is_award = award_flag(&res.guid);
    let mut line = verdict(res);
    if res.is_award {
        let title = if res.title_name.is_empty() {
            "?"
        } else {
            &res.title_name
        };
        line.push_str(&format!("; AWARD from {title} ({:08X})", res.title_id));
    }
    res.log.push(line);
    Ok(())
}

fn unescape_xml(s: &str) -> String {
    s.replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Display name for a guid-named raw blob: the marketplace `<guid>.xml` fullTitle, else the folder name, else the guid.
fn sidecar_name(path: &Path, guid: &str) -> String {
    let dir = path.parent().unwrap_or(Path::new(""));
    for cand in [dir.join(format!("{guid}.xml")), path.with_extension("xml")] {
        let Ok(bytes) = std::fs::read(&cand) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        if let Some(start) = text.find("<fullTitle>") {
            let rest = &text[start + "<fullTitle>".len()..];
            if let Some(end) = rest.find("</fullTitle>") {
                let title = rest[..end].trim();
                if !title.is_empty() {
                    return unescape_xml(title);
                }
            }
        }
    }
    std::path::absolute(path)
        .ok()
        .and_then(|p| {
            p.parent()
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| guid.to_string())
}

/// Art beside a raw blob: `<stem>.png`, else the marketplace thumbm/thumbsm.png when the folder holds one .bin.
fn sidecar_icon(path: &Path, res: &mut ItemResult) -> Option<Vec<u8>> {
    let folder = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut cands = vec![path.with_extension("png")];
    let bins = std::fs::read_dir(&folder)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .ends_with(".bin")
                })
                .count()
        })
        .unwrap_or(0);
    if bins == 1 {
        cands.push(folder.join("thumbm.png"));
        cands.push(folder.join("thumbsm.png"));
    }
    for cand in cands {
        if let Ok(data) = std::fs::read(&cand) {
            if validate_png(&data, &file_label(&cand), res) {
                return Some(data);
            }
        }
    }
    None
}

fn stem_of(path: &Path) -> String {
    let name = file_label(path);
    match name.rsplit_once('.') {
        Some((stem, _)) => stem.to_string(),
        None => name,
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A bare YTGR/STRB blob: guid-named files keep their product id, nameless ones get one synthesized from the content.
pub fn process_raw_bin(path: &Path, data: &[u8]) -> Result<Vec<ItemResult>> {
    let mut res = ItemResult::default();
    res.log.push(format!(
        "raw item blob: {:?}, {} bytes",
        String::from_utf8_lossy(&data[..4]),
        data.len()
    ));
    let stem = stem_of(path);
    let guid = guid_from_stem(&stem);
    scan_signatures(data, "asset blob")?;
    res.bodies = validate_strb(data, &mut res)?;
    res.bin_bytes = data.to_vec();
    if let Some(guid) = guid {
        res.categories = hex_field(&guid, 0, 8);
        res.title_id = hex_field(&guid, 24, 32);
        res.name = sidecar_name(path, &guid);
        res.guid = guid;
        metadata_mismatch(&mut res);
    } else {
        let Some(meta) = res.meta else {
            return refuse(format!(
                "{} has no product id and no asset-metadata block (STRB block 8), so its kind cannot be determined",
                file_label(path)
            ));
        };
        res.categories = meta.categories & 0x1FF_FFFF;
        if meta.bodies & 3 != 0 {
            res.bodies = u32::from(meta.bodies & 3);
        }
        let b = crc32fast::hash(data) & 0xFFFF;
        let digest = Sha1::digest(data);
        let mut tail: [u8; 8] = [0; 8];
        tail.copy_from_slice(&digest[..8]);
        if tail == [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0] {
            tail[7] = 0;
        }
        let hex = |s: &[u8]| s.iter().map(|b| format!("{b:02x}")).collect::<String>();
        res.guid = format!(
            "{:08x}-{b:04x}-{:04x}-{}-{}",
            res.categories,
            res.bodies,
            hex(&tail[..2]),
            hex(&tail[2..])
        );
        let name = stem.replace('_', " ").trim().to_string();
        res.name = if name.is_empty() { "(unnamed)".into() } else { name };
        res.log.push(format!(
            "no product id: classified by the metadata block, synthesized {} from the content",
            res.guid
        ));
    }
    res.icon_bytes = sidecar_icon(path, &mut res);
    clean_text(&mut res);
    res.is_award = award_flag(&res.guid);
    let mut line = verdict(&res);
    if res.icon_bytes.is_none() {
        line.push_str("; no icon beside it");
    }
    res.log.push(line);
    Ok(vec![res])
}

/// One result for an item container or raw blob, several for an award package of nested `<guid>.acp` items.
pub fn process_package(path: &Path) -> Result<Vec<ItemResult>> {
    let data = std::fs::read(path).map_err(|e| PackageError(format!("{}: {e}", path.display())))?;
    if data.len() > 64 << 20 {
        return refuse("file over 64MB, not an avatar item container");
    }
    if data.starts_with(b"YTGR") || data.starts_with(b"STRB") {
        return process_raw_bin(path, &data);
    }
    if data.len() < 0x348 {
        return refuse("file too small to be an STFS container");
    }
    let stem = stem_of(path);
    if is_stfs_magic(&data) && be32(&data, 0x344) == 0x9000 {
        let Some(guid) = guid_from_stem(&stem) else {
            return refuse(format!(
                "filename {stem:?} is not the item's product guid (32-hex or dashed). Item containers are named by their product id, which the closet needs"
            ));
        };
        let mut res = ItemResult {
            categories: hex_field(&guid, 0, 8),
            guid,
            ..ItemResult::default()
        };
        let files = parse_stfs(&data, &mut res, true)?;
        finish_item(&files, &mut res)?;
        return Ok(vec![res]);
    }
    let mut outer = ItemResult::default();
    let files = parse_stfs(&data, &mut outer, false)?;
    let mut results = Vec::new();
    for (name, content) in &files {
        let child_stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
        let Some(guid) = guid_from_stem(child_stem) else {
            continue;
        };
        if !is_stfs_magic(content) {
            continue;
        }
        let mut res = ItemResult {
            categories: hex_field(&guid, 0, 8),
            guid,
            ..ItemResult::default()
        };
        res.log.push(format!("nested item: {name}"));
        let child = parse_stfs(content, &mut res, true)?;
        finish_item(&child, &mut res)?;
        if res.title_name.is_empty() && outer.name != "(unnamed)" {
            res.title_name = outer.name.clone();
        }
        results.push(res);
    }
    if results.is_empty() {
        return refuse("not an avatar item container, and no nested avatar item containers found inside");
    }
    Ok(results)
}

pub const ICON_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "gif"];

/// Image formats the runtime decoder reads; the canonical extension, or `None` with the reason in warnings.
pub fn validate_icon_image(d: &[u8], label: &str, res: &mut ItemResult) -> Option<&'static str> {
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        return validate_png(d, label, res).then_some("png");
    }
    if d.starts_with(b"\xff\xd8\xff") {
        let tail = &d[d.len().saturating_sub(64)..];
        if d.len() < 256 || find(tail, b"\xff\xd9").is_none() {
            res.warnings.push(format!("{label}: truncated JPEG, dropped"));
            return None;
        }
        return Some("jpg");
    }
    if d.starts_with(b"BM") && d.len() >= 54 {
        if le32(d, 2) as usize > d.len() {
            res.warnings
                .push(format!("{label}: BMP header size exceeds file, dropped"));
            return None;
        }
        return Some("bmp");
    }
    if (d.starts_with(b"GIF87a") || d.starts_with(b"GIF89a")) && d.len() >= 13 {
        return Some("gif");
    }
    res.warnings
        .push(format!("{label}: not a png/jpg/bmp/gif image, dropped"));
    None
}

fn probe_reason(probe: &ItemResult, fallback: &str) -> String {
    if probe.warnings.is_empty() {
        fallback.to_string()
    } else {
        probe.warnings.join("; ")
    }
}

/// A package with award items needs a valid game icon before anything is written; `None` without awards.
pub fn require_game_icon(results: &[ItemResult], icon: &Path) -> Result<Option<Vec<u8>>> {
    if !results.iter().any(|r| r.is_award) {
        return Ok(None);
    }
    if icon.as_os_str().is_empty() {
        return refuse(
            "this container holds avatar awards. Set the game's icon first (Avatar award icons). Nothing was written",
        );
    }
    if !icon.is_file() {
        return refuse(format!("game icon not found: {}", icon.display()));
    }
    let data = std::fs::read(icon).map_err(|e| PackageError(e.to_string()))?;
    let mut probe = ItemResult::default();
    if validate_icon_image(&data, &file_label(icon), &mut probe).is_none() {
        return refuse(format!(
            "game icon rejected: {}",
            probe_reason(&probe, "unreadable")
        ));
    }
    Ok(Some(data))
}

/// Copies a validated image to `<closet>/titles/<TITLEID>.<ext>`, removing the title's icons of other formats.
pub fn install_title_icon(closet: &Path, title_id: &str, icon: &Path) -> Result<PathBuf> {
    let tid = title_id.trim().to_ascii_uppercase();
    if tid.len() != 8 || !tid.bytes().all(|c| c.is_ascii_hexdigit()) {
        return refuse(format!("title id {title_id:?} is not 8 hex digits"));
    }
    let data = std::fs::read(icon).map_err(|e| PackageError(format!("{}: {e}", icon.display())))?;
    let mut probe = ItemResult::default();
    let Some(ext) = validate_icon_image(&data, &file_label(icon), &mut probe) else {
        return refuse(probe_reason(&probe, "unreadable image"));
    };
    let dir = closet.join("titles");
    std::fs::create_dir_all(&dir).map_err(|e| PackageError(e.to_string()))?;
    for other in ICON_EXTS.iter().filter(|e| **e != ext) {
        let _ = std::fs::remove_file(dir.join(format!("{tid}.{other}")));
    }
    let dst = dir.join(format!("{tid}.{ext}"));
    std::fs::write(&dst, &data).map_err(|e| PackageError(e.to_string()))?;
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_from_stems() {
        assert_eq!(
            guid_from_stem("00000008000400000000000000000001").as_deref(),
            Some("00000008-0004-0000-0000-000000000001")
        );
        assert_eq!(
            guid_from_stem("00000008-0004-C172-CAEB-F9C44D5308C9").as_deref(),
            Some("00000008-0004-c172-caeb-f9c44d5308c9")
        );
        assert!(guid_from_stem("asset_v2").is_none());
    }

    #[test]
    fn category_mask_names() {
        assert_eq!(category_names(0), "(none)");
        assert_eq!(category_names(0x8 | (1 << 22)), "Top|Animation");
        assert!(award_flag("00000008-0004-0100-0000-000000000000"));
        assert!(!award_flag("00000008-0004-0200-0000-000000000000"));
    }

    #[test]
    fn closet_item_validates() {
        let path = Path::new(r"C:\Users\edward\Documents\ReXGlue\ae-sub\assets\closet")
            .join("00000008-0004-c172-caeb-f9c44d5308c9.bin");
        if !path.is_file() {
            return;
        }
        let results = process_package(&path).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].guid, "00000008-0004-c172-caeb-f9c44d5308c9");
        assert_eq!(results[0].categories, 8);
        assert!(results[0].log.last().unwrap().starts_with("verdict: clean"));
    }
}
