// Port of xenia/kernel/xam/avatars/animation.{h,cpp}.

use glam::{Quat, Vec3};

use crate::bits::BitStream;
use crate::error::{ensure, Result};
use crate::serializers::{QuaternionSerializer, ValueSerializer, VectorSerializer};
use crate::strb::{self, BlockId};

/// Animation load flags (AnimationLoadOption in the C++).
pub mod load_option {
    pub const NONE: u32 = 0;
    /// Decode per-frame elements.
    pub const ELEMENTS: u32 = 1 << 0;
    /// Keep a copy of the compressed frame data.
    pub const COMPRESSED_DATA: u32 = 1 << 1;
    pub const INVERT: u32 = 1 << 2;
    pub const GUEST: u32 = COMPRESSED_DATA;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationPose {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationMotion {
    pub position: Vec3,
    pub rotation: Quat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimationTexture {
    pub layer_index: u32,
}

/// Per-element serializer for one frame-set kind.
pub trait ElementSerializer: Copy + Default {
    type Element: Copy;
    const SERIALIZER_BIT_SIZE: usize;
    fn from_stream(stream: &mut BitStream) -> Result<Self>;
    fn invert(&mut self);
    fn element_bit_size(&self) -> usize;
    fn read(&self, stream: &mut BitStream) -> Result<Self::Element>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationPoseSerializer {
    pub position_serializer: VectorSerializer,
    pub rotation_serializer: QuaternionSerializer,
    pub scale_serializer: VectorSerializer,
}

impl ElementSerializer for AnimationPoseSerializer {
    type Element = AnimationPose;
    const SERIALIZER_BIT_SIZE: usize = VectorSerializer::serializer_bit_size()
        + QuaternionSerializer::serializer_bit_size()
        + VectorSerializer::serializer_bit_size();

    fn from_stream(stream: &mut BitStream) -> Result<Self> {
        Ok(Self {
            position_serializer: VectorSerializer::from_stream(stream)?,
            rotation_serializer: QuaternionSerializer::from_stream(stream)?,
            scale_serializer: VectorSerializer::from_stream(stream)?,
        })
    }

    fn invert(&mut self) {
        self.position_serializer.invert();
        self.rotation_serializer.invert();
    }

    fn element_bit_size(&self) -> usize {
        self.position_serializer.element_bit_size()
            + self.rotation_serializer.element_bit_size()
            + self.scale_serializer.element_bit_size()
    }

    fn read(&self, stream: &mut BitStream) -> Result<AnimationPose> {
        Ok(AnimationPose {
            position: self.position_serializer.read(stream)?,
            rotation: self.rotation_serializer.read(stream)?,
            scale: self.scale_serializer.read(stream)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationMotionSerializer {
    pub position_serializer: VectorSerializer,
    pub rotation_serializer: QuaternionSerializer,
}

impl ElementSerializer for AnimationMotionSerializer {
    type Element = AnimationMotion;
    const SERIALIZER_BIT_SIZE: usize =
        VectorSerializer::serializer_bit_size() + QuaternionSerializer::serializer_bit_size();

    fn from_stream(stream: &mut BitStream) -> Result<Self> {
        Ok(Self {
            position_serializer: VectorSerializer::from_stream(stream)?,
            rotation_serializer: QuaternionSerializer::from_stream(stream)?,
        })
    }

    fn invert(&mut self) {
        self.position_serializer.invert();
        self.rotation_serializer.invert();
    }

    fn element_bit_size(&self) -> usize {
        self.position_serializer.element_bit_size() + self.rotation_serializer.element_bit_size()
    }

    fn read(&self, stream: &mut BitStream) -> Result<AnimationMotion> {
        Ok(AnimationMotion {
            position: self.position_serializer.read(stream)?,
            rotation: self.rotation_serializer.read(stream)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimationTextureSerializer {
    pub layer_index_serializer: ValueSerializer<u32>,
}

impl ElementSerializer for AnimationTextureSerializer {
    type Element = AnimationTexture;
    const SERIALIZER_BIT_SIZE: usize = ValueSerializer::<u32>::serializer_bit_size();

    fn from_stream(stream: &mut BitStream) -> Result<Self> {
        Ok(Self {
            layer_index_serializer: ValueSerializer::from_stream(stream)?,
        })
    }

    fn invert(&mut self) {}

    fn element_bit_size(&self) -> usize {
        self.layer_index_serializer.element_bit_size()
    }

    fn read(&self, stream: &mut BitStream) -> Result<AnimationTexture> {
        Ok(AnimationTexture {
            layer_index: self.layer_index_serializer.read(stream)?,
        })
    }
}

/// Frames of one element kind. Serializers are stored in reverse stream order, as the C++ does.
#[derive(Clone, Debug, Default)]
pub struct AnimationFrameSet<S: ElementSerializer> {
    pub element_serializers: Vec<S>,
    pub frame_bit_count: usize,
    pub frame_count: usize,
    pub frames: Vec<Vec<S::Element>>,
}

impl<S: ElementSerializer> AnimationFrameSet<S> {
    pub fn read(
        stream: &mut BitStream,
        load_options: u32,
        frame_count: usize,
        element_count: usize,
        max_element_count: usize,
    ) -> Result<Self> {
        let local_frame_count = stream.read_u32()? as usize;
        ensure(local_frame_count == frame_count, "animation frame count mismatch")?;
        let local_element_count = stream.read_u32()? as usize;
        ensure(
            local_element_count == element_count,
            "animation element count mismatch",
        )?;
        ensure(
            local_element_count <= max_element_count,
            "animation element count above maximum",
        )?;

        let mut set = Self {
            element_serializers: vec![S::default(); max_element_count],
            frame_bit_count: 0,
            frame_count,
            frames: Vec::new(),
        };
        let mut frame_bit_count = 0usize;
        for i in 0..element_count {
            let mut serializer = S::from_stream(stream)?;
            if load_options & load_option::INVERT != 0 {
                serializer.invert();
            }
            frame_bit_count += serializer.element_bit_size();
            set.element_serializers[element_count - 1 - i] = serializer;
        }
        for _ in element_count..max_element_count {
            stream.advance(S::SERIALIZER_BIT_SIZE)?;
        }
        set.frame_bit_count = frame_bit_count;

        if load_options & load_option::ELEMENTS != 0 {
            set.frames.reserve(frame_count);
            for _ in 0..frame_count {
                let mut elements = Vec::with_capacity(element_count);
                for serializer in &set.element_serializers[..element_count] {
                    elements.push(serializer.read(stream)?);
                }
                set.frames.push(elements);
            }
        } else {
            let bits = frame_bit_count
                .checked_mul(frame_count)
                .ok_or(crate::Error::Malformed("animation frame data size overflow"))?;
            stream.advance(bits)?;
        }
        Ok(set)
    }
}

pub type PoseFrameSet = AnimationFrameSet<AnimationPoseSerializer>;
pub type MotionFrameSet = AnimationFrameSet<AnimationMotionSerializer>;
pub type TextureFrameSet = AnimationFrameSet<AnimationTextureSerializer>;

pub const MAX_POSE_ELEMENTS: usize = 72;
pub const MAX_MOTION_ELEMENTS: usize = 3;
pub const MAX_TEXTURE_ELEMENTS: usize = 5;

#[derive(Clone, Debug, Default)]
pub struct Animation {
    pub frame_count: u32,
    pub frames_per_second: f32,
    pub pose_counts: [u32; 2],
    pub motion_count: u32,
    pub texture_count: u32,
    pub pose_2_byte_offset: u32,
    pub textures_byte_offset: u32,
    pub motions_byte_offset: u32,
    pub compressed_data_bytes: Vec<u8>,
    pub pose_frame_sets: [PoseFrameSet; 2],
    pub motion_frame_set: MotionFrameSet,
    pub texture_frame_set: TextureFrameSet,
}

impl Animation {
    /// Parses a raw kAnimation block (animation blocks are not LZX compressed).
    pub fn read_data(data: &[u8], load_options: u32) -> Result<Self> {
        let mut stream = BitStream::new(data);
        let mut a = Animation {
            frame_count: stream.read_u32()?,
            frames_per_second: stream.read_f32()?,
            pose_counts: [stream.read_u32()?, stream.read_u32()?],
            motion_count: stream.read_u32()?,
            texture_count: stream.read_u32()?,
            pose_2_byte_offset: stream.read_u32()?,
            textures_byte_offset: stream.read_u32()?,
            motions_byte_offset: stream.read_u32()?,
            ..Default::default()
        };
        let compressed_data_size = stream.read_u32()? as usize;
        stream.align_to_next_byte()?;

        if load_options & load_option::COMPRESSED_DATA != 0 {
            a.compressed_data_bytes = stream.clone().take_bytes(compressed_data_size)?;
        }

        let mut slice = stream.slice(compressed_data_size * 8)?;
        ensure(
            stream.offset_bits() == stream.size_bits(),
            "trailing animation data",
        )?;
        let s = &mut slice;
        let frame_count = a.frame_count as usize;

        a.pose_frame_sets[0] = PoseFrameSet::read(
            s,
            load_options,
            frame_count,
            a.pose_counts[0] as usize,
            MAX_POSE_ELEMENTS,
        )?;
        s.align_to_next_byte()?;
        ensure(
            a.pose_2_byte_offset as usize * 8 == s.offset_bits(),
            "pose set 2 offset mismatch",
        )?;

        a.pose_frame_sets[1] = PoseFrameSet::read(
            s,
            load_options,
            frame_count,
            a.pose_counts[1] as usize,
            MAX_POSE_ELEMENTS,
        )?;
        s.align_to_next_byte()?;
        ensure(
            a.motions_byte_offset as usize * 8 == s.offset_bits(),
            "motion set offset mismatch",
        )?;

        a.motion_frame_set = MotionFrameSet::read(
            s,
            load_options,
            frame_count,
            a.motion_count as usize,
            MAX_MOTION_ELEMENTS,
        )?;
        s.align_to_next_byte()?;
        ensure(
            a.textures_byte_offset as usize * 8 == s.offset_bits(),
            "texture set offset mismatch",
        )?;

        a.texture_frame_set = TextureFrameSet::read(
            s,
            load_options,
            frame_count,
            a.texture_count as usize,
            MAX_TEXTURE_ELEMENTS,
        )?;
        s.align_to_next_byte()?;
        Ok(a)
    }

    /// Reads the first kAnimation block of a YTGR/STRB container; `None` when it has none.
    pub fn load(strb_buffer: &[u8], load_options: u32) -> Result<Option<Self>> {
        let Some(data) = strb::find_block(strb_buffer, BlockId::Animation)? else {
            return Ok(None);
        };
        Self::read_data(data, load_options).map(Some)
    }
}
