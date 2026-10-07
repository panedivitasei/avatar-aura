// Port of avatar-aura/native/avatarextract/main.cpp (self-contained STFS reader), cross-checked against
// ae-sub/sdk/src/filesystem/devices/stfs_container_device.cpp and stfs_xbox.h.
//! Read-only reader for Xbox 360 STFS packages ("CON ", "LIVE", "PIRS").
//!
//! Header fields are big-endian except the 24-bit block numbers and the file table block count,
//! which the format stores little-endian.

#![forbid(unsafe_code)]

/// STFS block size in bytes.
pub const BLOCK_SIZE: u32 = 0x1000;
/// Next-block marker that terminates a level-0 hash chain.
pub const END_OF_CHAIN: u32 = 0xFF_FFFF;
/// Blocks covered by one hash table at levels 0, 1 and 2.
pub const BLOCKS_PER_HASH_LEVEL: [u32; 3] = [170, 28_900, 4_913_000];

/// Size of `XContentHeader` plus `XContentMetadata`.
pub const HEADER_STRUCT_SIZE: usize = 0x344 + 0x93D6;

const OFF_HEADER_SIZE: usize = 0x340;
const OFF_CONTENT_TYPE: usize = 0x344;
const OFF_METADATA_VERSION: usize = 0x348;
const OFF_TITLE_ID: usize = 0x360;
const OFF_DESCRIPTOR: usize = 0x379;
const OFF_VOLUME_TYPE: usize = 0x3A9;
const OFF_DISPLAY_NAME: usize = 0x411;
const OFF_DESCRIPTION: usize = 0xD11;
const OFF_PUBLISHER: usize = 0x1611;
const OFF_TITLE_NAME: usize = 0x1691;

const HASH_ENTRY_SIZE: u64 = 0x18;
const DIR_ENTRY_SIZE: usize = 0x40;
const NAME_FIELD_LEN: usize = 40;

/// Errors produced while parsing or reading a package.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("package is {0} bytes, smaller than the STFS header")]
    TooSmall(usize),
    #[error("bad package magic {0:02X?}")]
    BadMagic([u8; 4]),
    #[error("volume type {0} is not STFS (SVOD packages are not supported)")]
    NotStfs(u32),
    #[error("{what} at offset {offset:#X} lies outside the {len}-byte package")]
    OutOfBounds {
        what: &'static str,
        offset: u64,
        len: usize,
    },
    #[error("directory entry {index} names parent {parent}, which is not an earlier directory")]
    BadParent { index: usize, parent: u16 },
    #[error("entry '{0}' is a directory")]
    IsDirectory(String),
    #[error("block chain of '{name}' ended with {missing} of {size} bytes unread")]
    TruncatedChain { name: String, missing: u32, size: u32 },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Package signature type from the first four bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Con,
    Live,
    Pirs,
}

impl PackageKind {
    /// Classifies a magic value, or `None` for anything else.
    pub fn from_magic(magic: &[u8]) -> Option<Self> {
        match magic.get(..4)? {
            b"CON " => Some(Self::Con),
            b"LIVE" => Some(Self::Live),
            b"PIRS" => Some(Self::Pirs),
            _ => None,
        }
    }
}

/// `StfsVolumeDescriptor` fields used for block addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeDescriptor {
    pub descriptor_length: u8,
    pub version: u8,
    /// One backing block per hash table instead of two.
    pub read_only_format: bool,
    /// Top-level hash table lives in the secondary backing block.
    pub root_active_index: bool,
    pub file_table_block_count: u16,
    pub file_table_block_number: u32,
    pub total_block_count: u32,
    pub free_block_count: u32,
}

/// One file table record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Position in [`Package::entries`].
    pub index: usize,
    pub name: String,
    pub size: u32,
    pub is_directory: bool,
    /// Index of the parent directory entry, `None` for the package root.
    pub parent: Option<usize>,
    pub start_block: u32,
    pub contiguous: bool,
    pub allocated_blocks: u32,
}

/// Block and hash-table addressing for one package layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Header size rounded up to a whole block.
    pub data_base: u64,
    /// 1 for read-only packages, 2 for read-write ones.
    pub blocks_per_hash_table: u32,
}

