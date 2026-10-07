// Port of xenia/kernel/xam/avatars/compression.{h,cpp}.

use crate::error::{Error, Result};
use crate::lzx;

const BLOCK_HEADER_SIZE: usize = 12;
const WINDOW_SIZE: u32 = 0x8000;

struct BlockHeader {
    compressed_size: usize,
    uncompressed_offset: usize,
    uncompressed_size: usize,
}

fn block_header(input: &[u8], offset: usize) -> BlockHeader {
    let le = |o: usize| u32::from_le_bytes([input[o], input[o + 1], input[o + 2], input[o + 3]]) as usize;
    BlockHeader {
        compressed_size: le(offset),
        uncompressed_offset: le(offset + 4),
        uncompressed_size: le(offset + 8),
    }
}

/// Sum of the block headers' uncompressed sizes.
pub fn uncompressed_size(input: &[u8]) -> Result<usize> {
    if input.len() < BLOCK_HEADER_SIZE {
        return Err(Error::Malformed("compressed data shorter than a block header"));
    }
    let mut output_size = 0usize;
    let mut offset = 0usize;
    while offset + BLOCK_HEADER_SIZE <= input.len() {
        let header = block_header(input, offset);
        offset += BLOCK_HEADER_SIZE;
        if header.compressed_size > input.len() - offset {
            return Err(Error::Malformed("compressed block extends past the data"));
        }
        output_size = output_size
            .checked_add(header.uncompressed_size)
            .ok_or(Error::Malformed("uncompressed size overflow"))?;
        offset += header.compressed_size;
    }
    Ok(output_size)
}

/// Decompresses a sequence of headered LZX blocks into `output`.
pub fn decompress_into(input: &[u8], output: &mut [u8]) -> Result<()> {
    if input.len() < BLOCK_HEADER_SIZE {
        return Err(Error::Malformed("compressed data shorter than a block header"));
    }
    let mut input_offset = 0usize;
    let mut output_offset = 0usize;
    while input_offset + BLOCK_HEADER_SIZE <= input.len() {
        let header = block_header(input, input_offset);
        input_offset += BLOCK_HEADER_SIZE;
        if header.compressed_size > input.len() - input_offset
            || header.uncompressed_size > output.len() - output_offset
            || output_offset != header.uncompressed_offset
        {
            return Err(Error::Malformed("bad compressed block header"));
        }
        lzx::decompress(
            &input[input_offset..input_offset + header.compressed_size],
            &mut output[output_offset..output_offset + header.uncompressed_size],
            WINDOW_SIZE,
        )?;
        input_offset += header.compressed_size;
        output_offset += header.uncompressed_size;
    }
    Ok(())
}

/// Sizes the output from the block headers and decompresses every block.
pub fn decompress(input: &[u8]) -> Result<Vec<u8>> {
    let size = uncompressed_size(input)?;
    let mut output = vec![0u8; size];
    decompress_into(input, &mut output)?;
    Ok(output)
}
