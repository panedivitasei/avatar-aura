// Port of avatar-aura/native/avatarextract/texdecode.h (BC1/BC2/BC3 and A8R8G8B8 to RGBA8).
//! Block decoders for Xbox 360 avatar textures.
//!
//! [`decode`] takes little-endian, tightly packed data. [`decode_xenos_layer`] takes the raw
//! payload as stored in avatar assets: every 16-bit word byte-swapped, linear block order, rows at a
//! padded stride and frame layers `data_rows * data_stride` apart.

#![forbid(unsafe_code)]

/// Errors produced while decoding.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("texture has zero width or height ({width}x{height})")]
    EmptyTexture { width: u32, height: u32 },
    #[error("unsupported D3D texture format {0} (low 6 bits {low})", low = .0 & 0x3F)]
    UnknownFormat(u32),
    #[error("texture data is {have} bytes, {need} required")]
    DataTooShort { need: u64, have: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Supported texel formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bc1,
    Bc2,
    Bc3,
    A8R8G8B8,
}

impl Format {
    /// Classifies an avatar texture format word; the low 6 bits hold the Xbox D3D format.
    pub fn from_d3d(format: u32) -> Option<Self> {
        match format & 0x3F {
            18 => Some(Self::Bc1),
            19 => Some(Self::Bc2),
            20 => Some(Self::Bc3),
            6 => Some(Self::A8R8G8B8),
            _ => None,
        }
    }

    /// Bytes per 4x4 block, or per texel for A8R8G8B8.
    pub fn bytes_per_block(self) -> u32 {
        match self {
            Self::Bc1 => 8,
            Self::Bc2 | Self::Bc3 => 16,
            Self::A8R8G8B8 => 4,
        }
    }

    /// Texels per block edge.
    pub fn block_dim(self) -> u32 {
        match self {
            Self::A8R8G8B8 => 1,
            _ => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bc1 => "DXT1/BC1",
            Self::Bc2 => "DXT3/BC2",
            Self::Bc3 => "DXT5/BC3",
            Self::A8R8G8B8 => "A8R8G8B8",
        }
    }

    /// Blocks per row and block rows for an image.
    pub fn block_counts(self, width: u32, height: u32) -> (u32, u32) {
        let d = self.block_dim();
        (width.div_ceil(d), height.div_ceil(d))
    }
}

/// Decoded block: 16 RGBA8 texels in row-major order.
pub type Block = [[u8; 4]; 16];

fn expand565(c: u16) -> [u8; 3] {
    let r5 = ((c >> 11) & 0x1F) as u8;
    let g6 = ((c >> 5) & 0x3F) as u8;
    let b5 = (c & 0x1F) as u8;
    [
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    ]
}

/// Four-entry RGBA palette from a BC1 colour block; `allow_alpha` enables the 3-colour mode.
pub fn bc1_palette(block: &[u8; 8], allow_alpha: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let e0 = expand565(c0);
    let e1 = expand565(c1);
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [e0[0], e0[1], e0[2], 255];
    pal[1] = [e1[0], e1[1], e1[2], 255];
    if c0 > c1 || !allow_alpha {
        for i in 0..3 {
            let (a, b) = (u16::from(e0[i]), u16::from(e1[i]));
            pal[2][i] = ((2 * a + b) / 3) as u8;
            pal[3][i] = ((a + 2 * b) / 3) as u8;
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = ((u16::from(e0[i]) + u16::from(e1[i])) / 2) as u8;
            pal[3][i] = 0;
        }
        pal[2][3] = 255;
        pal[3][3] = 0;
    }
    pal
}

fn colour_indices(bits: &[u8]) -> u32 {
    u32::from_le_bytes([bits[0], bits[1], bits[2], bits[3]])
}

fn colour_block(block: &[u8; 8], allow_alpha: bool) -> Block {
    let pal = bc1_palette(block, allow_alpha);
    let bits = colour_indices(&block[4..8]);
    std::array::from_fn(|t| pal[((bits >> (2 * t)) & 3) as usize])
}

fn colour_half(block: &[u8; 16]) -> &[u8; 8] {
    block[8..16].try_into().expect("8-byte slice")
}

/// Decodes one BC1 block.
pub fn decode_bc1_block(block: &[u8; 8]) -> Block {
    colour_block(block, true)
}

