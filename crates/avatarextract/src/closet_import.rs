// Port of modes 6 (--closet-import), 6b (--closet-icons) and 8 (--scan-closet) of avatarextract/main.cpp,
// with DetectItemBodies, the raw-archive walk and the closet index reader they share.
//! Closet import, icon backfill and closet validation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use avatar_formats::bits::BitStream;
use avatar_formats::strb::BlockId;

/// The editor's XamAvatarGetAssetIcon buffer is 0x10000 bytes.
pub const MAX_ICON_BYTES: u64 = 0x10000;
/// Claimed uncompressed sizes above this are rejected (the runtime allocates that much).
const MAX_UNCOMPRESSED: usize = 64 << 20;

// ---------------------------------------------------------------------------
// Lenient STRB walk
// ---------------------------------------------------------------------------

/// One block as the C++ walker sees it: the claimed payload may extend past the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawBlock {
    pub id: u64,
    /// Payload offset into the buffer handed to [`walk_blocks`].
    pub offset: usize,
    /// Claimed payload size (`data_size * entry_size`).
    pub size: usize,
}

impl RawBlock {
    /// The payload, or `None` when it reaches past the buffer (a fault in the C++).
    pub fn data<'a>(&self, buf: &'a [u8]) -> Option<&'a [u8]> {
        buf.get(self.offset..self.offset.checked_add(self.size)?)
    }
}

/// Raw block id of an STRB block kind.
pub fn raw_block_id(id: BlockId) -> u64 {
    match id {
        BlockId::Invalid => 0,
        BlockId::Animation => 1,
        BlockId::Texture => 2,
        BlockId::Model => 3,
        BlockId::ShapeOverrides => 4,
        BlockId::Skeleton => 5,
        BlockId::AssetMetadataUnversioned => 6,
        BlockId::CustomColorTable => 7,
        BlockId::AssetMetadataVersioned => 8,
        BlockId::Other(v) => v,
    }
}

fn strb_value(buf: &[u8], off: usize, size: usize, le: bool) -> u64 {
    let read = |n: usize| -> u64 {
        let mut v = 0u64;
        for i in 0..n {
            let b = u64::from(buf.get(off + i).copied().unwrap_or(0));
            let k = if le { i } else { n - 1 - i };
            v |= b << (8 * k);
        }
        v
    };
    match size {
        1 | 2 | 4 | 8 => read(size),
        // GetSTRBValue's default case.
        _ => 8,
    }
}

fn align_up(v: usize, a: usize) -> Option<usize> {
    Some(v.checked_add(a - 1)? / a * a)
}

/// Walks STRB blocks the way strb.cpp's WalkSTRBBlocks does: only the headers are bounds checked.
/// Offsets are relative to `buf`, after the YTGR signature when present. Returns `Err` for a zero
/// alignment, which hangs the C++ walker.
pub fn walk_blocks<'a>(
    buf: &'a [u8],
    mut visit: impl FnMut(RawBlock, &'a [u8]) -> bool,
) -> Result<bool, &'static str> {
    let mut strb = buf;
    if strb.len() >= 4 && &strb[..4] == b"YTGR" {
        if strb.len() < 0x140 {
            return Ok(false);
        }
        strb = &strb[0x140..];
    }
    if strb.len() < 24 || &strb[..4] != b"STRB" {
        return Ok(false);
    }
    let has_align = strb[4] != 0;
    let le = strb[5] != 0;
    let id_size = usize::from(strb[22]);
    let size_size = usize::from(strb[23]);
    let alignment = if has_align {
        usize::from(strb.get(26).copied().ok_or("STRB header truncated")?)
    } else {
        1
    };
    if alignment == 0 {
        return Err("STRB block alignment of zero");
    }
    let header = align_up(id_size + 2 * size_size, alignment).ok_or("STRB overflow")?;
    let mut off = align_up(if has_align { 30 } else { 26 }, alignment).ok_or("STRB overflow")?;
    while off.checked_add(header).is_some_and(|e| e <= strb.len()) {
        let id = strb_value(strb, off, id_size, le);
        let data_size = strb_value(strb, off + id_size, size_size, le);
        let entry_size = strb_value(strb, off + id_size + size_size, size_size, le);
        off += header;
        let size = usize::try_from(data_size.wrapping_mul(entry_size)).unwrap_or(usize::MAX);
        if visit(
            RawBlock {
                id,
                offset: off,
                size,
            },
            strb,
        ) {
            return Ok(true);
        }
        let step = usize::try_from(data_size)
            .ok()
            .and_then(|d| align_up(d, alignment));
        match step.and_then(|s| off.checked_add(s)) {
            Some(next) => off = next,
            None => break,
        }
    }
    Ok(false)
}

