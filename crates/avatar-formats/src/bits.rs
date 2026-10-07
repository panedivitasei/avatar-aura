// Port of xenia/kernel/xam/avatars/bit_stream.{h,cpp}.

use crate::error::{Error, Result};

/// Bit reader over a byte buffer; bits are consumed from the low end of each byte first.
#[derive(Clone, Debug)]
pub struct BitStream<'a> {
    buffer: &'a [u8],
    offset_bits: usize,
    size_bits: usize,
}

impl<'a> BitStream<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer,
            offset_bits: 0,
            size_bits: buffer.len() * 8,
        }
    }

    pub fn with_bits(buffer: &'a [u8], size_bits: usize) -> Result<Self> {
        if size_bits > buffer.len().saturating_mul(8) {
            return Err(Error::Overrun {
                offset: 0,
                needed: size_bits,
                size: buffer.len() * 8,
            });
        }
        Ok(Self {
            buffer,
            offset_bits: 0,
            size_bits,
        })
    }

    pub fn buffer(&self) -> &'a [u8] {
        self.buffer
    }

    pub fn offset_bits(&self) -> usize {
        self.offset_bits
    }

    pub fn size_bits(&self) -> usize {
        self.size_bits
    }

    pub fn bits_remaining(&self) -> usize {
        self.size_bits - self.offset_bits
    }

    pub fn set_offset(&mut self, offset_bits: usize) -> Result<()> {
        if offset_bits > self.size_bits {
            return Err(Error::Overrun {
                offset: self.offset_bits,
                needed: offset_bits.saturating_sub(self.offset_bits),
                size: self.size_bits,
            });
        }
        self.offset_bits = offset_bits;
        Ok(())
    }

    pub fn advance(&mut self, num_bits: usize) -> Result<()> {
        let target = self.offset_bits.checked_add(num_bits).ok_or(Error::Overrun {
            offset: self.offset_bits,
            needed: num_bits,
            size: self.size_bits,
        })?;
        self.set_offset(target)
    }

    pub fn align_to_next_byte(&mut self) -> Result<()> {
        let aligned = (self.offset_bits + 7) & !7;
        self.set_offset(aligned)
    }

    /// Splits off the next `num_bits` as an independent stream and advances past them.
    pub fn slice(&mut self, num_bits: usize) -> Result<BitStream<'a>> {
        self.check(num_bits)?;
        let read_bits = self.offset_bits % 8;
        let start = self.offset_bits >> 3;
        let mut stream = BitStream::with_bits(&self.buffer[start..], read_bits + num_bits)?;
        stream.set_offset(read_bits)?;
        self.advance(num_bits)?;
        Ok(stream)
    }

    fn check(&self, num_bits: usize) -> Result<()> {
        match self.offset_bits.checked_add(num_bits) {
            Some(end) if end <= self.size_bits => Ok(()),
            _ => Err(Error::Overrun {
                offset: self.offset_bits,
                needed: num_bits,
                size: self.size_bits,
            }),
        }
    }

    pub fn peek(&self, num_bits: usize) -> Result<u64> {
        if num_bits > 64 {
            return Err(Error::ReadWidth {
                requested: num_bits,
                width: 64,
            });
        }
        self.check(num_bits)?;
        let mut offset_bits = self.offset_bits;
        let mut num_bits = num_bits;
        let mut result = 0u64;
        let mut shift = 0usize;
        while num_bits > 0 {
            let remaining_bits = offset_bits % 8;
            let desired_bits = (8 - remaining_bits).min(num_bits);
            let mask = (1u64 << desired_bits) - 1;
            let value = u64::from(self.buffer[offset_bits >> 3] >> remaining_bits);
            result |= (mask & value) << shift;
            shift += desired_bits;
            num_bits -= desired_bits;
            offset_bits += desired_bits;
        }
        Ok(result)
    }

    pub fn read(&mut self, num_bits: usize) -> Result<u64> {
        let value = self.peek(num_bits)?;
        self.advance(num_bits)?;
        Ok(value)
    }

    fn read_width(&mut self, num_bits: usize, width: usize) -> Result<u64> {
        if num_bits > width {
            return Err(Error::ReadWidth {
                requested: num_bits,
                width,
            });
        }
        self.read(num_bits)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read(8)? as u8)
    }

    pub fn read_u8_bits(&mut self, num_bits: usize) -> Result<u8> {
        Ok(self.read_width(num_bits, 8)? as u8)
    }

    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(self.read(16)? as u16)
    }

    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(self.read(32)? as u32)
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(self.read(32)? as u32 as i32)
    }

    pub fn read_i64_bits(&mut self, num_bits: usize) -> Result<i64> {
        Ok(self.read_width(num_bits, 64)? as i64)
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read(1)? != 0)
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    pub fn read_bytes(&mut self, out: &mut [u8]) -> Result<()> {
        for b in out.iter_mut() {
            *b = self.read_u8()?;
        }
        Ok(())
    }

    /// Copies `count` bytes starting at the byte holding the current bit offset.
    pub fn take_bytes(&mut self, count: usize) -> Result<Vec<u8>> {
        let bits = count
            .checked_mul(8)
            .ok_or(Error::Malformed("byte count overflow"))?;
        self.check(bits)?;
        let start = self.offset_bits >> 3;
        let bytes = self.buffer[start..start + count].to_vec();
        self.advance(bits)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_low_bits_first() {
        let data = [0b1010_1100u8, 0xFF, 0x12, 0x34, 0x56, 0x78];
        let mut s = BitStream::new(&data);
        assert_eq!(s.read(2).unwrap(), 0b00);
        assert_eq!(s.read(3).unwrap(), 0b011);
        assert_eq!(s.read(3).unwrap(), 0b101);
        assert_eq!(s.read_u8().unwrap(), 0xFF);
        assert_eq!(s.read_u32().unwrap(), 0x7856_3412);
        assert!(s.read(1).is_err());
    }

    #[test]
    fn straddles_bytes_and_aligns() {
        let data = [0xF0u8, 0x0F, 0xAA];
        let mut s = BitStream::new(&data);
        s.advance(4).unwrap();
        assert_eq!(s.read(8).unwrap(), 0xFF);
        s.align_to_next_byte().unwrap();
        assert_eq!(s.offset_bits(), 16);
        assert_eq!(s.read_u8().unwrap(), 0xAA);
        assert!(s.align_to_next_byte().is_ok());
    }

    #[test]
    fn slice_keeps_bit_phase() {
        let data = [0b1111_0000u8, 0b0000_0001];
        let mut s = BitStream::new(&data);
        s.advance(3).unwrap();
        let mut sub = s.slice(6).unwrap();
        assert_eq!(s.offset_bits(), 9);
        assert_eq!(sub.offset_bits(), 3);
        assert_eq!(sub.read(6).unwrap(), 0b111110);
        assert!(sub.read(1).is_err());
    }

    #[test]
    fn rejects_wide_reads() {
        let data = [0u8; 16];
        let mut s = BitStream::new(&data);
        assert!(s.read(65).is_err());
        assert!(s.read_u8_bits(9).is_err());
    }
}
