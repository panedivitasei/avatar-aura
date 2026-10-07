// Port of the reading side of rex/kernel/xam/avatars/closet.{h,cpp}.
// Layout: <guid>.bin blobs, closet_index.tsv, icons/<guid>.png, closet_awards.tsv, closet_titles.tsv, titles/<TITLEID>.<ext>.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::asset_pack::AssetId;
use crate::error::Result;

const STOCK_GUID_TAIL: [u8; 8] = [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0];
const TITLE_ICON_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "gif"];
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosetItem {
    pub id: AssetId,
    pub categories: u32,
    pub bodies: u8,
    pub name: String,
    pub is_award: bool,
    /// Granting title, from `id.d[4..8]`.
    pub title_id: u32,
    pub title_name: String,
    pub description: String,
    /// FILETIME ticks; the blob's mtime when the awards sidecar has none.
    pub unlock_time: u64,
}

/// Parses the 8-4-4-4-12 text form; `None` on any malformed field.
pub fn parse_asset_id(text: &str) -> Option<AssetId> {
    text.parse().ok()
}

/// True when the id carries the stock asset pack's GUID tail.
pub fn is_stock_pack_id(id: &AssetId) -> bool {
    id.d == STOCK_GUID_TAIL
}

/// Provenance nibble `(c >> 8) & 0xF`: 1 = award, 2 = marketplace.
pub fn is_award_id(id: &AssetId) -> bool {
    (u32::from(id.c) >> 8) & 0xF == 1
}

pub fn title_id_of(id: &AssetId) -> u32 {
    u32::from_be_bytes([id.d[4], id.d[5], id.d[6], id.d[7]])
}

const C_SPACE: [char; 6] = [' ', '\t', '\n', '\r', '\x0b', '\x0c'];

/// Shared core of `strtoul`/`strtoull`: sign flag, magnitude, overflow flag.
fn parse_c_integer(text: &str, radix: u32) -> (bool, u64, bool) {
    let s = text.trim_start_matches(C_SPACE);
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let s = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(rest) if radix == 16 && rest.starts_with(|c: char| c.is_ascii_hexdigit()) => rest,
        _ => s,
    };
    let mut value: u64 = 0;
    let mut overflow = false;
    for c in s.chars() {
        let Some(d) = c.to_digit(radix) else {
            break;
        };
        match value
            .checked_mul(u64::from(radix))
            .and_then(|v| v.checked_add(u64::from(d)))
        {
            Some(v) => value = v,
            None => overflow = true,
        }
    }
    (negative, value, overflow)
}

/// C `strtoull`: saturates on overflow, negates on a leading minus.
fn strtoull(text: &str, radix: u32) -> u64 {
    match parse_c_integer(text, radix) {
        (_, _, true) => u64::MAX,
        (true, v, false) => v.wrapping_neg(),
        (false, v, false) => v,
    }
}

/// C `strtoul` with the 32-bit `unsigned long` of the Windows toolchain.
fn strtoul(text: &str, radix: u32) -> u32 {
    match parse_c_integer(text, radix) {
        (_, v, overflow) if overflow || v > u64::from(u32::MAX) => u32::MAX,
        (true, v, _) => (v as u32).wrapping_neg(),
        (false, v, _) => v as u32,
    }
}

/// Splits TSV text into rows of fields; tolerates a UTF-8 BOM and CRLF.
fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    lines(text)
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
}