/// The `occurrence`-th block with `id` and the buffer its offset is relative to.
pub fn find_block_n(
    buf: &[u8],
    id: BlockId,
    occurrence: usize,
) -> Result<Option<(RawBlock, &[u8])>, &'static str> {
    let want = raw_block_id(id);
    let mut seen = 0usize;
    let mut found = None;
    walk_blocks(buf, |b, strb| {
        if b.id != want {
            return false;
        }
        seen += 1;
        if seen - 1 != occurrence {
            return false;
        }
        found = Some((b, strb));
        true
    })?;
    Ok(found)
}

/// CountSTRBBlocks: blocks with `id` seen before the walk ends.
pub fn count_blocks(buf: &[u8], id: BlockId) -> usize {
    let want = raw_block_id(id);
    let mut count = 0usize;
    let _ = walk_blocks(buf, |b, _| {
        if b.id == want {
            count += 1;
        }
        false
    });
    count
}

// ---------------------------------------------------------------------------
// DetectItemBodies
// ---------------------------------------------------------------------------

/// Validation result before the fault guard folds errors into 0.
fn detect_item_bodies_impl(bytes: &[u8]) -> Result<u32, String> {
    // Chunk-compressed blocks: the claimed uncompressed size must be plausible and the first chunk must fit.
    for bid in [BlockId::Texture, BlockId::Model] {
        let Some((blk, strb)) = find_block_n(bytes, bid, 0)? else {
            continue;
        };
        if blk.size < 12 {
            return Ok(0);
        }
        let head = strb
            .get(blk.offset..blk.offset + 4)
            .ok_or("block header past the end of the item")?;
        let first_chunk = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        if u64::from(first_chunk) + 12 > blk.size as u64 {
            return Ok(0);
        }
        let data = blk.data(strb).ok_or("block payload past the end of the item")?;
        // A false GetUncompressedSize means a truncated or corrupt block that faults in lzxd at load.
        match avatar_formats::compression::uncompressed_size(data) {
            Ok(unc) if unc <= MAX_UNCOMPRESSED => {}
            _ => return Ok(0),
        }
    }
    // Gender from the raw shape headers: count, buffer size, target id {a u32, b u16, c u16, d 8B}.
    // Only a BODY-targeting shape (a == 2) says anything; .c is 1 male / 2 female.
    for occurrence in 0.. {
        let Some((blk, strb)) = find_block_n(bytes, BlockId::ShapeOverrides, occurrence)? else {
            break;
        };
        if blk.size < 24 {
            continue;
        }
        let head = strb
            .get(blk.offset..blk.offset + 16)
            .ok_or("shape header past the end of the item")?;
        let mut s = BitStream::new(head);
        let index_count = s.read_u32().map_err(|e| e.to_string())?;
        if index_count > 8192 {
            // The runtime's parser trusts this count as an allocation size.
            return Ok(0);
        }
        let _total_buffer_size = s.read_u32().map_err(|e| e.to_string())?;
        let a = s.read_u32().map_err(|e| e.to_string())?;
        let _b = s.read_u16().map_err(|e| e.to_string())?;
        let c = s.read_u16().map_err(|e| e.to_string())?;
        if a != 2 {
            continue;
        }
        match c {
            1 => return Ok(1),
            2 => return Ok(2),
            _ => {}
        }
    }
    Ok(3)
}