/// Decodes one BC2 block: explicit 4-bit alpha scaled by 17, colour always in 4-colour mode.
pub fn decode_bc2_block(block: &[u8; 16]) -> Block {
    let mut out = colour_block(colour_half(block), false);
    for y in 0..4 {
        let arow = u16::from_le_bytes([block[y * 2], block[y * 2 + 1]]);
        for x in 0..4 {
            let a4 = ((arow >> (x * 4)) & 0xF) as u8;
            out[y * 4 + x][3] = a4 * 17;
        }
    }
    out
}

/// Eight-entry BC3 alpha palette.
pub fn bc3_alpha_palette(a0: u8, a1: u8) -> [u8; 8] {
    let (w0, w1) = (u32::from(a0), u32::from(a1));
    let mut apal = [0u8; 8];
    apal[0] = a0;
    apal[1] = a1;
    if a0 > a1 {
        for i in 1..7u32 {
            apal[i as usize + 1] = (((7 - i) * w0 + i * w1) / 7) as u8;
        }
    } else {
        for i in 1..5u32 {
            apal[i as usize + 1] = (((5 - i) * w0 + i * w1) / 5) as u8;
        }
        apal[6] = 0;
        apal[7] = 255;
    }
    apal
}

/// Decodes one BC3 block: interpolated alpha, colour always in 4-colour mode.
pub fn decode_bc3_block(block: &[u8; 16]) -> Block {
    let mut out = colour_block(colour_half(block), false);
    let apal = bc3_alpha_palette(block[0], block[1]);
    let abits = block[2..8]
        .iter()
        .enumerate()
        .fold(0u64, |acc, (i, &b)| acc | (u64::from(b) << (8 * i)));
    for (t, texel) in out.iter_mut().enumerate() {
        texel[3] = apal[((abits >> (3 * t)) & 7) as usize];
    }
    out
}

/// Converts one little-endian A8R8G8B8 texel (bytes B, G, R, A) to RGBA8.
pub fn decode_a8r8g8b8_texel(s: [u8; 4]) -> [u8; 4] {
    [s[2], s[1], s[0], s[3]]
}

/// Swaps the bytes of every 16-bit word in place; a trailing odd byte is left alone.
pub fn swap16(data: &mut [u8]) {
    for pair in data.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
}

fn check_dims(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(Error::EmptyTexture { width, height });
    }
    Ok(())
}

/// Decodes little-endian block rows at `pitch` bytes into tightly packed RGBA8.
/// Edge blocks are clipped to the image so non-multiple-of-4 sizes do not overrun.
fn decode_pitched(format: Format, width: u32, height: u32, pitch: usize, data: &[u8]) -> Result<Vec<u8>> {
    check_dims(width, height)?;
    let (wb, hb) = format.block_counts(width, height);
    let bpb = format.bytes_per_block() as usize;
    let row_bytes = wb as usize * bpb;
    let need = (hb as u64 - 1) * pitch as u64 + row_bytes as u64;
    if (data.len() as u64) < need {
        return Err(Error::DataTooShort {
            need,
            have: data.len(),
        });
    }
    let (w, h) = (width as usize, height as usize);
    let dst_pitch = w * 4;
    let mut out = vec![0u8; dst_pitch * h];
    if format == Format::A8R8G8B8 {
        for y in 0..h {
            let row = &data[y * pitch..y * pitch + row_bytes];
            for (x, s) in row.as_chunks::<4>().0.iter().enumerate() {
                let p = decode_a8r8g8b8_texel(*s);
                out[y * dst_pitch + x * 4..y * dst_pitch + x * 4 + 4].copy_from_slice(&p);
            }
        }
        return Ok(out);
    }
    for by in 0..hb as usize {
        for bx in 0..wb as usize {
            let start = by * pitch + bx * bpb;
            let src = &data[start..start + bpb];
            let texels = match format {
                Format::Bc1 => decode_bc1_block(src.try_into().expect("8-byte block")),
                Format::Bc2 => decode_bc2_block(src.try_into().expect("16-byte block")),
                Format::Bc3 => decode_bc3_block(src.try_into().expect("16-byte block")),
                Format::A8R8G8B8 => unreachable!("handled above"),
            };
            let cw = 4.min(w - bx * 4);
            let ch = 4.min(h - by * 4);
            for y in 0..ch {
                for x in 0..cw {
                    let o = (by * 4 + y) * dst_pitch + (bx * 4 + x) * 4;
                    out[o..o + 4].copy_from_slice(&texels[y * 4 + x]);
                }
            }
        }
    }
    Ok(out)
}

