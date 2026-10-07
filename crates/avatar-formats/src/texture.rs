// Port of xenia/kernel/xam/avatars/texture.{h,cpp}.

use crate::bits::BitStream;
use crate::compression;
use crate::error::{ensure, Result};
use crate::strb::{self, BlockId};

/// Texture header plus raw GPU payload (`data_stride * data_rows * layer_count` bytes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Texture {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub total_data_size: u32,
    pub data_size: u32,
    pub layer_count: u32,
    pub is_empty: bool,
    pub is_tiled: bool,
    pub data_stride: u32,
    pub data_rows: u32,
    pub data_bytes: Vec<u8>,
}

impl Texture {
    pub fn read(stream: &mut BitStream) -> Result<Self> {
        let mut t = Texture {
            format: stream.read_u32()?,
            width: stream.read_u32()?,
            height: stream.read_u32()?,
            total_data_size: stream.read_u32()?,
            data_size: stream.read_u32()?,
            layer_count: stream.read_u32()?,
            is_empty: stream.read_bool()?,
            is_tiled: stream.read_bool()?,
            data_stride: stream.read_u32()?,
            data_rows: stream.read_u32()?,
            data_bytes: Vec::new(),
        };
        stream.align_to_next_byte()?;

        ensure(t.width <= 1024, "texture width above 1024")?;
        ensure(t.height <= 1024, "texture height above 1024")?;
        ensure(
            (1..=14).contains(&t.layer_count),
            "texture layer count outside 1..=14",
        )?;

        if !t.is_empty {
            let size = t.data_stride as usize * t.data_rows as usize * t.layer_count as usize;
            t.data_bytes = stream.take_bytes(size)?;
        }
        Ok(t)
    }

    /// Parses an uncompressed texture payload that must be consumed exactly.
    pub fn read_data(data: &[u8]) -> Result<Self> {
        let mut stream = BitStream::new(data);
        let t = Self::read(&mut stream)?;
        ensure(
            stream.offset_bits() == stream.size_bits(),
            "trailing texture data",
        )?;
        Ok(t)
    }

    /// Decodes the first kTexture block of a YTGR/STRB container; `None` when it has none.
    pub fn load(strb_buffer: &[u8]) -> Result<Option<Self>> {
        let Some(compressed) = strb::find_block(strb_buffer, BlockId::Texture)? else {
            return Ok(None);
        };
        let data = compression::decompress(compressed)?;
        Self::read_data(&data).map(Some)
    }
}
