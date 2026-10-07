//! Ports Assets, Resolve and ComputeOverrides from avatar_export.cpp, the offline mirror of LoadAssetsToGuest.
//! Components come from the pack or the closet; hats swap hair to its companion, hiding and face shapes apply.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use avatar_formats::animation::load_option as anim_opt;
use avatar_formats::blend_shape::apply::{apply_blend_shape, rescue_old_body_item};
use avatar_formats::closet::is_stock_pack_id;
use avatar_formats::prop::PropLoadOptions;
use avatar_formats::skeleton::data::load_skeleton;
use avatar_formats::skeleton::scaling::{apply_scales_to_skeleton_v1, apply_scales_to_skeleton_v2, BodyType};
use avatar_formats::strb::{self, BlockId};
use avatar_formats::{Animation, AssetId, AssetPack, BlendShape, Closet, Model, Prop, Skeleton, Texture};

use crate::args::Args;
use crate::manifest::{category, color_to_float4, ComponentInfo, Float4, Metadata, WHITE};
use crate::util::{category_names, slot_label};

/// The stock pack, the optional legacy pack and the closet, read like guest_load_asset.cpp LoadAsset.
pub struct Assets {
    pub pack: AssetPack,
    pub legacy_pack: Option<AssetPack>,
    pub closet: Closet,
}

impl Assets {
    /// Closet bytes for non-stock ids, else the pack entry indexed by `id.b`.
    pub fn load_bytes(&self, id: &AssetId) -> Option<Cow<'_, [u8]>> {
        if !is_stock_pack_id(id) {
            if let Some(bytes) = self.closet.read_item_bytes(id) {
                if !bytes.is_empty() {
                    return Some(Cow::Owned(bytes));
                }
            }
        }
        self.pack.asset_data(*id).map(Cow::Borrowed)
    }

    pub fn load_model(&self, id: &AssetId) -> Option<Model> {
        let bytes = self.load_bytes(id)?;
        Model::load(&bytes, 0).ok().flatten()
    }

    pub fn load_texture(&self, id: &AssetId) -> Option<Texture> {
        let bytes = self.load_bytes(id)?;
        Texture::load(&bytes).ok().flatten()
    }

    pub fn load_blend_shape(&self, id: &AssetId) -> Option<BlendShape> {
        let bytes = self.load_bytes(id)?;
        BlendShape::load(&bytes, 0).ok().flatten()
    }

    pub fn name(&self, id: &AssetId) -> String {
        if !is_stock_pack_id(id) {
            return self.closet.find(id).map(|i| i.name.clone()).unwrap_or_default();
        }
        let index = usize::from(id.b);
        if index < self.pack.asset_infos().len() {
            return self.pack.asset_name_by_index(index);
        }
        String::new()
    }

    /// The legacy pack's first body for the gender, used to remap pre-2010 items.
    pub fn load_legacy_body(&self, gender_c: u16) -> Option<Model> {
        let legacy = self.legacy_pack.as_ref()?;
        if gender_c != 1 && gender_c != 2 {
            return None;
        }
        for (i, info) in legacy.asset_infos().iter().enumerate() {
            if info.categories & category::BODY == 0 || u16::from(info.bodies) != gender_c {
                continue;
            }
            return legacy
                .asset_data_by_index(i)
                .and_then(|data| Model::load(data, 0).ok().flatten());
        }
        None
    }
}

/// LoadPack: reads a .toc into an AssetPack.
pub fn load_pack(path: &Path) -> Option<AssetPack> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    AssetPack::load(bytes).ok()
}