/// 1 male, 2 female, 3 unisex or no body shape, 0 malformed. A read past the item (an SEH fault in
/// the C++) is logged and counts as malformed.
pub fn detect_item_bodies(bytes: &[u8], log: &mut dyn Write) -> u32 {
    match detect_item_bodies_impl(bytes) {
        Ok(v) => v,
        Err(e) => {
            say!(log, "  (item parse fault, {e})\n");
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Raw archive walk
// ---------------------------------------------------------------------------

/// One item found in a raw marketplace archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveItem {
    pub bin: PathBuf,
    pub item_dir: PathBuf,
    /// Lowercase dashed guid; empty for a blob without a product id.
    pub guid: String,
    /// Empty when there is no usable art.
    pub icon: PathBuf,
    /// Why `icon` is empty.
    pub icon_note: String,
}

/// 8-4-4-4-12 hex with dashes.
pub fn is_dashed_guid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn file_name_str(p: &Path) -> String {
    p.file_name().map(|n| lossy(Path::new(n))).unwrap_or_default()
}

fn stem_str(p: &Path) -> String {
    p.file_stem().map(|n| lossy(Path::new(n))).unwrap_or_default()
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e == ext)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn read_text(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_default()
}

/// Layout (a): the guid from ID<suffix>.TXT, else (single variant) a 36-char <guid>.xml stem.
fn legacy_item_guid(item_dir: &Path, suffix: &str) -> String {
    let text = read_text(&item_dir.join(format!("ID{suffix}.TXT")));
    let eol = text
        .iter()
        .position(|&c| c == b'\r' || c == b'\n')
        .unwrap_or(text.len());
    let guid = String::from_utf8_lossy(&text[..eol]).to_ascii_lowercase();
    if is_dashed_guid(&guid) {
        return guid;
    }
    if !suffix.is_empty() {
        return String::new();
    }
    let Ok(rd) = fs::read_dir(item_dir) else {
        return String::new();
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() || !has_ext(&p, "xml") {
            continue;
        }
        let stem = stem_str(&p).to_ascii_lowercase();
        if is_dashed_guid(&stem) {
            return stem;
        }
    }
    String::new()
}

/// Walks a raw archive and calls `f` per item: avataritems_raw folders (asset_v2[_N].bin, ID[_N].TXT or <guid>.xml,
/// ICON[_N].PNG) and Avatar_Items dumps (<guid>.bin with thumbm/thumbsm[_N].png paired by mtime order).
pub fn walk_archive(dir: &Path, f: &mut dyn FnMut(&ArchiveItem)) {
    let mut subdirs = Vec::new();
    let mut legacy_bins = Vec::new();
    let mut guid_bins: Vec<(PathBuf, Option<SystemTime>)> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                subdirs.push(p);
                continue;
            }
            if !p.is_file() || !has_ext(&p, "bin") {
                continue;
            }
            if file_name_str(&p).starts_with("asset_v2") {
                legacy_bins.push(p);
            } else if is_dashed_guid(&stem_str(&p).to_ascii_lowercase()) {
                let t = mtime(&p);
                guid_bins.push((p, t));
            }
        }
    }
    for bin in legacy_bins {
        let fname = file_name_str(&bin);
        // "" or "_1", ... between "asset_v2" and ".bin".
        let suffix = fname
            .get(8..fname.len().saturating_sub(4))
            .unwrap_or("")
            .to_string();
        let mut item = ArchiveItem {
            guid: legacy_item_guid(dir, &suffix),
            bin,
            item_dir: dir.to_path_buf(),
            ..Default::default()
        };
        // ICON<suffix>.PNG pairs with the variant; a folder-level ICON.PNG is shared.
        let mut icon = dir.join(format!("ICON{suffix}.PNG"));
        if suffix.is_empty() || !icon.exists() {
            icon = dir.join("ICON.PNG");
        }
        if icon.exists() {
            item.icon = icon;
        } else {
            item.icon_note = "no ICON.PNG".to_string();
        }
        f(&item);
    }
    guid_bins.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    for i in 0..guid_bins.len() {
        let (bin, bin_time) = &guid_bins[i];
        let suffix = if i == 0 {
            String::new()
        } else {
            format!("_{}", i + 1)
        };
        let mut item = ArchiveItem {
            bin: bin.clone(),
            item_dir: dir.to_path_buf(),
            guid: stem_str(bin).to_ascii_lowercase(),
            icon_note: format!("no thumbm{suffix}.png / thumbsm{suffix}.png"),
            ..Default::default()
        };
        for base in ["thumbm", "thumbsm"] {
            let cand = dir.join(format!("{base}{suffix}.png"));
            if !cand.exists() {
                continue;
            }
            let art_time = mtime(&cand);
            let next_time = guid_bins.get(i + 1).map(|n| n.1);
            if art_time.is_none() || art_time < *bin_time || next_time.is_some_and(|n| art_time > n) {
                item.icon_note = format!(
                    "{} was not written together with this bin (pairing ambiguous), skipped",
                    file_name_str(&cand)
                );
                break;
            }
            let size = fs::metadata(&cand).map_or(u64::MAX, |m| m.len());
            if size > MAX_ICON_BYTES {
                item.icon_note = format!("{} over 64KB (editor buffer), skipped", file_name_str(&cand));
                continue;
            }
            item.icon = cand;
            item.icon_note.clear();
            break;
        }
        f(&item);
    }
    for sd in subdirs {
        walk_archive(&sd, f);
    }
}