/// Decodes tightly packed little-endian data into RGBA8 rows, top row first.
pub fn decode(format: Format, width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    check_dims(width, height)?;
    let (wb, _) = format.block_counts(width, height);
    let pitch = wb as usize * format.bytes_per_block() as usize;
    decode_pitched(format, width, height, pitch, data)
}

/// Decodes tightly packed BC1 data.
pub fn decode_bc1(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    decode(Format::Bc1, width, height, data)
}

/// Decodes tightly packed BC2 data.
pub fn decode_bc2(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    decode(Format::Bc2, width, height, data)
}

/// Decodes tightly packed BC3 data.
pub fn decode_bc3(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    decode(Format::Bc3, width, height, data)
}

/// Decodes tightly packed little-endian A8R8G8B8 data.
pub fn decode_a8r8g8b8(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    decode(Format::A8R8G8B8, width, height, data)
}

/// Geometry of a raw avatar texture payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XenosLayout {
    /// D3D format word; only the low 6 bits are used.
    pub format: u32,
    pub width: u32,
    pub height: u32,
    /// Declared row pitch in bytes.
    pub data_stride: u32,
    /// Declared rows per layer.
    pub data_rows: u32,
}

/// Decodes one frame layer of a raw avatar texture payload (16-bit swapped, padded rows).
/// Falls back to a tight layout when the declared geometry overruns the payload, as the
/// extractor does.
pub fn decode_xenos_layer(layout: &XenosLayout, layer: u32, data: &[u8]) -> Result<Vec<u8>> {
    check_dims(layout.width, layout.height)?;
    let format = Format::from_d3d(layout.format).ok_or(Error::UnknownFormat(layout.format))?;
    let (wb, hb) = format.block_counts(layout.width, layout.height);
    let row_bytes = u64::from(wb) * u64::from(format.bytes_per_block());
    let hb = u64::from(hb);

    let mut pitch = u64::from(layout.data_stride).max(row_bytes);
    let layer_bytes = u64::from(layout.data_rows) * u64::from(layout.data_stride);
    let mut base = u64::from(layer) * layer_bytes;
    let mut span = (hb - 1) * pitch + row_bytes;
    if layer_bytes == 0 || base + span > data.len() as u64 {
        pitch = row_bytes;
        base = u64::from(layer) * pitch * hb;
        span = pitch * hb;
        if base + span > data.len() as u64 {
            return Err(Error::DataTooShort {
                need: base + span,
                have: data.len(),
            });
        }
    }
    // The swapped copy covers whole pitched rows when they fit, matching the extractor's buffer.
    let copy_end = (base + pitch * hb).min(data.len() as u64);
    let mut swapped = data[base as usize..copy_end as usize].to_vec();
    swap16(&mut swapped);
    decode_pitched(format, layout.width, layout.height, pitch as usize, &swapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bc1(c0: u16, c1: u16, idx: u32) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..2].copy_from_slice(&c0.to_le_bytes());
        b[2..4].copy_from_slice(&c1.to_le_bytes());
        b[4..].copy_from_slice(&idx.to_le_bytes());
        b
    }

    #[test]
    fn bc1_all_black() {
        let px = decode_bc1_block(&bc1(0, 0, 0));
        assert!(px.iter().all(|p| *p == [0, 0, 0, 255]));
        // c0 == c1 selects the 3-colour mode, so index 3 is transparent black.
        let px = decode_bc1_block(&bc1(0, 0, 0xFFFF_FFFF));
        assert!(px.iter().all(|p| *p == [0, 0, 0, 0]));
    }

    #[test]
    fn bc1_all_white() {
        let px = decode_bc1_block(&bc1(0xFFFF, 0xFFFF, 0));
        assert!(px.iter().all(|p| *p == [255; 4]));
    }

    #[test]
    fn bc1_two_colour_gradient() {
        // Row y uses palette index y: red, blue, 2/3 red, 1/3 red.
        let idx = (0x55 << 8) | (0xAA << 16) | (0xFF << 24);
        let px = decode_bc1_block(&bc1(0xF800, 0x001F, idx));
        let rows = [
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [170, 0, 85, 255],
            [85, 0, 170, 255],
        ];
        for (y, want) in rows.iter().enumerate() {
            for x in 0..4 {
                assert_eq!(px[y * 4 + x], *want, "texel {x},{y}");
            }
        }
        // 3-colour mode averages the endpoints with truncation.
        let px = decode_bc1_block(&bc1(0x001F, 0xF800, 0xAAAA_AAAA));
        assert!(px.iter().all(|p| *p == [127, 0, 127, 255]));
    }

    #[test]
    fn bc2_alpha_nibbles_and_forced_four_colour() {
        let mut b = [0u8; 16];
        b[0..2].copy_from_slice(&0x3210u16.to_le_bytes());
        b[8..].copy_from_slice(&bc1(0, 0, 0xFFFF_FFFF));
        let px = decode_bc2_block(&b);
        assert_eq!(px[0..4].iter().map(|p| p[3]).collect::<Vec<_>>(), [0, 17, 34, 51]);
        assert_eq!(px[0][..3], [0, 0, 0]);
        assert_eq!(px[4][3], 0);
    }

    #[test]
    fn bc3_alpha_ramp() {
        assert_eq!(bc3_alpha_palette(255, 0), [255, 0, 218, 182, 145, 109, 72, 36]);
        assert_eq!(bc3_alpha_palette(0, 255), [0, 255, 51, 102, 153, 204, 0, 255]);
        // Texel t takes alpha index t % 8: 3-bit indices 0..7 packed twice.
        let mut b = [0u8; 16];
        b[0] = 255;
        b[1] = 0;
        let mut bits = 0u64;
        for t in 0..16u64 {
            bits |= (t % 8) << (3 * t);
        }
        b[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
        b[8..].copy_from_slice(&bc1(0xFFFF, 0xFFFF, 0));
        let px = decode_bc3_block(&b);
        let pal = bc3_alpha_palette(255, 0);
        for (t, p) in px.iter().enumerate() {
            assert_eq!(*p, [255, 255, 255, pal[t % 8]]);
        }
    }

    #[test]
    fn a8r8g8b8_byte_order() {
        let out = decode_a8r8g8b8(1, 1, &[0x10, 0x20, 0x30, 0x40]).unwrap();
        assert_eq!(out, [0x30, 0x20, 0x10, 0x40]);
    }

    #[test]
    fn clips_partial_blocks() {
        let data = bc1(0xFFFF, 0xFFFF, 0);
        let out = decode_bc1(3, 2, &data).unwrap();
        assert_eq!(out.len(), 3 * 2 * 4);
        assert!(out.iter().all(|&b| b == 255));
    }

    #[test]
    fn xenos_layer_unswaps_and_honours_stride() {
        // 8x4 BC1 with a 32-byte stride: block 0 white, block 1 red, padding after.
        let mut le = vec![0u8; 32];
        le[..8].copy_from_slice(&bc1(0xFFFF, 0xFFFF, 0));
        le[8..16].copy_from_slice(&bc1(0xF800, 0xF800, 0));
        let mut raw = le.clone();
        swap16(&mut raw);
        let layout = XenosLayout {
            format: 0x80 | 18,
            width: 8,
            height: 4,
            data_stride: 32,
            data_rows: 1,
        };
        let out = decode_xenos_layer(&layout, 0, &raw).unwrap();
        assert_eq!(out, decode_bc1(8, 4, &le[..16]).unwrap());
        assert_eq!(&out[16..20], &[255, 0, 0, 255]);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            decode_bc1(0, 4, &[]),
            Err(Error::EmptyTexture { width: 0, height: 4 })
        );
        assert!(matches!(
            decode_bc3(8, 8, &[0; 16]),
            Err(Error::DataTooShort { need: 64, .. })
        ));
        let layout = XenosLayout {
            format: 7,
            width: 4,
            height: 4,
            data_stride: 8,
            data_rows: 1,
        };
        assert_eq!(
            decode_xenos_layer(&layout, 0, &[0; 8]),
            Err(Error::UnknownFormat(7))
        );
        let layout = XenosLayout { format: 18, ..layout };
        assert!(matches!(
            decode_xenos_layer(&layout, 3, &[0; 8]),
            Err(Error::DataTooShort { .. })
        ));
    }
}
