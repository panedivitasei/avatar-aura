// Port of libmspack's lzxd.c (+ readbits.h, readhuff.h), (C) 2003-2023 Stuart Caie, LGPL 2.1.
// Reduced to the configuration xenia's lzx_decompress uses: regular LZX, no reset interval, no delta.

use crate::error::{Error, Result};

const MIN_MATCH: usize = 2;
const NUM_CHARS: usize = 256;
const BLOCKTYPE_INVALID: u32 = 0;
const BLOCKTYPE_VERBATIM: u32 = 1;
const BLOCKTYPE_ALIGNED: u32 = 2;
const BLOCKTYPE_UNCOMPRESSED: u32 = 3;
const NUM_PRIMARY_LENGTHS: usize = 7;
const NUM_SECONDARY_LENGTHS: usize = 249;
const FRAME_SIZE: usize = 32768;
const LENTABLE_SAFETY: usize = 64;
const HUFF_MAXBITS: u32 = 16;

const PRETREE_MAXSYMBOLS: usize = 20;
const PRETREE_TABLEBITS: u32 = 6;
const MAINTREE_MAXSYMBOLS: usize = NUM_CHARS + 290 * 8;
const MAINTREE_TABLEBITS: u32 = 12;
const LENGTH_MAXSYMBOLS: usize = NUM_SECONDARY_LENGTHS + 1;
const LENGTH_TABLEBITS: u32 = 12;
const ALIGNED_MAXSYMBOLS: usize = 8;
const ALIGNED_TABLEBITS: u32 = 7;

const POSITION_SLOTS: [u32; 11] = [30, 32, 34, 36, 38, 42, 50, 66, 98, 162, 290];
const EXTRA_BITS: [u8; 36] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14,
    15, 15, 16, 16,
];

const POSITION_BASE: [u32; 290] = build_position_base();

const fn build_position_base() -> [u32; 290] {
    let mut table = [0u32; 290];
    let mut i = 1;
    while i < 290 {
        let prev = i - 1;
        let extra = if prev < 4 {
            0
        } else if prev < 36 {
            (prev / 2) as u32 - 1
        } else {
            17
        };
        table[i] = table[prev] + (1 << extra);
        i += 1;
    }
    table
}

fn position_base(slot: usize) -> u32 {
    POSITION_BASE[slot]
}

struct Huffman {
    len: Vec<u8>,
    table: Vec<u16>,
    nsyms: usize,
    nbits: u32,
    empty: bool,
}

impl Huffman {
    fn new(nsyms: usize, nbits: u32) -> Self {
        Self {
            len: vec![0; nsyms + LENTABLE_SAFETY],
            table: vec![0; (1 << nbits) + nsyms * 2],
            nsyms,
            nbits,
            empty: false,
        }
    }

    /// make_decode_table, MSB bit order; false when the code lengths do not form a full tree.
    #[allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]
    fn build(&mut self) -> bool {
        let nsyms = self.nsyms;
        let nbits = self.nbits;
        let length = &self.len;
        let table = &mut self.table;
        let mut pos: u32 = 0;
        let mut table_mask: u32 = 1 << nbits;
        let mut bit_mask: u32 = table_mask >> 1;

        for bit_num in 1..=nbits {
            for sym in 0..nsyms {
                if u32::from(length[sym]) != bit_num {
                    continue;
                }
                let mut leaf = pos;
                pos += bit_mask;
                if pos > table_mask {
                    return false;
                }
                for _ in 0..bit_mask {
                    table[leaf as usize] = sym as u16;
                    leaf += 1;
                }
            }
            bit_mask >>= 1;
        }

        if pos == table_mask {
            return true;
        }

        for sym in pos..table_mask {
            table[sym as usize] = 0xFFFF;
        }

        let mut next_symbol: u32 = if ((table_mask >> 1) as usize) < nsyms {
            nsyms as u32
        } else {
            table_mask >> 1
        };

        pos <<= 16;
        table_mask <<= 16;
        bit_mask = 1 << 15;

        for bit_num in (nbits + 1)..=HUFF_MAXBITS {
            for sym in 0..nsyms {
                if u32::from(length[sym]) != bit_num {
                    continue;
                }
                if pos >= table_mask {
                    return false;
                }
                let mut leaf = pos >> 16;
                for fill in 0..(bit_num - nbits) {
                    let Some(&entry) = table.get(leaf as usize) else {
                        return false;
                    };
                    if entry == 0xFFFF {
                        let a = (next_symbol << 1) as usize;
                        if a + 1 >= table.len() {
                            return false;
                        }
                        table[a] = 0xFFFF;
                        table[a + 1] = 0xFFFF;
                        table[leaf as usize] = next_symbol as u16;
                        next_symbol += 1;
                    }
                    leaf = u32::from(table[leaf as usize]) << 1;
                    if (pos >> (15 - fill)) & 1 != 0 {
                        leaf += 1;
                    }
                }
                let Some(slot) = table.get_mut(leaf as usize) else {
                    return false;
                };
                *slot = sym as u16;
                pos += bit_mask;
            }
            bit_mask >>= 1;
        }

        pos == table_mask
    }