/// Loads a closet directory; an unreadable index gives an empty closet like the C++ loader.
pub fn load_closet(dir: &Path) -> Closet {
    Closet::load(dir).unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedComponent {
    pub info: ComponentInfo,
    pub model: Model,
    pub from_closet: bool,
    pub name: String,
    pub slot: String,
    /// Shader usage to constant.
    pub overrides: BTreeMap<u32, Float4>,
    pub notes: Vec<String>,
}

/// The carryable with its animation shared between export and preview.
pub struct ResolvedProp {
    pub model: Model,
    pub skeleton: Skeleton,
    pub animation: Option<Rc<Animation>>,
}

pub struct Resolved {
    pub body_type: BodyType,
    pub skeleton: Skeleton,
    pub components: Vec<ResolvedComponent>,
    pub replacement_textures: [Option<Rc<Texture>>; 6],
    pub replacement_ids: [AssetId; 6],
    pub prop: Option<ResolvedProp>,
    pub prop_info: ComponentInfo,
    pub prop_name: String,
    pub log: Vec<String>,
}

impl Resolved {
    fn log(&mut self, s: String) {
        println!("  {s}");
        self.log.push(s);
    }
}

/// Reads the four little- or big-endian ARGB words of a closet item's colour table block.
pub fn custom_color_table(item_bytes: &[u8]) -> Option<[u32; 3]> {
    let table = strb::find_block(item_bytes, BlockId::CustomColorTable)
        .ok()
        .flatten()?;
    if table.len() < 4 + 3 * 8 {
        return None;
    }
    let read = |p: &[u8], le: bool| {
        let b = [p[0], p[1], p[2], p[3]];
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let le = read(table, true) <= 0xFFFF;
    let colors = [0, 1, 2].map(|i| read(&table[4 + i * 8..], le));
    colors.iter().any(|&c| c != 0).then_some(colors)
}

/// GetShaderOverrides + ApplyItemColorTable as a usage-to-constant map.
pub fn compute_overrides(metadata: &Metadata, comp: &mut ResolvedComponent, closet: &Closet) {
    let categories = u32::from(comp.info.categories);
    let ov = &mut comp.overrides;
    if categories & category::BODY != 0 {
        ov.insert(22, color_to_float4(metadata.colors[0]));
    } else if categories & category::HAIR != 0
        && categories & (category::TOP | category::BOTTOM | category::SHOES | category::GLOVES) == 0
    {
        ov.insert(22, color_to_float4(metadata.colors[1]));
    } else {
        ov.insert(22, WHITE);
    }
    if categories & category::HEAD != 0 {
        for (i, &c) in metadata.colors.iter().enumerate() {
            ov.insert(13 + i as u32, color_to_float4(c));
        }
    }
    if comp.from_closet {
        if let Some(bytes) = closet.read_item_bytes(&comp.info.asset_id) {
            if let Some(colors) = custom_color_table(&bytes) {
                for (i, &c) in colors.iter().enumerate() {
                    ov.insert(22 + i as u32, color_to_float4(c));
                }
                comp.notes.push(format!(
                    "custom color table {:08X} {:08X} {:08X}",
                    colors[0], colors[1], colors[2]
                ));
            }
        }
    }
}

const SLOT_ACCEPTED: [[u32; 2]; 6] = [
    [0x8000, 0],
    [0x2000, 0],
    [0x4000, 0],
    [0x40000, 0x10000],
    [0x40000, 0x10000],
    [0x20000, 0x10000],
];

fn slot_accepts(slot: usize, cat: u32) -> bool {
    SLOT_ACCEPTED[slot][0] == cat || SLOT_ACCEPTED[slot][1] == cat
}

fn count_blocks(bytes: &[u8], id: BlockId) -> usize {
    strb::count_blocks(bytes, id).unwrap_or(0)
}

/// Resolve: `None` only when the built-in skeleton cannot be loaded.
#[allow(clippy::needless_range_loop)]
pub fn resolve(metadata: &Metadata, args: &Args, assets: &Assets) -> Option<Resolved> {
    let category_mask = 0x1FFF & !category::PROP;
    let body_type = metadata.body_type();

    let Some(mut skeleton) = load_skeleton(args.skeleton_version as u32, 0).ok().flatten() else {
        println!("ERROR: cannot load skeleton version {}", args.skeleton_version);
        return None;
    };
    if !args.no_scale {
        if args.skeleton_version == 1 {
            apply_scales_to_skeleton_v1(
                body_type,
                metadata.weight_factor,
                metadata.height_factor,
                &mut skeleton,
            );
        } else {
            apply_scales_to_skeleton_v2(
                body_type,
                metadata.weight_factor,
                metadata.height_factor,
                &mut skeleton,
            );
        }
    }

    let mut out = Resolved {
        body_type,
        skeleton,
        components: Vec::new(),
        replacement_textures: Default::default(),
        replacement_ids: [AssetId::default(); 6],
        prop: None,
        prop_info: ComponentInfo::default(),
        prop_name: String::new(),
        log: Vec::new(),
    };

    // Manifest blend shapes: the face morphs picked in the editor.
    let mut blend_shapes: Vec<(AssetId, BlendShape)> = Vec::new();
    for id in &metadata.blend_shapes {
        if id.is_zero() {
            continue;
        }
        match assets.load_blend_shape(id) {
            Some(shape) => blend_shapes.push((*id, shape)),
            None => out.log(format!("WARN: failed to load blend shape {id}")),
        }
    }

    let mut infos: Vec<ComponentInfo> = Vec::new();
    if metadata.body_component.matches(category_mask) {
        infos.push(metadata.body_component);
    }
    if metadata.head_component.matches(category_mask) {
        infos.push(metadata.head_component);
    }
    infos.extend(metadata.components.iter().filter(|c| c.matches(category_mask)));

    let pack_infos = assets.pack.asset_infos();

    // A worn hat swaps the hair to its "(Hat)" companion entry.
    if infos.iter().any(|i| i.matches(category::HAT)) {
        for info in infos.iter_mut() {
            if !info.matches(category::HAIR) || !is_stock_pack_id(&info.asset_id) {
                continue;
            }
            let hair_index = usize::from(info.asset_id.b);
            if hair_index >= pack_infos.len() {
                continue;
            }
            let id0 = pack_infos[hair_index].asset_ids[0];
            let partner = usize::from(id0.b);
            if id0.is_zero() || partner == hair_index || partner >= pack_infos.len() {
                continue;
            }
            let line = format!(
                "hat worn: hair {} -> companion pack index {partner}",
                info.asset_id
            );
            out.log(line);
            info.asset_id.b = partner as u16;
        }
    }

    // Body-hiding shapes.
    for info in &infos {
        if !is_stock_pack_id(&info.asset_id) {
            let Some(item_bytes) = assets.closet.read_item_bytes(&info.asset_id) else {
                continue;
            };
            if item_bytes.is_empty() {
                continue;
            }
            let count = count_blocks(&item_bytes, BlockId::ShapeOverrides);
            for occ in 0..count {
                let Ok(block) = strb::block_n(&item_bytes, BlockId::ShapeOverrides, occ) else {
                    break;
                };
                let Ok(mut shape) = BlendShape::read_data(block, 0) else {
                    out.log(format!(
                        "WARN: bad embedded hiding shape in closet item {}",
                        info.asset_id
                    ));
                    continue;
                };
                if shape.index_patch.original_asset_id.a == 2 {
                    shape.index_patch.original_asset_id.b = 0;
                }
                if shape.vertex_patch.original_asset_id.a == 2 {
                    shape.vertex_patch.original_asset_id.b = 0;
                }
                out.log(format!(
                    "closet item {} -> embedded hiding shape {}/{} (target {})",
                    info.asset_id,
                    occ + 1,
                    count,
                    shape.index_patch.original_asset_id
                ));
                blend_shapes.push((info.asset_id, shape));
            }
            continue;
        }
        let index = usize::from(info.asset_id.b);
        if index >= pack_infos.len() {
            continue;
        }
        let id0 = pack_infos[index].asset_ids[0];
        if id0.is_zero() || id0.a != 0x0100_0000 || usize::from(id0.b) >= pack_infos.len() {
            continue;
        }
        match assets.load_blend_shape(&id0) {
            Some(shape) => {
                out.log(format!("component {} -> body-hiding shape {id0}", info.asset_id));
                blend_shapes.push((id0, shape));
            }
            None => out.log(format!("WARN: failed to load hiding shape {id0}")),
        }
    }

    // Component models; shape-only assets join the blend-shape set instead.
    for info in &infos {
        if let Some(raw) = assets.load_bytes(&info.asset_id) {
            if count_blocks(&raw, BlockId::ShapeOverrides) > 0 && count_blocks(&raw, BlockId::Model) == 0 {
                match BlendShape::load(&raw, 0).ok().flatten() {
                    Some(shape) => {
                        out.log(format!(
                            "component {} is a shape asset (face morph)",
                            info.asset_id
                        ));
                        blend_shapes.push((info.asset_id, shape));
                    }
                    None => out.log(format!(
                        "WARN: failed to load component blend shape {}",
                        info.asset_id
                    )),
                }
                continue;
            }
        }
        let mut comp_info = *info;
        let mut model = assets.load_model(&info.asset_id);
        if model.is_none() {
            out.log(format!(
                "WARN: failed to load {} ({}), trying fallback",
                info.asset_id,
                category_names(u32::from(info.categories))
            ));
            if let Some(cand) = metadata
                .fallback_components
                .iter()
                .find(|c| c.categories == info.categories && !c.asset_id.is_zero())
            {
                model = assets.load_model(&cand.asset_id);
                comp_info = *cand;
            }
        }
        let Some(model) = model else {
            out.log(format!("ERROR: no model for component {}", info.asset_id));
            continue;
        };
        out.components.push(ResolvedComponent {
            info: comp_info,
            model,
            from_closet: !is_stock_pack_id(&comp_info.asset_id),
            name: assets.name(&comp_info.asset_id),
            slot: slot_label(u32::from(comp_info.categories)),
            overrides: BTreeMap::new(),
            notes: Vec::new(),
        });
    }

    // Replacement (face) textures: fixed slot meanings, misfiled entries moved to a free slot that takes them.
    let mut loaded: [Option<Texture>; 6] = Default::default();
    let mut slot_source = [usize::MAX; 6];
    for i in 0..6 {
        let ti = &metadata.textures[i];
        if ti.asset_id.is_zero() {
            continue;
        }
        let Some(tex) = assets.load_texture(&ti.asset_id) else {
            out.log(format!(
                "WARN: failed to load replacement texture {}",
                ti.asset_id
            ));
            continue;
        };
        if slot_accepts(i, ti.asset_id.a) {
            out.replacement_textures[i] = Some(Rc::new(tex));
            out.replacement_ids[i] = ti.asset_id;
            slot_source[i] = i;
        } else {
            loaded[i] = Some(tex);
        }
    }
    for i in 0..6 {
        if loaded[i].is_none() {
            continue;
        }
        let cat = metadata.textures[i].asset_id.a;
        for s in 0..6 {
            if out.replacement_textures[s].is_none() && slot_accepts(s, cat) {
                out.log(format!(
                    "replacement texture {} reslotted {i} -> {s}",
                    metadata.textures[i].asset_id
                ));
                out.replacement_textures[s] = loaded[i].take().map(Rc::new);
                out.replacement_ids[s] = metadata.textures[i].asset_id;
                slot_source[s] = i;
                break;
            }
        }
    }
    // Eye-shadow overlay auto-pair, skipped for the opaque-black "none" tint.
    let eyeshadow_none = metadata.colors[5] & 0xFF_FFFF == 0;
    if !eyeshadow_none
        && out.replacement_textures[4].is_none()
        && out.replacement_textures[1].is_some()
        && slot_source[1] != usize::MAX
        && is_stock_pack_id(&metadata.textures[slot_source[1]].asset_id)
    {
        let eye_id = metadata.textures[slot_source[1]].asset_id;
        let eye_index = usize::from(eye_id.b);
        if eye_index < pack_infos.len() {
            let id0 = pack_infos[eye_index].asset_ids[0];
            if !id0.is_zero() && id0.a == 0x40000 && usize::from(id0.b) < pack_infos.len() {
                if let Some(shadow) = assets.load_texture(&id0) {
                    out.log(format!("eyes {eye_id} -> paired eye-shadow overlay {id0}"));
                    out.replacement_textures[4] = Some(Rc::new(shadow));
                    out.replacement_ids[4] = id0;
                }
            }
        }
    }

    // Old-body item rescue, then the blend-shape apply loop.
    for (owner_id, shape) in blend_shapes.iter_mut() {
        if shape.vertex_patch.original_asset_id.a != 2 {
            continue;
        }
        let Some(body_index) = out
            .components
            .iter()
            .position(|c| shape.matches(&c.info.asset_id))
        else {
            continue;
        };
        let owner_index = out.components.iter().rposition(|c| c.info.asset_id == *owner_id);
        let legacy = assets.load_legacy_body(shape.vertex_patch.original_asset_id.c);
        let body_model = out.components[body_index].model.clone();
        let owner_model = owner_index.map(|i| &mut out.components[i].model);
        rescue_old_body_item(shape, &body_model, owner_model, legacy.as_ref());
    }
    for ci in 0..out.components.len() {
        for (shape_id, shape) in &blend_shapes {
            let comp_id = out.components[ci].info.asset_id;
            if !shape.matches(&comp_id) {
                continue;
            }
            if !apply_blend_shape(shape, &comp_id, &mut out.components[ci].model) {
                out.log(format!(
                    "WARN: failed to apply blend shape {shape_id} to {comp_id}"
                ));
            } else {
                let slot = out.components[ci].slot.clone();
                out.log(format!("applied blend shape {shape_id} to {comp_id} ({slot})"));
            }
        }
    }

    for comp in out.components.iter_mut() {
        compute_overrides(metadata, comp, &assets.closet);
    }

    // Carryable.
    if args.want_prop {
        if let Some(c) = metadata.components.iter().find(|c| c.matches(category::PROP)) {
            out.prop_info = *c;
        }
        if !out.prop_info.asset_id.is_zero() {
            let prop_id = out.prop_info.asset_id;
            if let Some(raw) = assets.load_bytes(&prop_id) {
                let opts = PropLoadOptions {
                    model: 0,
                    skeleton: 0,
                    animation: anim_opt::ELEMENTS,
                    blend_shape: 0,
                };
                let prop = Prop::load(&raw, &opts).ok().flatten();
                out.prop_name = assets.name(&prop_id);
                match prop {
                    Some(p) => {
                        let line = format!("carryable {prop_id} '{}'", out.prop_name);
                        out.log(line);
                        out.prop = Some(ResolvedProp {
                            model: p.model,
                            skeleton: p.skeleton,
                            animation: p.animation.map(Rc::new),
                        });
                    }
                    None => out.log(format!("WARN: failed to load carryable {prop_id}")),
                }
            }
        }
    }
    Some(out)
}
