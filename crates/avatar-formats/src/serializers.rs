// Port of xenia/kernel/xam/avatars/serializers.h.

use glam::{Quat, Vec3};

use crate::bits::BitStream;
use crate::error::{Error, Result};

/// Lattice-quantized vector: per-axis bit widths over a base point and a hexagonal-packed step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VectorSerializer {
    pub quant_radius: f32,
    pub base_x: f32,
    pub base_y: f32,
    pub base_z: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub delta_z: f32,
    pub bit_count_x: u8,
    pub bit_count_y: u8,
    pub bit_count_z: u8,
}

impl VectorSerializer {
    pub fn element_bit_size(&self) -> usize {
        self.bit_count_x as usize + self.bit_count_y as usize + self.bit_count_z as usize
    }

    pub const fn serializer_bit_size() -> usize {
        32 + 32 + 32 + 32 + 6 + 6 + 6
    }

    pub fn from_stream(stream: &mut BitStream) -> Result<Self> {
        let quant_radius = stream.read_f32()?;
        let base_x = stream.read_f32()?;
        let base_y = stream.read_f32()?;
        let base_z = stream.read_f32()?;
        let bit_count_x = stream.read_u8_bits(6)?;
        let bit_count_y = stream.read_u8_bits(6)?;
        let bit_count_z = stream.read_u8_bits(6)?;
        Ok(Self {
            quant_radius,
            base_x,
            base_y,
            base_z,
            delta_x: quant_radius * 2.0,
            delta_y: quant_radius * (2.0f32 / 3.0f32) * 6.0f32.sqrt(),
            delta_z: quant_radius * 3.0f32.sqrt(),
            bit_count_x,
            bit_count_y,
            bit_count_z,
        })
    }

    pub fn invert(&mut self) {
        self.base_z = -self.base_z;
        self.delta_z = -self.delta_z;
    }

    pub fn read(&self, stream: &mut BitStream) -> Result<Vec3> {
        let x = stream.read_i64_bits(self.bit_count_x as usize)?;
        let y = stream.read_i64_bits(self.bit_count_y as usize)?;
        let z = stream.read_i64_bits(self.bit_count_z as usize)?;
        let out_y = self.base_y + self.delta_y * y as f32;
        let mut out_z = self.base_z + self.delta_z * z as f32;
        if (y & 1) != 0 {
            out_z += (1.0f32 / 3.0f32) * self.delta_z;
        }
        let mut out_x = self.base_x + self.delta_x * x as f32;
        if (y & 1) != (z & 1) {
            out_x += 0.5 * self.delta_x;
        }
        Ok(Vec3::new(out_x, out_y, out_z))
    }
}

/// Rotation stored as a quantized axis-angle vector.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuaternionSerializer {
    pub base_serializer: VectorSerializer,
}

impl QuaternionSerializer {
    pub fn element_bit_size(&self) -> usize {
        self.base_serializer.element_bit_size()
    }

    pub const fn serializer_bit_size() -> usize {
        VectorSerializer::serializer_bit_size()
    }

    pub fn from_stream(stream: &mut BitStream) -> Result<Self> {
        Ok(Self {
            base_serializer: VectorSerializer::from_stream(stream)?,
        })
    }

    pub fn invert(&mut self) {
        let s = &mut self.base_serializer;
        s.base_x = -s.base_x;
        s.delta_x = -s.delta_x;
        s.base_y = -s.base_y;
        s.delta_y = -s.delta_y;
    }

    pub fn read(&self, stream: &mut BitStream) -> Result<Quat> {
        let v = self.base_serializer.read(stream)?;
        let num = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        if num < 1e-7 {
            Ok(Quat::from_xyzw(0.0, 0.0, 0.0, 1.0))
        } else {
            let num2 = num.sin();
            let w = num.cos();
            let num3 = num2 / num;
            Ok(Quat::from_xyzw(v.x * num3, v.y * num3, v.z * num3, w))
        }
    }
}

/// Integer types a `ValueSerializer` can carry.
pub trait SerialValue: Copy + Default + std::fmt::Debug + PartialEq {
    const BITS: usize;
    fn read_full(stream: &mut BitStream) -> Result<Self>;
    /// C++ usual arithmetic conversion of the base value to `uint64_t`.
    fn to_u64(self) -> u64;
    fn from_u64(v: u64) -> Self;
}

impl SerialValue for u16 {
    const BITS: usize = 16;
    fn read_full(stream: &mut BitStream) -> Result<Self> {
        stream.read_u16()
    }
    fn to_u64(self) -> u64 {
        u64::from(self)
    }
    fn from_u64(v: u64) -> Self {
        v as u16
    }
}

