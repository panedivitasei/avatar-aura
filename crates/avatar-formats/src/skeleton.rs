// Port of xenia/kernel/xam/avatars/skeleton.{h,cpp}.

pub mod data;
pub mod scaling;

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::bits::BitStream;
use crate::compression;
use crate::error::{ensure, Result};
use crate::serializers::{QuaternionSerializer, VectorSerializer};
use crate::strb::{self, BlockId};

/// Skeleton load flags (SkeletonLoadOption in the C++).
pub mod load_option {
    pub const NONE: u32 = 0;
    pub const INVERT: u32 = 1 << 0;
}

pub const INVALID_INDEX: u8 = 255;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JointBindPose {
    pub position: Vec3,
    pub rotation: Quat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JointPose {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

/// `bindpose` is model space; `pose` is the parent-relative rest pose built by `initialize`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Joint {
    pub parent_index: u8,
    pub first_child_index: u8,
    pub next_index: u8,
    pub bindpose: JointBindPose,
    pub pose: JointPose,
}

#[derive(Clone, Copy, Debug, Default)]
struct JointSerializer {
    position_serializer: VectorSerializer,
    rotation_serializer: QuaternionSerializer,
}

impl JointSerializer {
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

    fn read(&self, stream: &mut BitStream) -> Result<Joint> {
        let parent_index = stream.read_u8()?;
        let position = self.position_serializer.read(stream)?;
        let rotation = self.rotation_serializer.read(stream)?;
        Ok(Joint {
            parent_index,
            bindpose: JointBindPose { position, rotation },
            ..Default::default()
        })
    }
}

/// glm::mat4_cast with the translation column set.
fn rotation_translation(q: Quat, p: Vec3) -> Mat4 {
    let mut m = Mat4::from_quat(q);
    m.w_axis = Vec4::new(p.x, p.y, p.z, 1.0);
    m
}

/// glm::quat_cast: picks the largest component and keeps it positive.
pub(crate) fn glm_quat_cast(m: &Mat4) -> Quat {
    let (m00, m01, m02) = (m.x_axis.x, m.x_axis.y, m.x_axis.z);
    let (m10, m11, m12) = (m.y_axis.x, m.y_axis.y, m.y_axis.z);
    let (m20, m21, m22) = (m.z_axis.x, m.z_axis.y, m.z_axis.z);
    let four_x = m00 - m11 - m22;
    let four_y = m11 - m00 - m22;
    let four_z = m22 - m00 - m11;
    let four_w = m00 + m11 + m22;
    let mut biggest_index = 0;
    let mut four_biggest = four_w;
    if four_x > four_biggest {
        four_biggest = four_x;
        biggest_index = 1;
    }
    if four_y > four_biggest {
        four_biggest = four_y;
        biggest_index = 2;
    }
    if four_z > four_biggest {
        four_biggest = four_z;
        biggest_index = 3;
    }
    let biggest_val = (four_biggest + 1.0).sqrt() * 0.5;
    let mult = 0.25 / biggest_val;
    match biggest_index {
        0 => Quat::from_xyzw(
            (m12 - m21) * mult,
            (m20 - m02) * mult,
            (m01 - m10) * mult,
            biggest_val,
        ),
        1 => Quat::from_xyzw(
            biggest_val,
            (m01 + m10) * mult,
            (m20 + m02) * mult,
            (m12 - m21) * mult,
        ),
        2 => Quat::from_xyzw(
            (m01 + m10) * mult,
            biggest_val,
            (m12 + m21) * mult,
            (m20 - m02) * mult,
        ),
        _ => Quat::from_xyzw(
            (m20 + m02) * mult,
            (m12 + m21) * mult,
            biggest_val,
            (m01 - m10) * mult,
        ),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
}

impl Skeleton {
    fn initialize(&mut self) -> Result<()> {
        let joints = &mut self.joints;
        ensure(!joints.is_empty() && joints.len() < 255, "skeleton joint count")?;
        let joint_count = joints.len();
        ensure(
            joints[1..]
                .iter()
                .all(|j| (j.parent_index as usize) < joint_count),
            "joint parent index out of range",
        )?;

        for joint in joints.iter_mut() {
            joint.first_child_index = INVALID_INDEX;
            joint.next_index = INVALID_INDEX;
            joint.pose.position = joint.bindpose.position;
            joint.pose.rotation = joint.bindpose.rotation;
            joint.pose.scale = Vec3::ONE;
        }

        for i in (1..joint_count).rev() {
            let joint = joints[i];
            let parent = joints[joint.parent_index as usize];
            let matrix_a = rotation_translation(joint.bindpose.rotation, joint.bindpose.position);
            // The parent quaternion keeps its real w; mirrored bones sit near w = 0.
            let matrix = rotation_translation(parent.bindpose.rotation, parent.bindpose.position);
            let matrix_b = matrix.inverse();
            let matrix2 = matrix_b * matrix_a;
            let position = matrix2.w_axis;
            let rotation = glm_quat_cast(&matrix2);
            let pose = &mut joints[i].pose;
            pose.position = Vec3::new(position.x, position.y, position.z);
            pose.rotation = rotation;
        }

        {
            let joint = &mut joints[0];
            joint.pose.position = joint.bindpose.position;
            joint.pose.rotation = joint.bindpose.rotation;
        }

        for parent_index in 0..joint_count {
            let Some(first_child_index) =
                (0..joint_count).find(|&c| joints[c].parent_index as usize == parent_index)
            else {
                continue;
            };
            joints[parent_index].first_child_index = first_child_index as u8;
            let mut prev_index = first_child_index;
            for next_index in first_child_index + 1..joint_count {
                if joints[next_index].parent_index as usize == parent_index {
                    joints[prev_index].next_index = next_index as u8;
                    prev_index = next_index;
                }
            }
        }
        Ok(())
    }

    /// Parses an uncompressed skeleton payload.
    pub fn read_data(data: &[u8], load_options: u32) -> Result<Self> {
        let mut stream = BitStream::new(data);
        let joint_count = stream.read_u32()? as usize;
        ensure(joint_count <= 72, "joint count above 72")?;

        let mut serializer = JointSerializer::from_stream(&mut stream)?;
        if load_options & load_option::INVERT != 0 {
            serializer.invert();
        }
        let mut skeleton = Skeleton {
            joints: Vec::with_capacity(joint_count),
        };
        for _ in 0..joint_count {
            skeleton.joints.push(serializer.read(&mut stream)?);
        }

        stream.align_to_next_byte()?;
        ensure(
            stream.offset_bits() == stream.size_bits(),
            "trailing skeleton data",
        )?;

        skeleton.initialize()?;
        Ok(skeleton)
    }

    /// Decodes the first kSkeleton block of a YTGR/STRB container; `None` when it has none.
    pub fn load(strb_buffer: &[u8], load_options: u32) -> Result<Option<Self>> {
        let Some(compressed) = strb::find_block(strb_buffer, BlockId::Skeleton)? else {
            return Ok(None);
        };
        let data = compression::decompress(compressed)?;
        Self::read_data(&data, load_options).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_skeletons_load() {
        let nxe = data::load_skeleton(1, load_option::NONE).unwrap().unwrap();
        assert_eq!(nxe.joints.len(), 71);
        assert_eq!(nxe.joints[0].first_child_index, 1);
        let kinect = data::load_skeleton(2, load_option::INVERT).unwrap().unwrap();
        assert!(!kinect.joints.is_empty());
        assert!(data::load_skeleton(3, 0).unwrap().is_none());
    }

    #[test]
    fn quat_cast_round_trips() {
        let q = Quat::from_xyzw(0.1, -0.7, 0.2, 0.6).normalize();
        let back = glm_quat_cast(&Mat4::from_quat(q));
        assert!((back.dot(q).abs() - 1.0).abs() < 1e-6);
    }
}