    fn build_maybe_empty(&mut self) -> Result<()> {
        self.empty = false;
        if !self.build() {
            if self.len[..self.nsyms].iter().any(|&l| l > 0) {
                return Err(Error::Lzx("bad huffman table"));
            }
            self.empty = true;
        }
        Ok(())
    }
}

struct Bits<'a> {
    input: &'a [u8],
    pos: usize,
    bit_buffer: u32,
    bits_left: i32,
}

impl<'a> Bits<'a> {
    fn next_byte(&mut self) -> Result<u8> {
        if self.pos < self.input.len() {
            let b = self.input[self.pos];
            self.pos += 1;
            return Ok(b);
        }
        // read_input fakes two zero bytes once the source is exhausted.
        if self.pos - self.input.len() >= 2 {
            return Err(Error::Lzx("out of input bytes"));
        }
        self.pos += 1;
        Ok(0)
    }

    fn read_bytes(&mut self) -> Result<()> {
        let b0 = self.next_byte()?;
        let b1 = self.next_byte()?;
        let data = (u32::from(b1) << 8) | u32::from(b0);
        self.bit_buffer |= data << (32 - 16 - self.bits_left) as u32;
        self.bits_left += 16;
        Ok(())
    }

    fn ensure(&mut self, nbits: i32) -> Result<()> {
        while self.bits_left < nbits {
            self.read_bytes()?;
        }
        Ok(())
    }

    fn peek(&self, nbits: u32) -> u32 {
        if nbits == 0 {
            0
        } else {
            self.bit_buffer >> (32 - nbits)
        }
    }

    fn remove(&mut self, nbits: u32) {
        self.bit_buffer = if nbits >= 32 { 0 } else { self.bit_buffer << nbits };
        self.bits_left -= nbits as i32;
    }

    fn read(&mut self, nbits: u32) -> Result<u32> {
        self.ensure(nbits as i32)?;
        let v = self.peek(nbits);
        self.remove(nbits);
        Ok(v)
    }

    fn read_huffsym(&mut self, h: &Huffman) -> Result<usize> {
        self.ensure(HUFF_MAXBITS as i32)?;
        let mut sym = h.table[self.peek(h.nbits) as usize];
        if sym as usize >= h.nsyms {
            let mut idx: u32 = 1 << (32 - h.nbits);
            loop {
                idx >>= 1;
                if idx == 0 {
                    return Err(Error::Lzx("huffman traverse overrun"));
                }
                let bit = u32::from(self.bit_buffer & idx != 0);
                let i = ((u32::from(sym) << 1) | bit) as usize;
                sym = *h.table.get(i).ok_or(Error::Lzx("huffman traverse overrun"))?;
                if (sym as usize) < h.nsyms {
                    break;
                }
            }
        }
        let len = u32::from(h.len[sym as usize]);
        self.remove(len);
        Ok(sym as usize)
    }

