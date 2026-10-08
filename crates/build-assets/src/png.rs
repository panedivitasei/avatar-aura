// Port of stbi_write_png_to_mem from stb_image_write v1.16 at its defaults (compression level 8, filter by
// lowest absolute sum), so re-encoded face layers match the C++ avatarextract output byte for byte.

const ZHASH: usize = 16384;
const QUALITY: usize = 8;

const LENGTH_BASE: [u32; 30] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195,
    227, 258, 259,
];
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u32; 31] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073,
    4097, 6145, 8193, 12289, 16385, 24577, 32768,
];
const DIST_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

struct Bits {
    out: Vec<u8>,
    buf: u32,
    count: u32,
}

impl Bits {
    fn add(&mut self, code: u32, bits: u32) {
        self.buf |= code << self.count;
        self.count += bits;
        while self.count >= 8 {
            self.out.push(self.buf as u8);
            self.buf >>= 8;
            self.count -= 8;
        }
    }

    fn huffa(&mut self, code: u32, bits: u32) {
        self.add(bitrev(code, bits), bits);
    }

    /// Fixed Huffman code for a literal/length symbol.
    fn huff(&mut self, n: u32) {
        match n {
            0..=143 => self.huffa(0x30 + n, 8),
            144..=255 => self.huffa(0x190 + n - 144, 9),
            256..=279 => self.huffa(n - 256, 7),
            _ => self.huffa(0xc0 + n - 280, 8),
        }
    }
}

fn bitrev(mut code: u32, bits: u32) -> u32 {
    let mut res = 0;
    for _ in 0..bits {
        res = (res << 1) | (code & 1);
        code >>= 1;
    }
    res
}

fn count_match(data: &[u8], a: usize, b: usize, limit: usize) -> usize {
    let limit = limit.min(258);
    (0..limit).take_while(|&i| data[a + i] == data[b + i]).count()
}

fn zhash(d: &[u8]) -> usize {
    let mut h = u32::from(d[0]) + (u32::from(d[1]) << 8) + (u32::from(d[2]) << 16);
    h ^= h << 3;
    h = h.wrapping_add(h >> 5);
    h ^= h << 4;
    h = h.wrapping_add(h >> 17);
    h ^= h << 25;
    h = h.wrapping_add(h >> 6);
    h as usize & (ZHASH - 1)
}

/// stbi_zlib_compress: one fixed-Huffman block with stb's hash chains and one-step lazy matching.
fn zlib(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut bits = Bits {
        out: vec![0x78, 0x5e],
        buf: 0,
        count: 0,
    };
    bits.add(1, 1);
    bits.add(1, 2);
    let mut table: Vec<Vec<usize>> = vec![Vec::new(); ZHASH];
    let mut i = 0usize;
    while i + 3 < len {
        let h = zhash(&data[i..]);
        let mut best = 3;
        let mut best_loc = None;
        for &p in &table[h] {
            if p as isize > i as isize - 32768 {
                let d = count_match(data, p, i, len - i);
                if d >= best {
                    best = d;
                    best_loc = Some(p);
                }
            }
        }
        if table[h].len() == 2 * QUALITY {
            table[h].drain(..QUALITY);
        }
        table[h].push(i);
        if best_loc.is_some() {
            let h2 = zhash(&data[i + 1..]);
            for &p in &table[h2] {
                if p as isize > i as isize - 32767 && count_match(data, p, i + 1, len - i - 1) > best {
                    best_loc = None;
                    break;
                }
            }
        }
        if let Some(loc) = best_loc {
            let d = (i - loc) as u32;
            let best = best as u32;
            let mut j = 0;
            while best > LENGTH_BASE[j + 1] - 1 {
                j += 1;
            }
            bits.huff(j as u32 + 257);
            if LENGTH_EXTRA[j] != 0 {
                bits.add(best - LENGTH_BASE[j], LENGTH_EXTRA[j]);
            }
            let mut j = 0;
            while d > DIST_BASE[j + 1] - 1 {
                j += 1;
            }
            bits.add(bitrev(j as u32, 5), 5);
            if DIST_EXTRA[j] != 0 {
                bits.add(d - DIST_BASE[j], DIST_EXTRA[j]);
            }
            i += best as usize;
        } else {
            bits.huff(u32::from(data[i]));
            i += 1;
        }
    }
    while i < len {
        bits.huff(u32::from(data[i]));
        i += 1;
    }
    bits.huff(256);
    while bits.count != 0 {
        bits.add(0, 1);
    }
    let mut out = bits.out;
    if out.len() > len + 2 + len.div_ceil(32767) * 5 {
        out.truncate(2);
        let mut j = 0;
        while j < len {
            let block = (len - j).min(32767);
            out.push(u8::from(len - j == block));
            out.extend_from_slice(&(block as u16).to_le_bytes());
            out.extend_from_slice(&(!(block as u16)).to_le_bytes());
            out.extend_from_slice(&data[j..j + block]);
            j += block;
        }
    }
    let (mut s1, mut s2) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    out.extend_from_slice(&s2.to_be_bytes()[2..]);
    out.extend_from_slice(&s1.to_be_bytes()[2..]);
    out
}