// ---------------------------------------------------------------------------
// closet_index.tsv
// ---------------------------------------------------------------------------

/// closet_index.tsv as guid -> rest of line (raw bytes), tolerating CRLF and a UTF-8 BOM.
pub fn read_closet_index(closet_dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut lines = BTreeMap::new();
    let text = read_text(&closet_dir.join("closet_index.tsv"));
    let body = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&text);
    for raw in body.split(|&c| c == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let Some(tab) = line.iter().position(|&c| c == b'\t') else {
            continue;
        };
        if tab != 36 {
            continue;
        }
        let guid = String::from_utf8_lossy(&line[..36]);
        if is_dashed_guid(&guid) {
            lines.insert(guid.to_ascii_lowercase(), line[37..].to_vec());
        }
    }
    lines
}

/// Category mask from the guid's first dword.
pub fn guid_categories(guid: &str) -> u32 {
    guid.get(..8)
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Mode 6: closet import
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ImportCounts {
    scanned: usize,
    imported: usize,
    dupes: usize,
    failed: usize,
    poisoned: usize,
    icons: usize,
    no_icon: usize,
    unpaired: usize,
}

struct ImportState<'a> {
    closet_dir: &'a Path,
    icon_cat_mask: u32,
    index_lines: BTreeMap<String, Vec<u8>>,
    attempted: BTreeSet<String>,
    tsv: File,
    journal: File,
    n: ImportCounts,
}

impl ImportState<'_> {
    fn import(&mut self, item: &ArchiveItem, log: &mut dyn Write) -> anyhow::Result<()> {
        if item.guid.is_empty() {
            self.n.unpaired += 1;
            self.n.failed += 1;
            return Ok(());
        }
        if self.index_lines.contains_key(&item.guid) {
            self.n.dupes += 1;
            return Ok(());
        }
        if self.attempted.contains(&item.guid) {
            self.n.poisoned += 1;
            return Ok(());
        }
        // Journal the attempt before parsing so a crash poisons the item for the next run.
        writeln!(self.journal, "{}", item.guid)?;
        self.journal.flush()?;
        let bytes = match fs::read(&item.bin) {
            Ok(b) if !b.is_empty() => b,
            _ => {
                self.n.failed += 1;
                return Ok(());
            }
        };
        let bodies = detect_item_bodies(&bytes, log);
        if bodies == 0 {
            say!(log, "  skipping malformed item: {}\n", item.item_dir.display());
            self.n.failed += 1;
            return Ok(());
        }
        let categories = guid_categories(&item.guid);
        let name: String = file_name_str(&item.item_dir)
            .chars()
            .map(|c| if matches!(c, '\t' | '\n' | '\r') { ' ' } else { c })
            .collect();
        // Always overwrite: a multi-variant folder can pair a bin with the wrong guid, and a re-import corrects it.
        if fs::write(self.closet_dir.join(format!("{}.bin", item.guid)), &bytes).is_err() {
            self.n.failed += 1;
            return Ok(());
        }
        if categories & self.icon_cat_mask != 0 {
            if item.icon.as_os_str().is_empty() {
                self.n.no_icon += 1;
            } else {
                let icons = self.closet_dir.join("icons");
                let _ = fs::create_dir_all(&icons);
                match fs::copy(&item.icon, icons.join(format!("{}.png", item.guid))) {
                    Ok(_) => self.n.icons += 1,
                    Err(_) => self.n.no_icon += 1,
                }
            }
        }
        let meta = format!("{categories:08X}\t{bodies}\t");
        let mut rest = meta.clone().into_bytes();
        rest.extend_from_slice(name.as_bytes());
        self.index_lines.insert(item.guid.clone(), rest);
        writeln!(self.tsv, "{}\t{}{}", item.guid, meta, name)?;
        self.tsv.flush()?;
        self.n.imported += 1;
        if self.n.imported.is_multiple_of(1000) {
            say!(log, "  ... {} imported\n", self.n.imported);
            let _ = log.flush();
        }
        Ok(())
    }
}

fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Mode 6: dedupes a raw marketplace archive by guid into `<closet_dir>/<guid>.bin` and closet_index.tsv
/// (guid TAB categories-hex TAB bodies TAB name), copying icons for categories in `icon_cat_mask`.
pub fn run_closet_import(raw_root: &Path, closet_dir: &Path, icon_cat_mask: u32, log: &mut dyn Write) -> i32 {
    let _ = fs::create_dir_all(closet_dir);
    let index_lines = read_closet_index(closet_dir);
    // Resume: a guid in the attempt journal that never made the index crashed the process last time.
    let mut attempted = BTreeSet::new();
    let journal_text = read_text(&closet_dir.join("closet_attempted.log"));
    for raw in journal_text.split(|&c| c == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let guid = String::from_utf8_lossy(line).into_owned();
        if line.len() == 36 && !index_lines.contains_key(&guid) {
            attempted.insert(guid);
        }
    }
    if !index_lines.is_empty() || !attempted.is_empty() {
        say!(
            log,
            "  resuming: {} indexed, {} poison-skipped\n",
            index_lines.len(),
            attempted.len()
        );
    }
    let index_path = closet_dir.join("closet_index.tsv");
    if index_path.exists() {
        // The walk appends and the end rewrites: keep the pre-run index.
        let _ = fs::copy(&index_path, closet_dir.join("closet_index.tsv.bak"));
    }
    let (Ok(mut tsv), Ok(journal)) = (
        open_append(&index_path),
        open_append(&closet_dir.join("closet_attempted.log")),
    ) else {
        say!(log, "ERROR: cannot open index/journal for append.\n");
        return 1;
    };
    let existing = read_text(&index_path);
    if existing.last().is_some_and(|&c| c != b'\n') {
        let _ = tsv.write_all(b"\n");
    }
    let mut st = ImportState {
        closet_dir,
        icon_cat_mask,
        index_lines,
        attempted,
        tsv,
        journal,
        n: ImportCounts::default(),
    };
    walk_archive(raw_root, &mut |item| {
        st.n.scanned += 1;
        if let Err(e) = st.import(item, log) {
            say!(log, "  item threw ({e}), skipping\n");
            let _ = log.flush();
            st.n.failed += 1;
        }
    });
    let ImportState { index_lines, n, .. } = st;
    // Rewrite the index sorted, deduped, without BOM, LF only.
    let mut out = Vec::new();
    for (guid, rest) in &index_lines {
        out.extend_from_slice(guid.as_bytes());
        out.push(b'\t');
        out.extend_from_slice(rest);
        out.push(b'\n');
    }
    if fs::write(&index_path, out).is_err() {
        say!(log, "ERROR: cannot write {}\n", index_path.display());
        return 1;
    }
    say!(
        log,
        "closet import: scanned={} imported={} (icons={}, no-icon={}) dupes={} failed={} (unpaired={}) poisoned={} -> {}\n",
        n.scanned,
        n.imported,
        n.icons,
        n.no_icon,
        n.dupes,
        n.failed,
        n.unpaired,
        n.poisoned,
        closet_dir.display()
    );
    0
}

// ---------------------------------------------------------------------------
// Mode 6b: closet icon backfill
// ---------------------------------------------------------------------------

/// icons_generated.tsv lines (newline kept), as fgets returns them.
fn generated_lines(path: &Path) -> Vec<Vec<u8>> {
    let text = read_text(path);
    text.split_inclusive(|&c| c == b'\n')
        .map(<[u8]>::to_vec)
        .collect()
}

fn generated_guid(line: &[u8]) -> Option<String> {
    (line.len() > 36 && line[36] == b'\t').then(|| String::from_utf8_lossy(&line[..36]).into_owned())
}