impl SerialValue for u32 {
    const BITS: usize = 32;
    fn read_full(stream: &mut BitStream) -> Result<Self> {
        stream.read_u32()
    }
    fn to_u64(self) -> u64 {
        u64::from(self)
    }
    fn from_u64(v: u64) -> Self {
        v as u32
    }
}

impl SerialValue for i32 {
    const BITS: usize = 32;
    fn read_full(stream: &mut BitStream) -> Result<Self> {
        stream.read_i32()
    }
    fn to_u64(self) -> u64 {
        self as i64 as u64
    }
    fn from_u64(v: u64) -> Self {
        v as u32 as i32
    }
}

/// Base value plus a fixed-width unsigned delta.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ValueSerializer<T: SerialValue> {
    pub base_value: T,
    pub bit_count: T,
}

impl<T: SerialValue> ValueSerializer<T> {
    pub fn element_bit_size(&self) -> usize {
        self.bit_count.to_u64() as usize
    }

    pub const fn serializer_bit_size() -> usize {
        T::BITS * 2
    }

    pub fn from_stream(stream: &mut BitStream) -> Result<Self> {
        let base_value = T::read_full(stream)?;
        let bit_count = T::read_full(stream)?;
        if bit_count.to_u64() > 64 {
            return Err(Error::Malformed("value serializer bit count above 64"));
        }
        Ok(Self {
            base_value,
            bit_count,
        })
    }

    pub fn read(&self, stream: &mut BitStream) -> Result<T> {
        let delta = stream.read(self.bit_count.to_u64() as usize)?;
        Ok(T::from_u64(self.base_value.to_u64().wrapping_add(delta)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BitWriter {
        bytes: Vec<u8>,
        bit: usize,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                bit: 0,
            }
        }
        fn put(&mut self, value: u64, bits: usize) {
            for i in 0..bits {
                if self.bit / 8 >= self.bytes.len() {
                    self.bytes.push(0);
                }
                if (value >> i) & 1 != 0 {
                    self.bytes[self.bit / 8] |= 1 << (self.bit % 8);
                }
                self.bit += 1;
            }
        }
        fn put_f32(&mut self, v: f32) {
            self.put(u64::from(v.to_bits()), 32);
        }
    }

    #[test]
    fn value_serializer_adds_base() {
        let mut w = BitWriter::new();
        w.put((-5i32) as u32 as u64, 32);
        w.put(4, 32);
        w.put(9, 4);
        w.put(15, 4);
        let mut s = BitStream::new(&w.bytes);
        let ser = ValueSerializer::<i32>::from_stream(&mut s).unwrap();
        assert_eq!(ser.element_bit_size(), 4);
        assert_eq!(ser.read(&mut s).unwrap(), 4);
        assert_eq!(ser.read(&mut s).unwrap(), 10);
    }

    #[test]
    fn value_serializer_rejects_huge_width() {
        let mut w = BitWriter::new();
        w.put(0, 16);
        w.put(65, 16);
        let mut s = BitStream::new(&w.bytes);
        assert!(ValueSerializer::<u16>::from_stream(&mut s).is_err());
    }

    #[test]
    fn vector_serializer_lattice() {
        let mut w = BitWriter::new();
        w.put_f32(0.5);
        w.put_f32(1.0);
        w.put_f32(2.0);
        w.put_f32(3.0);
        w.put(3, 6);
        w.put(3, 6);
        w.put(3, 6);
        w.put(2, 3);
        w.put(1, 3);
        w.put(0, 3);
        let mut s = BitStream::new(&w.bytes);
        let ser = VectorSerializer::from_stream(&mut s).unwrap();
        assert_eq!(s.offset_bits(), VectorSerializer::serializer_bit_size());
        assert_eq!(ser.delta_x, 1.0);
        let v = ser.read(&mut s).unwrap();
        let dy = 0.5f32 * (2.0 / 3.0) * 6.0f32.sqrt();
        let dz = 0.5f32 * 3.0f32.sqrt();
        assert_eq!(v.y, 2.0 + dy);
        assert_eq!(v.z, 3.0 + (1.0 / 3.0) * dz);
        assert_eq!(v.x, 1.0 + 2.0 + 0.5);
    }

    #[test]
    fn quaternion_identity_for_zero_vector() {
        let mut w = BitWriter::new();
        for _ in 0..4 {
            w.put_f32(0.0);
        }
        w.put(0, 18);
        let mut s = BitStream::new(&w.bytes);
        let ser = QuaternionSerializer::from_stream(&mut s).unwrap();
        assert_eq!(ser.read(&mut s).unwrap(), Quat::IDENTITY);
    }
}
