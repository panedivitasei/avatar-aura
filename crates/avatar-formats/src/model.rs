// Port of xenia/kernel/xam/avatars/model.{h,cpp}.

use glam::Vec3;

use crate::bits::BitStream;
use crate::compression;
use crate::error::{ensure, Result};
use crate::serializers::{ValueSerializer, VectorSerializer};
use crate::strb::{self, BlockId};
use crate::texture::Texture;

/// Model load flags (ModelLoadOption in the C++).
pub mod load_option {
    pub const NONE: u32 = 0;
    /// Mirrors the mesh across Z (left-handed output).
    pub const INVERT: u32 = 1 << 0;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaderParameterType {
    #[default]
    Invalid,
    Texture,
    VertexConstant,
    PixelConstant,
    Other(u32),
}

impl ShaderParameterType {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            0 => Self::Invalid,
            1 => Self::Texture,
            2 => Self::VertexConstant,
            3 => Self::PixelConstant,
            other => Self::Other(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderParameterTexture {
    pub index: u16,
    pub uv_layer: u16,
    pub flags: u16,
}

/// `texture` is filled for texture parameters, `constant_values` for every other type.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShaderParameter {
    pub r#type: ShaderParameterType,
    pub usage: u32,
    pub texture: ShaderParameterTexture,
    pub constant_values: [f32; 4],
}

impl ShaderParameter {
    pub fn read(stream: &mut BitStream) -> Result<Self> {
        let mut p = ShaderParameter {
            r#type: ShaderParameterType::from_raw(stream.read_u32()?),
            usage: stream.read_u32()?,
            ..Default::default()
        };
        if p.r#type == ShaderParameterType::Texture {
            p.texture.index = stream.read_u16()?;
            p.texture.uv_layer = stream.read_u16()?;
            p.texture.flags = stream.read_u32()? as u16;
            stream.advance(32 * 2)?;
        } else {
            for v in p.constant_values.iter_mut() {
                *v = stream.read_f32()?;
            }
        }
        Ok(p)
    }
}

/// UV pairs are Xenos half floats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uv {
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: u32,
    pub blend_weight: u32,
    pub blend_indices: u32,
    pub color: u32,
    pub uvs: Vec<Uv>,
}

