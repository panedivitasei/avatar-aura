// Port of xenia/kernel/xam/avatars/prop.{h,cpp}.

use crate::animation::Animation;
use crate::asset_pack::AssetId;
use crate::blend_shape::{apply::apply_blend_shape, BlendShape};
use crate::error::Result;
use crate::model::Model;
use crate::skeleton::Skeleton;

/// Per-part load flags; see each module's `load_option`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropLoadOptions {
    pub model: u32,
    pub skeleton: u32,
    pub animation: u32,
    pub blend_shape: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Prop {
    pub model: Model,
    pub skeleton: Skeleton,
    pub animation: Option<Animation>,
}

impl Prop {
    /// A prop needs a model and a skeleton; `None` when either block is absent.
    pub fn load(strb_buffer: &[u8], load_options: &PropLoadOptions) -> Result<Option<Self>> {
        let Some(mut model) = Model::load(strb_buffer, load_options.model)? else {
            return Ok(None);
        };
        let Some(skeleton) = Skeleton::load(strb_buffer, load_options.skeleton)? else {
            return Ok(None);
        };
        let animation = Animation::load(strb_buffer, load_options.animation)?;
        // Untested upstream: a prop's own shape targets the zero asset id.
        if let Some(blend_shape) = BlendShape::load(strb_buffer, load_options.blend_shape)? {
            apply_blend_shape(&blend_shape, &AssetId::default(), &mut model);
        }
        Ok(Some(Prop {
            model,
            skeleton,
            animation,
        }))
    }
}
