// Port of xenia/kernel/xam/avatars/blend_shape.{h,cpp}.

pub mod apply;

use glam::Vec3;

use crate::asset_pack::AssetId;
use crate::bits::BitStream;
use crate::error::{ensure, Result};
use crate::model::invert_normal_z;
use crate::serializers::{ValueSerializer, VectorSerializer};
use crate::strb::{self, BlockId};

/// Blend shape load flags (BlendShapeLoadOption in the C++).
pub mod load_option {
    pub const NONE: u32 = 0;
    pub const INVERT: u32 = 1 << 0;
}

fn read_asset_id(stream: &mut BitStream) -> Result<AssetId> {
    let a = stream.read_u32()?;
    let b = stream.read_u16()?;
    let c = stream.read_u16()?;
    let mut d = [0u8; 8];
    stream.read_bytes(&mut d)?;
    Ok(AssetId { a, b, c, d })
}

/// Triangles to collapse, as global triangle indices across the target's batches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendShapeIndexPatch {
    pub total_buffer_size: u32,
    pub original_asset_id: AssetId,
    pub indices: Vec<i32>,
}

impl BlendShapeIndexPatch {
    pub fn read(stream: &mut BitStream) -> Result<Self> {
        let index_count = stream.read_u32()?;
        ensure(index_count <= 8192, "blend shape index count above 8192")?;
        let total_buffer_size = stream.read_u32()?;
        let original_asset_id = read_asset_id(stream)?;
        let serializer = ValueSerializer::<i32>::from_stream(stream)?;
        let mut indices = Vec::with_capacity(index_count as usize);
        for _ in 0..index_count {
            indices.push(serializer.read(stream)?);
        }
        // The index list is byte aligned; odd counts at 12-bit width end mid-byte.
        stream.align_to_next_byte()?;
        Ok(Self {
            total_buffer_size,
            original_asset_id,
            indices,
        })
    }
}

/// Replacement vertex; `original_offset` is a byte offset into the target's concatenated vertex buffers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendShapeVertex {
    pub original_offset: i32,
    pub position: Vec3,
    pub normal: u32,
    pub blend_weight: u32,
    pub blend_indices: u32,
    pub color: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct BlendShapeVertexSerializer {
    inverted: bool,
    original_index_serializer: ValueSerializer<i32>,
    position_serializer: VectorSerializer,
    normal_serializer: ValueSerializer<u32>,
    blend_weight_serializer: ValueSerializer<u32>,
    blend_index_serializer: ValueSerializer<u32>,
    color_serializer: ValueSerializer<u32>,
}

impl BlendShapeVertexSerializer {
    fn from_stream(stream: &mut BitStream) -> Result<Self> {
        Ok(Self {
            inverted: false,
            original_index_serializer: ValueSerializer::from_stream(stream)?,
            position_serializer: VectorSerializer::from_stream(stream)?,
            normal_serializer: ValueSerializer::from_stream(stream)?,
            blend_weight_serializer: ValueSerializer::from_stream(stream)?,
            blend_index_serializer: ValueSerializer::from_stream(stream)?,
            color_serializer: ValueSerializer::from_stream(stream)?,
        })
    }

    fn invert(&mut self) {
        self.inverted = !self.inverted;
        self.position_serializer.invert();
    }

    fn read(&self, stream: &mut BitStream) -> Result<BlendShapeVertex> {
        let original_offset = self.original_index_serializer.read(stream)?;
        let position = self.position_serializer.read(stream)?;
        let mut normal = self.normal_serializer.read(stream)?;
        if self.inverted {
            normal = invert_normal_z(normal);
        }
        Ok(BlendShapeVertex {
            original_offset,
            position,
            normal,
            blend_weight: self.blend_weight_serializer.read(stream)?,
            blend_indices: self.blend_index_serializer.read(stream)?,
            color: self.color_serializer.read(stream)?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendShapeVertexPatch {
    pub total_buffer_size: u32,
    pub original_asset_id: AssetId,
    pub vertices: Vec<BlendShapeVertex>,
}

impl BlendShapeVertexPatch {
    pub fn read(stream: &mut BitStream, load_options: u32) -> Result<Self> {
        let vertex_count = stream.read_u32()?;
        ensure(vertex_count <= 8192, "blend shape vertex count above 8192")?;
        let total_buffer_size = stream.read_u32()?;
        let original_asset_id = read_asset_id(stream)?;
        let mut serializer = BlendShapeVertexSerializer::from_stream(stream)?;
        if load_options & load_option::INVERT != 0 {
            serializer.invert();
        }
        let mut vertices = Vec::with_capacity(vertex_count as usize);
        for _ in 0..vertex_count {
            vertices.push(serializer.read(stream)?);
        }
        Ok(Self {
            total_buffer_size,
            original_asset_id,
            vertices,
        })
    }
}

/// Exact id compare, except body targets (a = 2) ignore `b`, which differs between sources.
pub fn target_matches(target: &AssetId, asset_id: &AssetId) -> bool {
    if target == asset_id {
        return true;
    }
    if target.a == 2 && asset_id.a == 2 {
        return target.c == asset_id.c && target.d == asset_id.d;
    }
    false
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendShape {
    pub index_patch: BlendShapeIndexPatch,
    pub vertex_patch: BlendShapeVertexPatch,
}

impl BlendShape {
    pub fn matches(&self, asset_id: &AssetId) -> bool {
        target_matches(&self.index_patch.original_asset_id, asset_id)
            && target_matches(&self.vertex_patch.original_asset_id, asset_id)
    }

    /// Parses one raw kShapeOverrides block (shape blocks are not compressed).
    pub fn read_data(data: &[u8], load_options: u32) -> Result<Self> {
        let mut stream = BitStream::new(data);
        let index_patch = BlendShapeIndexPatch::read(&mut stream)?;
        let vertex_patch = BlendShapeVertexPatch::read(&mut stream, load_options)?;
        stream.align_to_next_byte()?;
        ensure(
            stream.offset_bits() == stream.size_bits(),
            "trailing blend shape data",
        )?;
        Ok(Self {
            index_patch,
            vertex_patch,
        })
    }

    /// Reads the first shape block; unisex items carry one per body, see `load_all`.
    pub fn load(strb_buffer: &[u8], load_options: u32) -> Result<Option<Self>> {
        let Some(data) = strb::find_block(strb_buffer, BlockId::ShapeOverrides)? else {
            return Ok(None);
        };
        Self::read_data(data, load_options).map(Some)
    }

    /// Reads every shape block in container order.
    pub fn load_all(strb_buffer: &[u8], load_options: u32) -> Result<Vec<Self>> {
        strb::blocks(strb_buffer)?
            .into_iter()
            .filter(|b| b.id == BlockId::ShapeOverrides)
            .map(|b| Self::read_data(b.data, load_options))
            .collect()
    }
}
