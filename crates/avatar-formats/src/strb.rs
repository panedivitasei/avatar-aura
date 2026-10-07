// Port of xenia/kernel/xam/avatars/strb.{h,cpp}.

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockId {
    Invalid,
    Animation,
    Texture,
    Model,
    ShapeOverrides,
    Skeleton,
    AssetMetadataUnversioned,
    CustomColorTable,
    AssetMetadataVersioned,
    Other(u64),
}

impl BlockId {
    pub fn from_raw(raw: u64) -> Self {
        match raw {
            0 => Self::Invalid,
            1 => Self::Animation,
            2 => Self::Texture,
            3 => Self::Model,
            4 => Self::ShapeOverrides,
            5 => Self::Skeleton,
            6 => Self::AssetMetadataUnversioned,
            7 => Self::CustomColorTable,
            8 => Self::AssetMetadataVersioned,
            other => Self::Other(other),
        }
    }
}

/// One block: its id and `entry_count * entry_size` bytes of payload.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    pub id: BlockId,
    pub data: &'a [u8],
}

const SIGNATURE_SIZE: usize = 0x140;

fn read_value(buf: &[u8], offset: usize, size: usize, little_endian: bool) -> Result<u64> {
    let bytes = buf
        .get(
            offset
                ..offset
                    .checked_add(size)
                    .ok_or(Error::Malformed("STRB value offset"))?,
        )
        .ok_or(Error::Malformed("STRB value past end"))?;
    let mut v = 0u64;
    match size {
        1 | 2 | 4 | 8 => {
            if little_endian {
                for &b in bytes.iter().rev() {
                    v = (v << 8) | u64::from(b);
                }
            } else {
                for &b in bytes {
                    v = (v << 8) | u64::from(b);
                }
            }
            Ok(v)
        }
        _ => Err(Error::Malformed("STRB value size not 1/2/4/8")),
    }
}

/// rex::align: `(value + alignment - 1) & ~(alignment - 1)`.
fn align(value: usize, alignment: usize) -> Result<usize> {
    let sum = value
        .checked_add(alignment - 1)
        .ok_or(Error::Malformed("STRB size overflow"))?;
    Ok(sum & !(alignment - 1))
}

/// Visits blocks in order until `visit` returns true; a malformed block stops the walk with an error.
pub fn walk<'a>(buffer: &'a [u8], mut visit: impl FnMut(Block<'a>) -> bool) -> Result<bool> {
    let mut strb = buffer;
    if strb.len() >= 4 && &strb[..4] == b"YTGR" {
        if strb.len() < SIGNATURE_SIZE {
            return Err(Error::NotStrb);
        }
        strb = &strb[SIGNATURE_SIZE..];
    }
    if strb.len() < 24 || &strb[..4] != b"STRB" {
        return Err(Error::NotStrb);
    }

    let has_block_alignment = strb[4] != 0;
    let little_endian = strb[5] != 0;
    let block_id_size = strb[22] as usize;
    let block_size_size = strb[23] as usize;
    let block_alignment = if has_block_alignment {
        *strb.get(26).ok_or(Error::Malformed("STRB header truncated"))? as usize
    } else {
        1
    };
    if block_alignment == 0 {
        return Err(Error::Malformed("STRB block alignment of zero"));
    }

    let block_header_size = align(block_id_size + block_size_size + block_size_size, block_alignment)?;
    let block_start_offset = align(if has_block_alignment { 30 } else { 26 }, block_alignment)?;
    if block_header_size == 0 {
        return Err(Error::Malformed("STRB block header size of zero"));
    }

    let mut block_offset = block_start_offset;
    while block_offset
        .checked_add(block_header_size)
        .is_some_and(|end| end <= strb.len())
    {
        let id = read_value(strb, block_offset, block_id_size, little_endian)?;
        let data_size = read_value(strb, block_offset + block_id_size, block_size_size, little_endian)?;
        let entry_size = read_value(
            strb,
            block_offset + block_id_size + block_size_size,
            block_size_size,
            little_endian,
        )?;
        block_offset += block_header_size;

        let total = data_size
            .checked_mul(entry_size)
            .ok_or(Error::Malformed("STRB block size overflow"))?;
        let total = usize::try_from(total).map_err(|_| Error::Malformed("STRB block size overflow"))?;
        let data = block_offset
            .checked_add(total)
            .and_then(|end| strb.get(block_offset..end))
            .ok_or(Error::Malformed("STRB block extends past the container"))?;
        if visit(Block {
            id: BlockId::from_raw(id),
            data,
        }) {
            return Ok(true);
        }

        let data_size =
            usize::try_from(data_size).map_err(|_| Error::Malformed("STRB block size overflow"))?;
        block_offset = block_offset
            .checked_add(align(data_size, block_alignment)?)
            .ok_or(Error::Malformed("STRB block size overflow"))?;
    }
    Ok(false)
}

/// Every block in the container.
pub fn blocks(buffer: &[u8]) -> Result<Vec<Block<'_>>> {
    let mut out = Vec::new();
    walk(buffer, |b| {
        out.push(b);
        false
    })?;
    Ok(out)
}

/// The `occurrence`-th (0-based) block with the given id.
pub fn block_n(buffer: &[u8], id: BlockId, occurrence: usize) -> Result<&[u8]> {
    let mut seen = 0usize;
    let mut found = None;
    walk(buffer, |b| {
        if b.id != id {
            return false;
        }
        seen += 1;
        if seen - 1 != occurrence {
            return false;
        }
        found = Some(b.data);
        true
    })?;
    found.ok_or(Error::BlockNotFound(id))
}

pub fn block(buffer: &[u8], id: BlockId) -> Result<&[u8]> {
    block_n(buffer, id, 0)
}

/// Like `block`, but a missing block is `Ok(None)`.
pub fn find_block(buffer: &[u8], id: BlockId) -> Result<Option<&[u8]>> {
    match block(buffer, id) {
        Ok(data) => Ok(Some(data)),
        Err(Error::BlockNotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn count_blocks(buffer: &[u8], id: BlockId) -> Result<usize> {
    Ok(blocks(buffer)?.iter().filter(|b| b.id == id).count())
}