    /// Drops the partial byte buffer and returns the next raw input byte.
    fn raw_byte(&mut self) -> Result<u8> {
        self.next_byte()
    }
}

struct Decoder<'a> {
    bits: Bits<'a>,
    window: Vec<u8>,
    window_size: u32,
    window_posn: u32,
    frame_posn: u32,
    frame: u32,
    offset: usize,
    length: usize,
    r0: u32,
    r1: u32,
    r2: u32,
    header_read: bool,
    block_remaining: u32,
    block_length: u32,
    block_type: u32,
    intel_filesize: i32,
    intel_started: bool,
    num_offsets: usize,
    pretree: Huffman,
    maintree: Huffman,
    length_tree: Huffman,
    aligned: Huffman,
}

impl<'a> Decoder<'a> {
    fn new(input: &'a [u8], window_bits: u32, output_length: usize) -> Result<Self> {
        if !(15..=21).contains(&window_bits) {
            return Err(Error::Lzx("unsupported window size"));
        }
        let window_size = 1u32 << window_bits;
        Ok(Self {
            bits: Bits {
                input,
                pos: 0,
                bit_buffer: 0,
                bits_left: 0,
            },
            window: vec![0; window_size as usize],
            window_size,
            window_posn: 0,
            frame_posn: 0,
            frame: 0,
            offset: 0,
            length: output_length,
            r0: 1,
            r1: 1,
            r2: 1,
            header_read: false,
            block_remaining: 0,
            block_length: 0,
            block_type: BLOCKTYPE_INVALID,
            intel_filesize: 0,
            intel_started: false,
            num_offsets: (POSITION_SLOTS[(window_bits - 15) as usize] as usize) << 3,
            pretree: Huffman::new(PRETREE_MAXSYMBOLS, PRETREE_TABLEBITS),
            maintree: Huffman::new(MAINTREE_MAXSYMBOLS, MAINTREE_TABLEBITS),
            length_tree: Huffman::new(LENGTH_MAXSYMBOLS, LENGTH_TABLEBITS),
            aligned: Huffman::new(ALIGNED_MAXSYMBOLS, ALIGNED_TABLEBITS),
        })
    }

    fn read_lens(&mut self, which: TreeId, first: usize, last: usize) -> Result<()> {
        for x in 0..20 {
            self.pretree.len[x] = self.bits.read(4)? as u8;
        }
        if !self.pretree.build() {
            return Err(Error::Lzx("bad pretree"));
        }
        let lens = match which {
            TreeId::Main => &mut self.maintree.len,
            TreeId::Length => &mut self.length_tree.len,
        };
        let mut x = first;
        while x < last {
            let z = self.bits.read_huffsym(&self.pretree)?;
            match z {
                17 => {
                    let y = self.bits.read(4)? + 4;
                    for _ in 0..y {
                        set_len(lens, &mut x, 0)?;
                    }
                }
                18 => {
                    let y = self.bits.read(5)? + 20;
                    for _ in 0..y {
                        set_len(lens, &mut x, 0)?;
                    }
                }
                19 => {
                    let y = self.bits.read(1)? + 4;
                    let z = self.bits.read_huffsym(&self.pretree)?;
                    let v = delta_len(lens, x, z)?;
                    for _ in 0..y {
                        set_len(lens, &mut x, v)?;
                    }
                }
                _ => {
                    let v = delta_len(lens, x, z)?;
                    set_len(lens, &mut x, v)?;
                }
            }
        }
        Ok(())
    }