impl Layout {
    pub fn new(header_size: u32, read_only_format: bool) -> Self {
        let bs = u64::from(BLOCK_SIZE);
        Self {
            data_base: u64::from(header_size).div_ceil(bs) * bs,
            blocks_per_hash_table: if read_only_format { 1 } else { 2 },
        }
    }

    /// Physical block number of a data block after skipping the interleaved hash tables.
    pub fn block_to_physical(&self, block_index: u32) -> u64 {
        let index = u64::from(block_index);
        let per = u64::from(self.blocks_per_hash_table);
        let mut base = u64::from(BLOCKS_PER_HASH_LEVEL[0]);
        let mut block = index;
        for _ in 0..3 {
            block += (index + base) / base * per;
            if index < base {
                break;
            }
            base *= u64::from(BLOCKS_PER_HASH_LEVEL[0]);
        }
        block
    }

    /// Byte offset of a data block.
    pub fn block_to_offset(&self, block_index: u32) -> u64 {
        self.data_base + (self.block_to_physical(block_index) << 12)
    }

    /// Physical block number of the hash table covering `block_index` at `hash_level`.
    pub fn hash_block_number(&self, block_index: u32, hash_level: u32) -> u64 {
        let per = u64::from(self.blocks_per_hash_table);
        let l0 = u64::from(BLOCKS_PER_HASH_LEVEL[0]);
        let l1 = u64::from(BLOCKS_PER_HASH_LEVEL[1]);
        let step0 = l0 + per;
        let step1 = l1 + (l0 + 1) * per;
        let index = u64::from(block_index);
        match hash_level {
            0 => {
                if index < l0 {
                    return 0;
                }
                let block = index / l0 * step0 + (index / l1 + 1) * per;
                if index < l1 {
                    block
                } else {
                    block + per
                }
            }
            1 => {
                if index < l1 {
                    step0
                } else {
                    index / l1 * step1 + per
                }
            }
            _ => step1,
        }
    }

    /// Byte offset of the primary copy of a hash table.
    pub fn hash_block_offset(&self, block_index: u32, hash_level: u32) -> u64 {
        self.data_base + (self.hash_block_number(block_index, hash_level) << 12)
    }
}

/// A parsed package borrowing the raw bytes.
#[derive(Debug, Clone)]
pub struct Package<'a> {
    data: &'a [u8],
    pub kind: PackageKind,
    pub header_size: u32,
    pub content_type: u32,
    pub metadata_version: u32,
    pub title_id: u32,
    /// English display name.
    pub display_name: String,
    /// English description.
    pub description: String,
    pub publisher: String,
    pub title_name: String,
    pub descriptor: VolumeDescriptor,
    pub layout: Layout,
    entries: Vec<Entry>,
}

fn be_u32(d: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
}

fn be_u16(d: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([d[off], d[off + 1]])
}

fn le_u24(d: &[u8], off: usize) -> u32 {
    u32::from(d[off]) | (u32::from(d[off + 1]) << 8) | (u32::from(d[off + 2]) << 16)
}