fn paeth(a: i32, b: i32, c: i32) -> u8 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// stbiw__encode_png_line; row 0 maps up/avg/paeth onto their no-previous-row forms.
fn encode_line(px: &[u8], stride: usize, y: usize, n: usize, filter: usize, line: &mut [u8]) {
    let kind = if y == 0 { [0, 1, 0, 5, 6][filter] } else { filter };
    let z = &px[stride * y..stride * (y + 1)];
    let up = |i: usize| px[stride * (y - 1) + i];
    if kind == 0 {
        line.copy_from_slice(z);
        return;
    }
    for i in 0..n {
        line[i] = match kind {
            2 => z[i].wrapping_sub(up(i)),
            3 => z[i].wrapping_sub(up(i) >> 1),
            4 => z[i].wrapping_sub(paeth(0, i32::from(up(i)), 0)),
            _ => z[i],
        };
    }
    for i in n..z.len() {
        line[i] = match kind {
            1 => z[i].wrapping_sub(z[i - n]),
            2 => z[i].wrapping_sub(up(i)),
            3 => z[i].wrapping_sub(((u32::from(z[i - n]) + u32::from(up(i))) >> 1) as u8),
            4 => z[i].wrapping_sub(paeth(i32::from(z[i - n]), i32::from(up(i)), i32::from(up(i - n)))),
            5 => z[i].wrapping_sub(z[i - n] >> 1),
            _ => z[i].wrapping_sub(paeth(i32::from(z[i - n]), 0, 0)),
        };
    }
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// RGBA8 pixels to PNG bytes, as stbi_write_png with comp 4 writes them.
pub fn encode_rgba(px: &[u8], width: usize, height: usize) -> Vec<u8> {
    let n = 4;
    let stride = width * n;
    let mut filt = Vec::with_capacity((stride + 1) * height);
    let mut line = vec![0u8; stride];
    for y in 0..height {
        let mut best = 0;
        let mut best_val = i64::MAX;
        for f in 0..5 {
            encode_line(px, stride, y, n, f, &mut line);
            let est: i64 = line.iter().map(|&b| i64::from((b as i8).unsigned_abs())).sum();
            if est < best_val {
                best_val = est;
                best = f;
            }
        }
        if best != 4 {
            encode_line(px, stride, y, n, best, &mut line);
        }
        filt.push(best as u8);
        filt.extend_from_slice(&line);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib(&filt));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_decoder() {
        let (w, h) = (37, 11);
        let px: Vec<u8> = (0..w * h * 4).map(|i| ((i * 7) ^ (i >> 3)) as u8).collect();
        let png = encode_rgba(&px, w, h);
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.dimensions(), (w as u32, h as u32));
        assert_eq!(img.into_raw(), px);
    }
}