    fn decompress(&mut self, out: &mut [u8]) -> Result<()> {
        let mut out_bytes = out.len();
        if out_bytes == 0 {
            return Ok(());
        }
        let window_size = self.window_size;
        let end_frame = ((self.offset + out_bytes) / FRAME_SIZE) as u32 + 1;
        let mut out_pos = 0usize;
        let mut e8_buf = vec![0u8; FRAME_SIZE];

        while self.frame < end_frame {
            if !self.header_read {
                let mut j = 0u32;
                let mut i = self.bits.read(1)?;
                if i != 0 {
                    i = self.bits.read(16)?;
                    j = self.bits.read(16)?;
                }
                self.intel_filesize = ((i << 16) | j) as i32;
                self.header_read = true;
            }

            let mut frame_size = FRAME_SIZE as u32;
            if self.length != 0 && self.length - self.offset < frame_size as usize {
                frame_size = (self.length - self.offset) as u32;
            }

            let mut bytes_todo = self.frame_posn as i64 + frame_size as i64 - self.window_posn as i64;
            while bytes_todo > 0 {
                if self.block_remaining == 0 {
                    if self.block_type == BLOCKTYPE_UNCOMPRESSED && (self.block_length & 1) != 0 {
                        self.bits.raw_byte()?;
                    }
                    self.block_type = self.bits.read(3)?;
                    let i = self.bits.read(16)?;
                    let j = self.bits.read(8)?;
                    self.block_length = (i << 8) | j;
                    self.block_remaining = self.block_length;

                    match self.block_type {
                        BLOCKTYPE_ALIGNED | BLOCKTYPE_VERBATIM => {
                            if self.block_type == BLOCKTYPE_ALIGNED {
                                for i in 0..8 {
                                    self.aligned.len[i] = self.bits.read(3)? as u8;
                                }
                                if !self.aligned.build() {
                                    return Err(Error::Lzx("bad aligned table"));
                                }
                            }
                            self.read_lens(TreeId::Main, 0, 256)?;
                            self.read_lens(TreeId::Main, 256, NUM_CHARS + self.num_offsets)?;
                            if !self.maintree.build() {
                                return Err(Error::Lzx("bad main table"));
                            }
                            if self.maintree.len[0xE8] != 0 {
                                self.intel_started = true;
                            }
                            self.read_lens(TreeId::Length, 0, NUM_SECONDARY_LENGTHS)?;
                            self.length_tree.build_maybe_empty()?;
                        }
                        BLOCKTYPE_UNCOMPRESSED => {
                            self.intel_started = true;
                            if self.bits.bits_left == 0 {
                                self.bits.ensure(16)?;
                            }
                            self.bits.bits_left = 0;
                            self.bits.bit_buffer = 0;
                            let mut buf = [0u8; 12];
                            for b in buf.iter_mut() {
                                *b = self.bits.raw_byte()?;
                            }
                            self.r0 = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                            self.r1 = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
                            self.r2 = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
                        }
                        _ => return Err(Error::Lzx("bad block type")),
                    }
                }

                let mut this_run = self.block_remaining as i64;
                if this_run > bytes_todo {
                    this_run = bytes_todo;
                }
                bytes_todo -= this_run;
                self.block_remaining -= this_run as u32;

                match self.block_type {
                    BLOCKTYPE_ALIGNED | BLOCKTYPE_VERBATIM => {
                        while this_run > 0 {
                            let mut main_element = self.bits.read_huffsym(&self.maintree)?;
                            if main_element < NUM_CHARS {
                                self.window[self.window_posn as usize] = main_element as u8;
                                self.window_posn += 1;
                                this_run -= 1;
                                continue;
                            }
                            main_element -= NUM_CHARS;

                            let mut match_length = main_element & NUM_PRIMARY_LENGTHS;
                            if match_length == NUM_PRIMARY_LENGTHS {
                                if self.length_tree.empty {
                                    return Err(Error::Lzx("length symbol needed but tree is empty"));
                                }
                                let footer = self.bits.read_huffsym(&self.length_tree)?;
                                match_length += footer;
                            }
                            match_length += MIN_MATCH;

                            let slot = main_element >> 3;
                            let match_offset: u32 = match slot {
                                0 => self.r0,
                                1 => {
                                    std::mem::swap(&mut self.r1, &mut self.r0);
                                    self.r0
                                }
                                2 => {
                                    std::mem::swap(&mut self.r2, &mut self.r0);
                                    self.r0
                                }
                                _ => {
                                    let extra = if slot >= 36 {
                                        17
                                    } else {
                                        u32::from(EXTRA_BITS[slot])
                                    };
                                    let mut m = position_base(slot).wrapping_sub(2);
                                    if extra >= 3 && self.block_type == BLOCKTYPE_ALIGNED {
                                        if extra > 3 {
                                            let v = self.bits.read(extra - 3)?;
                                            m = m.wrapping_add(v << 3);
                                        }
                                        let a = self.bits.read_huffsym(&self.aligned)? as u32;
                                        m = m.wrapping_add(a);
                                    } else if extra != 0 {
                                        let v = self.bits.read(extra)?;
                                        m = m.wrapping_add(v);
                                    }
                                    self.r2 = self.r1;
                                    self.r1 = self.r0;
                                    self.r0 = m;
                                    m
                                }
                            };

                            if self.window_posn as usize + match_length > window_size as usize {
                                return Err(Error::Lzx("match ran over window wrap"));
                            }

                            let mut dest = self.window_posn as usize;
                            let mut i = match_length;
                            if match_offset > self.window_posn {
                                if match_offset as usize > self.offset {
                                    return Err(Error::Lzx("match offset beyond stream"));
                                }
                                let mut j = (match_offset - self.window_posn) as usize;
                                if j > window_size as usize {
                                    return Err(Error::Lzx("match offset beyond window"));
                                }
                                let mut src = window_size as usize - j;
                                if j < i {
                                    i -= j;
                                    while j > 0 {
                                        self.window[dest] = self.window[src];
                                        dest += 1;
                                        src += 1;
                                        j -= 1;
                                    }
                                    src = 0;
                                }
                                while i > 0 {
                                    self.window[dest] = self.window[src];
                                    dest += 1;
                                    src += 1;
                                    i -= 1;
                                }
                            } else {
                                let mut src = dest - match_offset as usize;
                                while i > 0 {
                                    self.window[dest] = self.window[src];
                                    dest += 1;
                                    src += 1;
                                    i -= 1;
                                }
                            }

                            this_run -= match_length as i64;
                            self.window_posn += match_length as u32;
                        }
                    }
                    BLOCKTYPE_UNCOMPRESSED => {
                        let start = self.window_posn as usize;
                        self.window_posn += this_run as u32;
                        for k in 0..this_run as usize {
                            self.window[start + k] = self.bits.raw_byte()?;
                        }
                        this_run = 0;
                    }
                    _ => return Err(Error::Lzx("bad block type")),
                }

                if this_run < 0 {
                    let over = (-this_run) as u32;
                    if over > self.block_remaining {
                        return Err(Error::Lzx("match overran block"));
                    }
                    self.block_remaining -= over;
                }
            }

            if self.window_posn.wrapping_sub(self.frame_posn) != frame_size {
                return Err(Error::Lzx("decode beyond output frame limits"));
            }

            if self.bits.bits_left > 0 {
                self.bits.ensure(16)?;
            }
            if self.bits.bits_left & 15 != 0 {
                let n = (self.bits.bits_left & 15) as u32;
                self.bits.remove(n);
            }

            let fp = self.frame_posn as usize;
            let fs = frame_size as usize;
            let frame_data: &[u8] =
                if self.intel_started && self.intel_filesize != 0 && self.frame < 32768 && fs > 10 {
                    e8_buf[..fs].copy_from_slice(&self.window[fp..fp + fs]);
                    let mut curpos = self.offset as i32;
                    let filesize = self.intel_filesize;
                    let mut d = 0usize;
                    let dataend = fs - 10;
                    while d < dataend {
                        let b = e8_buf[d];
                        d += 1;
                        if b != 0xE8 {
                            curpos = curpos.wrapping_add(1);
                            continue;
                        }
                        let abs_off =
                            i32::from_le_bytes([e8_buf[d], e8_buf[d + 1], e8_buf[d + 2], e8_buf[d + 3]]);
                        if abs_off >= curpos.wrapping_neg() && abs_off < filesize {
                            let rel_off = if abs_off >= 0 {
                                abs_off.wrapping_sub(curpos)
                            } else {
                                abs_off.wrapping_add(filesize)
                            };
                            e8_buf[d..d + 4].copy_from_slice(&rel_off.to_le_bytes());
                        }
                        d += 4;
                        curpos = curpos.wrapping_add(5);
                    }
                    &e8_buf[..fs]
                } else {
                    &self.window[fp..fp + fs]
                };

            let n = out_bytes.min(fs);
            out[out_pos..out_pos + n].copy_from_slice(&frame_data[..n]);
            out_pos += n;
            self.offset += n;
            out_bytes -= n;

            self.frame_posn += frame_size;
            self.frame += 1;
            if self.window_posn == window_size {
                self.window_posn = 0;
            }
            if self.frame_posn == window_size {
                self.frame_posn = 0;
            }
        }

        if out_bytes != 0 {
            return Err(Error::Lzx("bytes left to output"));
        }
        Ok(())
    }
}