/// NUL-terminated UTF-16BE string of at most `max_chars` units.
fn utf16_be(d: &[u8], off: usize, max_chars: usize) -> String {
    let units: Vec<u16> = (0..max_chars)
        .map(|i| be_u16(d, off + i * 2))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

impl<'a> Package<'a> {
    /// Parses the header and walks the file table.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < HEADER_STRUCT_SIZE {
            return Err(Error::TooSmall(data.len()));
        }
        let kind =
            PackageKind::from_magic(data).ok_or(Error::BadMagic([data[0], data[1], data[2], data[3]]))?;
        let volume_type = be_u32(data, OFF_VOLUME_TYPE);
        if volume_type != 0 {
            return Err(Error::NotStfs(volume_type));
        }
        let d = OFF_DESCRIPTOR;
        let flags = data[d + 2];
        let descriptor = VolumeDescriptor {
            descriptor_length: data[d],
            version: data[d + 1],
            read_only_format: flags & 1 != 0,
            root_active_index: flags & 2 != 0,
            file_table_block_count: u16::from_le_bytes([data[d + 3], data[d + 4]]),
            file_table_block_number: le_u24(data, d + 5),
            total_block_count: be_u32(data, d + 0x1C),
            free_block_count: be_u32(data, d + 0x20),
        };
        let header_size = be_u32(data, OFF_HEADER_SIZE);
        let mut pkg = Package {
            data,
            kind,
            header_size,
            content_type: be_u32(data, OFF_CONTENT_TYPE),
            metadata_version: be_u32(data, OFF_METADATA_VERSION),
            title_id: be_u32(data, OFF_TITLE_ID),
            display_name: utf16_be(data, OFF_DISPLAY_NAME, 128),
            description: utf16_be(data, OFF_DESCRIPTION, 128),
            publisher: utf16_be(data, OFF_PUBLISHER, 64),
            title_name: utf16_be(data, OFF_TITLE_NAME, 64),
            descriptor,
            layout: Layout::new(header_size, descriptor.read_only_format),
            entries: Vec::new(),
        };
        pkg.entries = pkg.walk_file_table()?;
        Ok(pkg)
    }

    /// The raw package bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    /// File table records in on-disk order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Slash-joined path of an entry from the package root.
    pub fn path(&self, entry: &Entry) -> String {
        let mut parts = vec![entry.name.as_str()];
        let mut parent = entry.parent;
        // Parents always precede children, so the walk terminates.
        while let Some(p) = parent.and_then(|i| self.entries.get(i)) {
            parts.push(p.name.as_str());
            parent = p.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// Looks up an entry by slash-joined path, ignoring ASCII case.
    pub fn find(&self, path: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| self.path(e).eq_ignore_ascii_case(path))
    }

    /// Reads a file's bytes by following its level-0 hash chain.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        if entry.is_directory {
            return Err(Error::IsDirectory(entry.name.clone()));
        }
        // Capacity is bounded by the package size so a forged length cannot over-allocate.
        let mut out = Vec::with_capacity((entry.size as usize).min(self.data.len()));
        let mut block = entry.start_block;
        let mut remaining = entry.size;
        while remaining > 0 && block != END_OF_CHAIN {
            let off = self.layout.block_to_offset(block);
            let chunk = remaining.min(BLOCK_SIZE);
            out.extend_from_slice(self.slice(off, chunk as usize, "data block")?);
            remaining -= chunk;
            block = self.next_block(block)?;
        }
        if remaining != 0 {
            return Err(Error::TruncatedChain {
                name: entry.name.clone(),
                missing: remaining,
                size: entry.size,
            });
        }
        Ok(out)
    }

    /// Next block in the level-0 chain after `block_index`.
    pub fn next_block(&self, block_index: u32) -> Result<u32> {
        Ok(self.hash_info(block_index)? & 0xFF_FFFF)
    }

    fn slice(&self, offset: u64, len: usize, what: &'static str) -> Result<&'a [u8]> {
        let oob = Error::OutOfBounds {
            what,
            offset,
            len: self.data.len(),
        };
        let start = usize::try_from(offset).map_err(|_| oob.clone())?;
        let end = start.checked_add(len).ok_or_else(|| oob.clone())?;
        self.data.get(start..end).ok_or(oob)
    }

    fn hash_entry_info(&self, table_offset: u64, record: u32) -> Result<u32> {
        let off = table_offset + u64::from(record) * HASH_ENTRY_SIZE + 0x14;
        let b = self.slice(off, 4, "hash entry")?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Level-0 hash entry info word, following the active-index flags of read-write packages.
    fn hash_info(&self, block_index: u32) -> Result<u32> {
        let bs = u64::from(BLOCK_SIZE);
        let d = &self.descriptor;
        let active = |info: u32| if info & 0x4000_0000 != 0 { bs } else { 0 };
        let mut secondary = if d.root_active_index { bs } else { 0 };
        if d.read_only_format {
            secondary = 0;
        } else if d.total_block_count > BLOCKS_PER_HASH_LEVEL[0] {
            if d.total_block_count > BLOCKS_PER_HASH_LEVEL[1] {
                let lv2 = self.layout.hash_block_offset(block_index, 2);
                let record = (block_index / BLOCKS_PER_HASH_LEVEL[1]) % BLOCKS_PER_HASH_LEVEL[0];
                secondary = active(self.hash_entry_info(lv2 + secondary, record)?);
            }
            let lv1 = self.layout.hash_block_offset(block_index, 1);
            let record = (block_index / BLOCKS_PER_HASH_LEVEL[0]) % BLOCKS_PER_HASH_LEVEL[0];
            secondary = active(self.hash_entry_info(lv1 + secondary, record)?);
        }
        let lv0 = self.layout.hash_block_offset(block_index, 0);
        self.hash_entry_info(lv0 + secondary, block_index % BLOCKS_PER_HASH_LEVEL[0])
    }

    fn walk_file_table(&self) -> Result<Vec<Entry>> {
        let mut entries: Vec<Entry> = Vec::new();
        let mut table_block = self.descriptor.file_table_block_number;
        for _ in 0..self.descriptor.file_table_block_count {
            if table_block == END_OF_CHAIN {
                break;
            }
            let off = self.layout.block_to_offset(table_block);
            let block = self.slice(off, BLOCK_SIZE as usize, "file table block")?;
            for raw in block.as_chunks::<DIR_ENTRY_SIZE>().0 {
                if raw[0] == 0 {
                    break;
                }
                let flags = raw[0x28];
                let name_len = usize::from(flags & 0x3F).min(NAME_FIELD_LEN);
                let index = entries.len();
                let parent_raw = be_u16(raw, 0x32);
                let parent = if parent_raw == 0xFFFF {
                    None
                } else {
                    let p = usize::from(parent_raw);
                    if p >= index || !entries[p].is_directory {
                        return Err(Error::BadParent {
                            index,
                            parent: parent_raw,
                        });
                    }
                    Some(p)
                };
                entries.push(Entry {
                    index,
                    name: String::from_utf8_lossy(&raw[..name_len]).into_owned(),
                    size: be_u32(raw, 0x34),
                    is_directory: flags & 0x80 != 0,
                    parent,
                    start_block: le_u24(raw, 0x2F),
                    contiguous: flags & 0x40 != 0,
                    allocated_blocks: le_u24(raw, 0x2C),
                });
            }
            table_block = self.next_block(table_block)?;
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_block_offsets_read_only() {
        let l = Layout::new(0x971A, true);
        assert_eq!(l.data_base, 0xA000);
        assert_eq!(l.block_to_physical(0), 1);
        assert_eq!(l.block_to_physical(169), 170);
        // Level-1 table at 171 and the second level-0 table at 172.
        assert_eq!(l.block_to_physical(170), 173);
        assert_eq!(l.block_to_physical(339), 342);
        assert_eq!(l.block_to_physical(340), 344);
        assert_eq!(l.block_to_offset(0), 0xB000);
    }

    #[test]
    fn data_block_offsets_read_write() {
        let l = Layout::new(0xA000, false);
        assert_eq!(l.block_to_physical(0), 2);
        assert_eq!(l.block_to_physical(169), 171);
        // Two level-1 copies plus two level-0 copies precede block 170.
        assert_eq!(l.block_to_physical(170), 176);
    }

    #[test]
    fn hash_block_numbers_match_block_steps() {
        let ro = Layout::new(0, true);
        assert_eq!(ro.hash_block_number(0, 0), 0);
        assert_eq!(ro.hash_block_number(169, 0), 0);
        assert_eq!(ro.hash_block_number(170, 0), 172);
        assert_eq!(ro.hash_block_number(0, 1), 0xAB);
        assert_eq!(ro.hash_block_number(0, 2), 0x718F);
        let rw = Layout::new(0, false);
        assert_eq!(rw.hash_block_number(170, 0), 174);
        assert_eq!(rw.hash_block_number(0, 1), 0xAC);
        assert_eq!(rw.hash_block_number(0, 2), 0x723A);
    }

    #[test]
    fn hash_tables_precede_their_data_blocks() {
        for read_only in [true, false] {
            let l = Layout::new(0, read_only);
            let per = u64::from(l.blocks_per_hash_table);
            for b in [0u32, 1, 169, 170, 171, 339, 340, 28_899, 28_900, 28_901, 60_000] {
                let phys = l.block_to_physical(b);
                let h0 = l.hash_block_number(b, 0);
                assert!(h0 + per <= phys, "block {b}: hash {h0} overlaps data {phys}");
                assert_ne!(l.hash_block_number(b, 1), phys);
            }
        }
    }

    /// Builds a package with a directory and one file spread over consecutive blocks.
    fn synth(file: &[u8], read_only: bool) -> Vec<u8> {
        let layout = Layout::new(HEADER_STRUCT_SIZE as u32, read_only);
        let blocks = file.len().div_ceil(BLOCK_SIZE as usize) as u32;
        let total = blocks + 1;
        let end = layout.block_to_offset(total - 1) + u64::from(BLOCK_SIZE);
        let mut d = vec![0u8; end as usize];
        d[..4].copy_from_slice(b"LIVE");
        d[OFF_HEADER_SIZE..OFF_HEADER_SIZE + 4].copy_from_slice(&(HEADER_STRUCT_SIZE as u32).to_be_bytes());
        d[OFF_TITLE_ID..OFF_TITLE_ID + 4].copy_from_slice(&0xFFFE_07D1u32.to_be_bytes());
        for (i, c) in "Hat".encode_utf16().enumerate() {
            let o = OFF_DISPLAY_NAME + i * 2;
            d[o..o + 2].copy_from_slice(&c.to_be_bytes());
        }
        let ds = OFF_DESCRIPTOR;
        d[ds] = 0x24;
        d[ds + 2] = u8::from(read_only);
        d[ds + 3] = 1;
        d[ds + 0x1C..ds + 0x20].copy_from_slice(&total.to_be_bytes());
        // Table block 0 ends its chain; file blocks 1..=blocks link forward.
        let h0 = layout.hash_block_offset(0, 0) as usize;
        let set = |d: &mut Vec<u8>, b: u32, next: u32| {
            let o = h0 + b as usize * 0x18 + 0x14;
            d[o..o + 4].copy_from_slice(&(0x8000_0000 | next).to_be_bytes());
        };
        set(&mut d, 0, END_OF_CHAIN);
        for b in 1..=blocks {
            set(&mut d, b, if b == blocks { END_OF_CHAIN } else { b + 1 });
        }
        let ft = layout.block_to_offset(0) as usize;
        d[ft..ft + 3].copy_from_slice(b"dir");
        d[ft + 0x28] = 0x80 | 3;
        d[ft + 0x32..ft + 0x34].copy_from_slice(&0xFFFFu16.to_be_bytes());
        let e = ft + 0x40;
        d[e..e + 9].copy_from_slice(b"asset.bin");
        d[e + 0x28] = 9;
        d[e + 0x2C] = blocks as u8;
        d[e + 0x2F] = 1;
        d[e + 0x34..e + 0x38].copy_from_slice(&(file.len() as u32).to_be_bytes());
        for (b, src) in file.chunks(BLOCK_SIZE as usize).enumerate() {
            let o = layout.block_to_offset(b as u32 + 1) as usize;
            d[o..o + src.len()].copy_from_slice(src);
        }
        d
    }

    #[test]
    fn synthetic_package_round_trip() {
        let file: Vec<u8> = (0..9000u32).map(|i| (i * 7 + 3) as u8).collect();
        for read_only in [true, false] {
            let bytes = synth(&file, read_only);
            let pkg = Package::parse(&bytes).unwrap();
            assert_eq!(pkg.kind, PackageKind::Live);
            assert_eq!(pkg.title_id, 0xFFFE_07D1);
            assert_eq!(pkg.display_name, "Hat");
            assert_eq!(pkg.entries().len(), 2);
            let f = &pkg.entries()[1];
            assert_eq!(pkg.path(f), "dir/asset.bin");
            assert_eq!(pkg.find("DIR/ASSET.BIN").map(|e| e.index), Some(1));
            assert_eq!(pkg.read(f).unwrap(), file);
            assert!(matches!(pkg.read(&pkg.entries()[0]), Err(Error::IsDirectory(_))));
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Package::parse(&[0; 16]).unwrap_err(), Error::TooSmall(16));
        let mut bytes = synth(&[1, 2, 3], true);
        bytes[..4].copy_from_slice(b"XEX2");
        assert!(matches!(Package::parse(&bytes), Err(Error::BadMagic(_))));
        let mut bytes = synth(&[1, 2, 3], true);
        bytes.truncate(bytes.len() - 0x1000);
        let pkg = Package::parse(&bytes).unwrap();
        assert!(matches!(
            pkg.read(&pkg.entries()[1]),
            Err(Error::OutOfBounds { .. })
        ));
        let mut bytes = synth(&[1, 2, 3], true);
        let ft = Layout::new(HEADER_STRUCT_SIZE as u32, true).block_to_offset(0) as usize;
        bytes[ft + 0x40 + 0x32..ft + 0x40 + 0x34].copy_from_slice(&5u16.to_be_bytes());
        assert!(matches!(Package::parse(&bytes), Err(Error::BadParent { .. })));
    }
}