fn read_text_file(path: &Path) -> String {
    std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// Whole file, or `None` when missing, unreadable or empty.
fn read_whole_file(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok().filter(|b| !b.is_empty())
}

fn file_mtime_filetime(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    let ticks = since.as_secs().checked_mul(10_000_000)? + u64::from(since.subsec_nanos() / 100);
    ticks.checked_add(FILETIME_UNIX_EPOCH)
}

#[derive(Clone, Debug, Default)]
pub struct Closet {
    is_loaded: bool,
    award_count: usize,
    dir: PathBuf,
    items: Vec<ClosetItem>,
    by_guid: HashMap<String, usize>,
}

impl Closet {
    /// Loads closet_index.tsv and the award sidecars; a missing index gives an empty, unloaded closet.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        let mut closet = Closet {
            dir,
            ..Default::default()
        };
        let bytes = match std::fs::read(closet.dir.join("closet_index.tsv")) {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(closet),
            Err(e) => return Err(e.into()),
        };
        let text = String::from_utf8_lossy(&bytes);
        closet.items = parse_index(&text);
        // Name order for the selection grids; the file itself is guid ordered.
        closet.items.sort_by(|a, b| a.name.cmp(&b.name));
        for (i, item) in closet.items.iter().enumerate() {
            closet.by_guid.insert(item.id.to_string(), i);
        }
        closet.is_loaded = true;
        closet.load_award_details();
        Ok(closet)
    }

    fn load_award_details(&mut self) {
        let mut title_names: HashMap<u32, String> = HashMap::new();
        for row in parse_tsv(&read_text_file(&self.dir.join("closet_titles.tsv"))) {
            if row.len() >= 2 {
                title_names.insert(strtoul(&row[0], 16), row[1].clone());
            }
        }

        struct AwardRow {
            title_name: String,
            description: String,
            unlock_time: u64,
        }
        let mut award_rows: HashMap<String, AwardRow> = HashMap::new();
        for row in parse_tsv(&read_text_file(&self.dir.join("closet_awards.tsv"))) {
            if row.is_empty() || row[0].len() != 36 {
                continue;
            }
            let r = AwardRow {
                title_name: row.get(2).cloned().unwrap_or_default(),
                description: row.get(3).cloned().unwrap_or_default(),
                unlock_time: row.get(4).map(|t| strtoull(t, 10)).unwrap_or(0),
            };
            award_rows.insert(row[0].to_ascii_lowercase(), r);
        }

        let dir = self.dir.clone();
        for item in self.items.iter_mut() {
            if !is_award_id(&item.id) {
                continue;
            }
            item.is_award = true;
            item.title_id = title_id_of(&item.id);
            let guid = item.id.to_string();
            if let Some(r) = award_rows.get(&guid) {
                item.title_name = r.title_name.clone();
                item.description = r.description.clone();
                item.unlock_time = r.unlock_time;
            }
            if item.title_name.is_empty() {
                if let Some(name) = title_names.get(&item.title_id) {
                    item.title_name = name.clone();
                }
            }
            if item.title_name.is_empty() {
                item.title_name = format!("{:08X}", item.title_id);
            }
            if item.unlock_time == 0 {
                if let Some(t) = file_mtime_filetime(&dir.join(format!("{guid}.bin"))) {
                    item.unlock_time = t;
                }
            }
            self.award_count += 1;
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.is_loaded
    }

    pub fn items(&self) -> &[ClosetItem] {
        &self.items
    }

    pub fn award_count(&self) -> usize {
        self.award_count
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stock pack ids never resolve here.
    pub fn find(&self, id: &AssetId) -> Option<&ClosetItem> {
        if self.items.is_empty() || is_stock_pack_id(id) {
            return None;
        }
        self.by_guid.get(&id.to_string()).map(|&i| &self.items[i])
    }

    pub fn has_item_bytes(&self, id: &AssetId) -> bool {
        !self.dir.as_os_str().is_empty() && self.dir.join(format!("{id}.bin")).exists()
    }

    /// The item's raw YTGR/STRB blob; an unindexed blob in a loaded closet still resolves.
    pub fn read_item_bytes(&self, id: &AssetId) -> Option<Vec<u8>> {
        let path = self.dir.join(format!("{id}.bin"));
        if self.find(id).is_none() && (!self.is_loaded || is_stock_pack_id(id)) {
            return None;
        }
        read_whole_file(&path)
    }

    /// The imported marketplace icon (icons/<guid>.png) as file bytes.
    pub fn read_item_icon(&self, id: &AssetId) -> Option<Vec<u8>> {
        self.find(id)?;
        read_whole_file(&self.dir.join("icons").join(format!("{id}.png")))
    }

    /// A game's tile art (titles/<TITLEID>.<ext>), falling back to titles/_default.<ext>.
    pub fn read_title_icon(&self, title_id: u32) -> Option<Vec<u8>> {
        if !self.is_loaded {
            return None;
        }
        let titles = self.dir.join("titles");
        TITLE_ICON_EXTS
            .iter()
            .find_map(|ext| read_whole_file(&titles.join(format!("{title_id:08X}.{ext}"))))
            .or_else(|| {
                TITLE_ICON_EXTS
                    .iter()
                    .find_map(|ext| read_whole_file(&titles.join(format!("_default.{ext}"))))
            })
    }
}

/// closet_index.tsv rows: guid TAB categories-hex TAB bodies TAB name; malformed rows are skipped.
pub fn parse_index(text: &str) -> Vec<ClosetItem> {
    let mut items = Vec::new();
    for line in lines(text) {
        let mut parts = line.splitn(4, '\t');
        let (Some(guid), Some(categories), Some(bodies), Some(name)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let Some(id) = parse_asset_id(guid) else {
            continue;
        };
        items.push(ClosetItem {
            id,
            categories: strtoul(categories, 16),
            bodies: strtoul(bodies, 10) as u8,
            name: name.to_owned(),
            ..Default::default()
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strtoul_matches_c() {
        assert_eq!(strtoul("00000ffc", 16), 0xffc);
        assert_eq!(strtoul("0x1F", 16), 0x1f);
        assert_eq!(strtoul("12abc", 10), 12);
        assert_eq!(strtoul("zz", 16), 0);
        assert_eq!(strtoul("1ffffffff", 16), u32::MAX);
        assert_eq!(strtoull("18446744073709551616", 10), u64::MAX);
    }

    #[test]
    fn index_rows() {
        let text = "00000ffc-bf47-52b1-cc91-58e258570b10\t00000ffc\t1\tNightmare\tOnesie\r\nbad\t1\t2\t3\n\n\
                    00000008-7c4c-7201-ceee-f418444d07d9\t8\t3\n";
        let items = parse_index(text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Nightmare\tOnesie");
        assert_eq!(items[0].categories, 0xffc);
        assert_eq!(items[0].bodies, 1);
    }

    #[test]
    fn id_classification() {
        let award: AssetId = "00000008-0004-c172-caeb-f9c44d5308c9".parse().unwrap();
        assert!(is_award_id(&award));
        assert_eq!(title_id_of(&award), 0x4d5308c9);
        let bought: AssetId = "00000008-7c4c-7201-ceee-f418444d07d9".parse().unwrap();
        assert!(!is_award_id(&bought));
        assert!(!is_stock_pack_id(&bought));
        let stock = AssetId {
            d: STOCK_GUID_TAIL,
            ..Default::default()
        };
        assert!(is_stock_pack_id(&stock));
    }
}