fn set_len(lens: &mut [u8], x: &mut usize, v: u8) -> Result<()> {
    *lens
        .get_mut(*x)
        .ok_or(Error::Lzx("code length run overflows table"))? = v;
    *x += 1;
    Ok(())
}

fn delta_len(lens: &[u8], x: usize, z: usize) -> Result<u8> {
    let cur = *lens.get(x).ok_or(Error::Lzx("code length run overflows table"))?;
    let mut v = i32::from(cur) - z as i32;
    if v < 0 {
        v += 17;
    }
    Ok(v as u8)
}

#[derive(Clone, Copy)]
enum TreeId {
    Main,
    Length,
}

/// Decodes one LZX stream into `out`, sized to the expected output length.
pub fn decompress(input: &[u8], out: &mut [u8], window_size: u32) -> Result<()> {
    if !window_size.is_power_of_two() {
        return Err(Error::Lzx("window size is not a power of two"));
    }
    let window_bits = window_size.trailing_zeros();
    let mut decoder = Decoder::new(input, window_bits, out.len())?;
    decoder.decompress(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_base_matches_table() {
        let expected = [
            0u32, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512,
        ];
        for (i, &e) in expected.iter().enumerate() {
            assert_eq!(position_base(i), e);
        }
        assert_eq!(position_base(30), 32768);
        assert_eq!(position_base(36), 262144);
        assert_eq!(position_base(37), 393216);
        assert_eq!(position_base(289), 33423360);
    }

    #[test]
    fn uncompressed_block_round_trip() {
        // Header bit 0 (no E8), block type 3, length 5, then pad, R0..R2, raw bytes.
        let payload = b"hello";
        let mut bits: Vec<(u32, u32)> = vec![(0, 1), (3, 3), (0, 16), (5, 8)];
        let mut words: Vec<u16> = Vec::new();
        let mut acc: u32 = 0;
        let mut n = 0u32;
        for (v, w) in bits.drain(..) {
            acc = (acc << w) | v;
            n += w;
        }
        acc <<= 32 - n;
        words.push((acc >> 16) as u16);
        words.push(acc as u16);
        let mut input = Vec::new();
        for w in words {
            input.extend_from_slice(&w.to_le_bytes());
        }
        input.extend_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        input.extend_from_slice(payload);
        let mut out = [0u8; 5];
        decompress(&input, &mut out, 0x8000).unwrap();
        assert_eq!(&out, payload);
    }

    #[test]
    fn garbage_does_not_panic() {
        let mut seed = 0x1234_5678u32;
        for len in [0usize, 1, 2, 7, 64, 513] {
            let input: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect();
            let mut out = vec![0u8; 40000];
            let _ = decompress(&input, &mut out, 0x8000);
        }
    }
}