/// Mode 6b: copies archive art to `<closet_dir>/icons/<guid>.png` for items already in the index.
/// Existing icons are kept, except --gen-icons stand-ins (icons_generated.tsv), which real art replaces.
pub fn run_closet_icons(raw_root: &Path, closet_dir: &Path, icon_cat_mask: u32, log: &mut dyn Write) -> i32 {
    let indexed = read_closet_index(closet_dir);
    if indexed.is_empty() {
        say!(
            log,
            "ERROR: no closet_index.tsv (or empty) at {}; run --closet-import first.\n",
            closet_dir.display()
        );
        return 1;
    }
    let icons_dir = closet_dir.join("icons");
    let _ = fs::create_dir_all(&icons_dir);
    let (mut scanned, mut copied, mut present, mut no_icon) = (0usize, 0usize, 0usize, 0usize);
    let (mut unknown, mut filtered, mut noted) = (0usize, 0usize, 0usize);
    let gen_tsv = closet_dir.join("icons_generated.tsv");
    let generated: BTreeSet<String> = generated_lines(&gen_tsv)
        .iter()
        .filter_map(|l| generated_guid(l))
        .collect();
    let mut replaced_guids = BTreeSet::new();
    walk_archive(raw_root, &mut |item| {
        scanned += 1;
        if item.guid.is_empty() || !indexed.contains_key(&item.guid) {
            unknown += 1;
            return;
        }
        if guid_categories(&item.guid) & icon_cat_mask == 0 {
            filtered += 1;
            return;
        }
        let dst = icons_dir.join(format!("{}.png", item.guid));
        let has_icon = !item.icon.as_os_str().is_empty();
        let stand_in = generated.contains(&item.guid);
        if dst.exists() && !(stand_in && has_icon) {
            present += 1;
            return;
        }
        if stand_in && has_icon {
            replaced_guids.insert(item.guid.clone());
        }
        if !has_icon {
            no_icon += 1;
            // Plain "no art" is the common case; only pairing and size refusals get a line.
            if !item.icon_note.starts_with("no ") {
                if noted < 200 {
                    say!(
                        log,
                        "  {}  {}: {}\n",
                        item.guid,
                        file_name_str(&item.item_dir),
                        item.icon_note
                    );
                }
                noted += 1;
            }
            return;
        }
        // fs::copy overwrites, so a stand-in is replaced; elsewhere dst does not exist here.
        match fs::copy(&item.icon, &dst) {
            Ok(_) => copied += 1,
            Err(_) => no_icon += 1,
        }
        if copied != 0 && copied.is_multiple_of(1000) {
            say!(log, "  ... {copied} icons copied\n");
            let _ = log.flush();
        }
    });
    if !replaced_guids.is_empty() {
        let mut replaced = 0usize;
        let mut keep = Vec::new();
        for line in generated_lines(&gen_tsv) {
            if generated_guid(&line).is_some_and(|g| replaced_guids.contains(&g)) {
                replaced += 1;
                continue;
            }
            keep.extend_from_slice(&line);
        }
        let _ = fs::write(&gen_tsv, keep);
        say!(log, "  generated stand-ins replaced by store art: {replaced}\n");
    }
    say!(
        log,
        "closet icons: scanned={scanned} copied={copied} already-present={present} no-icon={no_icon} \
         not-in-index={unknown} category-filtered={filtered} -> {}\n",
        icons_dir.display()
    );
    0
}

// ---------------------------------------------------------------------------
// Mode 8: closet scan
// ---------------------------------------------------------------------------

/// Mode 8: runs every `<closet_dir>/*.bin` through [`detect_item_bodies`] and lists the bad ones.
/// Returns 2 when any item is bad.
pub fn run_closet_scan(closet_dir: &Path, log: &mut dyn Write) -> i32 {
    let (mut scanned, mut bad) = (0usize, 0usize);
    if let Ok(rd) = fs::read_dir(closet_dir) {
        for entry in rd {
            let Ok(entry) = entry else { break };
            let p = entry.path();
            if !p.is_file() || !has_ext(&p, "bin") {
                continue;
            }
            scanned += 1;
            let Ok(bytes) = fs::read(&p) else { continue };
            if bytes.is_empty() || detect_item_bodies(&bytes, log) == 0 {
                say!(log, "BAD {}\n", file_name_str(&p));
                let _ = log.flush();
                bad += 1;
            }
            if scanned.is_multiple_of(5000) {
                say!(log, "  ... {scanned} scanned\n");
                let _ = log.flush();
            }
        }
    }
    say!(log, "closet scan: scanned={scanned} bad={bad}\n");
    if bad != 0 {
        2
    } else {
        0
    }
}