/// Flips the 10-bit Z field of a packed normal.
pub(crate) fn invert_normal_z(normal: u32) -> u32 {
    let normal_z = (!(normal >> 22)).wrapping_add(1);
    (normal & !(0x3FFu32 << 22)) | (normal_z << 22)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriangleBatch {
    pub shader_id: u32,
    pub shader_parameters: Vec<ShaderParameter>,
    pub triangle_count: u32,
    pub uv_count: u32,
    pub vertex_size: u32,
    pub index_size: u32,
    pub vertex_array_offset: u32,
    pub index_array_offset: u32,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

impl TriangleBatch {
    pub fn read(stream: &mut BitStream, load_options: u32) -> Result<Self> {
        let mut b = TriangleBatch {
            shader_id: stream.read_u32()?,
            ..Default::default()
        };
        let shader_parameter_count = stream.read_u8_bits(5)?;
        b.triangle_count = stream.read_u32()?;
        let vertex_count = stream.read_u32()?;
        b.uv_count = stream.read_u32()?;
        b.vertex_size = stream.read_u32()?;
        b.index_size = stream.read_u32()?;
        b.vertex_array_offset = stream.read_u32()?;
        b.index_array_offset = stream.read_u32()?;
        stream.align_to_next_byte()?;

        ensure(b.triangle_count <= 8192, "triangle count above 8192")?;
        ensure(vertex_count <= 8192, "vertex count above 8192")?;
        ensure(b.uv_count <= 6, "uv count above 6")?;
        ensure(
            b.vertex_size == 32u32.wrapping_add(4u32.wrapping_mul(b.uv_count.wrapping_sub(1))),
            "vertex size does not match uv count",
        )?;

        for _ in 0..shader_parameter_count {
            b.shader_parameters.push(ShaderParameter::read(stream)?);
            stream.align_to_next_byte()?;
        }

        b.vertices = Self::read_vertices(stream, vertex_count as usize, b.uv_count as usize, load_options)?;
        stream.align_to_next_byte()?;

        b.indices = Self::read_indices(stream, b.triangle_count as usize)?;
        stream.align_to_next_byte()?;
        Ok(b)
    }

    fn read_vertices(
        stream: &mut BitStream,
        vertex_count: usize,
        uv_count: usize,
        load_options: u32,
    ) -> Result<Vec<Vertex>> {
        let local_vertex_count = stream.read_u32()?;
        ensure(
            vertex_count == local_vertex_count as usize,
            "vertex count mismatch",
        )?;

        let mut vector_serializer = VectorSerializer::from_stream(stream)?;
        let normal_serializer = ValueSerializer::<i32>::from_stream(stream)?;
        let blend_weight_serializer = ValueSerializer::<u32>::from_stream(stream)?;
        let blend_indices_serializer = ValueSerializer::<u32>::from_stream(stream)?;
        let color_serializer = ValueSerializer::<u32>::from_stream(stream)?;

        let inverted = load_options & load_option::INVERT != 0;
        if inverted {
            vector_serializer.invert();
        }

        let mut vertices = Vec::with_capacity(vertex_count);
        for _ in 0..vertex_count {
            let position = vector_serializer.read(stream)?;
            let mut normal = normal_serializer.read(stream)? as u32;
            if inverted {
                normal = invert_normal_z(normal);
            }
            let blend_weight = blend_weight_serializer.read(stream)?;
            let blend_indices = blend_indices_serializer.read(stream)?;
            let color = color_serializer.read(stream)?;
            let mut uvs = Vec::with_capacity(uv_count);
            for _ in 0..uv_count {
                let x = stream.read_u16()?;
                let y = stream.read_u16()?;
                uvs.push(Uv { x, y });
            }
            vertices.push(Vertex {
                position,
                normal,
                blend_weight,
                blend_indices,
                color,
                uvs,
            });
        }
        Ok(vertices)
    }

    fn read_indices(stream: &mut BitStream, triangle_count: usize) -> Result<Vec<u16>> {
        let index_count = stream.read_u32()? as usize;
        ensure(index_count == triangle_count * 3, "index count mismatch")?;
        let index_serializer = ValueSerializer::<u16>::from_stream(stream)?;
        let mut indices = Vec::with_capacity(index_count);
        for _ in 0..index_count {
            indices.push(index_serializer.read(stream)?);
        }
        Ok(indices)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelTexture {
    pub gpu_offset: u32,
    pub gpu_size: u32,
    pub texture: Texture,
}

impl ModelTexture {
    pub fn read(stream: &mut BitStream) -> Result<Self> {
        Ok(ModelTexture {
            gpu_offset: stream.read_u32()?,
            gpu_size: stream.read_u32()?,
            texture: Texture::read(stream)?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub cpu_size: u32,
    pub gpu_size: u32,
    pub texture_buffer_size: u32,
    pub vertex_buffer_size: u32,
    pub index_buffer_size: u32,
    pub vertex_buffer_offset: u32,
    pub index_buffer_offset: u32,
    pub triangle_batch_array_offset: u32,
    pub texture_array_offset: u32,
    pub texture_scratch_size: u32,
    pub triangle_batches: Vec<TriangleBatch>,
    pub textures: Vec<ModelTexture>,
}

impl Model {
    /// Parses an uncompressed model payload.
    pub fn read_data(data: &[u8], load_options: u32) -> Result<Self> {
        let mut stream = BitStream::new(data);
        let s = &mut stream;
        let mut m = Model {
            cpu_size: s.read_u32()?,
            gpu_size: s.read_u32()?,
            texture_buffer_size: s.read_u32()?,
            vertex_buffer_size: s.read_u32()?,
            index_buffer_size: s.read_u32()?,
            ..Default::default()
        };
        let triangle_batch_count = s.read_u32()?;
        let texture_count = s.read_u32()?;
        m.vertex_buffer_offset = s.read_u32()?;
        m.index_buffer_offset = s.read_u32()?;
        m.triangle_batch_array_offset = s.read_u32()?;
        m.texture_array_offset = s.read_u32()?;
        m.texture_scratch_size = s.read_u32()?;
        ensure(triangle_batch_count <= 16, "triangle batch count above 16")?;
        ensure(texture_count <= 18, "texture count above 18")?;

        for _ in 0..triangle_batch_count {
            m.triangle_batches.push(TriangleBatch::read(s, load_options)?);
            s.align_to_next_byte()?;
        }
        for _ in 0..texture_count {
            m.textures.push(ModelTexture::read(s)?);
            s.align_to_next_byte()?;
        }

        // Vertex-pair GPU offsets; parsed for framing only, the list ends on an odd count.
        loop {
            let vertex_pair_count = s.read_u32()?;
            ensure(vertex_pair_count <= 400, "vertex pair count above 400")?;
            let vertex_pair_serializer = ValueSerializer::<u32>::from_stream(s)?;
            for _ in 0..vertex_pair_count {
                vertex_pair_serializer.read(s)?;
            }
            s.align_to_next_byte()?;
            if vertex_pair_count & 1 != 0 {
                break;
            }
        }

        ensure(stream.offset_bits() == stream.size_bits(), "trailing model data")?;
        Ok(m)
    }

    /// Decodes the first kModel block of a YTGR/STRB container; `None` when it has none.
    pub fn load(strb_buffer: &[u8], load_options: u32) -> Result<Option<Self>> {
        let Some(compressed) = strb::find_block(strb_buffer, BlockId::Model)? else {
            return Ok(None);
        };
        let data = compression::decompress(compressed)?;
        Self::read_data(&data, load_options).map(Some)
    }

    pub fn vertex_count(&self) -> usize {
        self.triangle_batches.iter().map(|b| b.vertices.len()).sum()
    }

    pub fn triangle_count(&self) -> usize {
        self.triangle_batches.iter().map(|b| b.indices.len() / 3).sum()
    }
}
