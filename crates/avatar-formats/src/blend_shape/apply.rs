// Port of xenia/kernel/xam/avatars/blend_shape_apply.{h,cpp}.

use glam::Vec3;

use super::{target_matches, BlendShape, BlendShapeIndexPatch, BlendShapeVertexPatch};
use crate::asset_pack::AssetId;
use crate::model::Model;

const MAX_TUCK_DISTANCE: f32 = 0.04;

fn apply_index_patch(patch: &BlendShapeIndexPatch, model: &mut Model) {
    let batch_count = model.triangle_batches.len();
    let mut offsets = Vec::with_capacity(batch_count + 1);
    let mut offset = 0usize;
    for batch in &model.triangle_batches {
        offsets.push(offset);
        offset += batch.triangle_count as usize;
    }
    offsets.push(offset);

    for &raw in &patch.indices {
        let index = raw as usize;
        for j in 1..=batch_count {
            if index < offsets[j] {
                let j = j - 1;
                let local = index - offsets[j];
                let indices = &mut model.triangle_batches[j].indices;
                if local * 3 + 2 < indices.len() {
                    let value = indices[local * 3];
                    indices[local * 3 + 1] = value;
                    indices[local * 3 + 2] = value;
                }
                break;
            }
        }
    }
}

fn vertex_offsets(model: &Model) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(model.triangle_batches.len() + 1);
    let mut offset = 0usize;
    for batch in &model.triangle_batches {
        offsets.push(offset);
        offset += batch.vertices.len() * batch.vertex_size as usize;
    }
    offsets.push(offset);
    offsets
}

fn apply_vertex_patch(patch: &BlendShapeVertexPatch, model: &mut Model) {
    let offsets = vertex_offsets(model);
    // Body tucks only nudge rim vertices; a far tuck comes from an older body revision and is skipped.
    let clamp_tucks = patch.original_asset_id.a == 2;
    let batch_count = model.triangle_batches.len();
    for vertex_patch in &patch.vertices {
        let original_offset = vertex_patch.original_offset as usize;
        for j in 1..=batch_count {
            if original_offset < offsets[j] {
                let j = j - 1;
                let batch = &mut model.triangle_batches[j];
                let Some(index) = (original_offset - offsets[j]).checked_div(batch.vertex_size as usize)
                else {
                    break;
                };
                let Some(vertex) = batch.vertices.get_mut(index) else {
                    break;
                };
                if clamp_tucks {
                    let dx = vertex.position.x - vertex_patch.position.x;
                    let dy = vertex.position.y - vertex_patch.position.y;
                    let dz = vertex.position.z - vertex_patch.position.z;
                    if dx * dx + dy * dy + dz * dz > MAX_TUCK_DISTANCE * MAX_TUCK_DISTANCE {
                        break;
                    }
                }
                vertex.position = vertex_patch.position;
                vertex.normal = vertex_patch.normal;
                vertex.blend_weight = vertex_patch.blend_weight;
                vertex.blend_indices = vertex_patch.blend_indices;
                vertex.color = vertex_patch.color;
                break;
            }
        }
    }
}

/// All positions in batch order; `None` when the batch layout differs from `like`.
fn flatten_positions(model: &Model, like: Option<&Model>) -> Option<Vec<Vec3>> {
    if let Some(like) = like {
        if model.triangle_batches.len() != like.triangle_batches.len() {
            return None;
        }
    }
    let mut out = Vec::new();
    for (i, batch) in model.triangle_batches.iter().enumerate() {
        if let Some(like) = like {
            let other = &like.triangle_batches[i];
            if batch.vertices.len() != other.vertices.len() || batch.vertex_size != other.vertex_size {
                return None;
            }
        }
        out.extend(batch.vertices.iter().map(|v| v.position));
    }
    Some(out)
}

fn distance(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Remaps a pre-Fall-2010 item and its tucks into the current body's space: per vertex via the
/// same-topology legacy body when given, else by a per-axis affine fit.
pub fn rescue_old_body_item(
    blend_shape: &mut BlendShape,
    body_model: &Model,
    mut item_model: Option<&mut Model>,
    legacy_body_model: Option<&Model>,
) -> bool {
    let patch = &mut blend_shape.vertex_patch;
    if patch.original_asset_id.a != 2 || patch.vertices.len() < 4 {
        return false;
    }
    let mut total_vertex_buffer_size: usize = body_model
        .triangle_batches
        .iter()
        .map(|b| b.vertices.len() * b.vertex_size as usize)
        .sum();
    total_vertex_buffer_size = (total_vertex_buffer_size + 127) & !127;
    if total_vertex_buffer_size != patch.total_buffer_size as usize {
        return false;
    }

    let offsets = vertex_offsets(body_model);
    let batch_count = body_model.triangle_batches.len();
    let mut pairs: Vec<(Vec3, Vec3)> = Vec::new();
    let mut sum_disp = 0.0f32;
    for pv in &patch.vertices {
        let off = pv.original_offset as usize;
        for j in 1..=batch_count {
            if off < offsets[j] {
                let j = j - 1;
                let batch = &body_model.triangle_batches[j];
                if let Some(vi) = (off - offsets[j]).checked_div(batch.vertex_size as usize) {
                    if vi < batch.vertices.len() {
                        let bp = batch.vertices[vi].position;
                        pairs.push((bp, pv.position));
                        sum_disp += distance(bp, pv.position);
                    }
                }
                break;
            }
        }
    }
    if pairs.len() < 4 {
        return false;
    }
    let mean_disp = sum_disp / pairs.len() as f32;
    // A real tuck stays close to the body.
    if mean_disp <= 0.04 {
        return false;
    }

    if let Some(legacy) = legacy_body_model {
        if batch_count == 1 {
            if let (Some(cur_pos), Some(old_pos)) = (
                flatten_positions(body_model, None),
                flatten_positions(legacy, Some(body_model)),
            ) {
                let vertex_size = body_model.triangle_batches[0].vertex_size as usize;
                let mut old_sum = 0.0f32;
                let mut old_n = 0usize;
                for pv in patch.vertices.iter() {
                    let Some(vi) = (pv.original_offset as usize).checked_div(vertex_size) else {
                        continue;
                    };
                    if vi >= old_pos.len() {
                        continue;
                    }
                    old_sum += distance(old_pos[vi], pv.position);
                    old_n += 1;
                }
                let old_mean = if old_n != 0 { old_sum / old_n as f32 } else { 1e9 };
                if old_mean <= 0.03 {
                    for pv in patch.vertices.iter_mut() {
                        let Some(vi) = (pv.original_offset as usize).checked_div(vertex_size) else {
                            continue;
                        };
                        if vi >= old_pos.len() || vi >= cur_pos.len() {
                            continue;
                        }
                        pv.position.x = cur_pos[vi].x + (pv.position.x - old_pos[vi].x);
                        pv.position.y = cur_pos[vi].y + (pv.position.y - old_pos[vi].y);
                        pv.position.z = cur_pos[vi].z + (pv.position.z - old_pos[vi].z);
                    }
                    if let Some(item) = item_model.as_deref_mut() {
                        if !old_pos.is_empty() {
                            warp_by_nearest(item, &cur_pos, &old_pos);
                        }
                    }
                    return true;
                }
            }
        }
    }

    // Per-axis least squares: patch = scale * body + offset; flat axes fall back to a pure offset.
    let mut scale = [0.0f32; 3];
    let mut offset = [0.0f32; 3];
    for axis in 0..3 {
        let get = |v: &Vec3| match axis {
            0 => v.x,
            1 => v.y,
            _ => v.z,
        };
        let mut mb = 0.0f32;
        let mut mp = 0.0f32;
        for (b, p) in &pairs {
            mb += get(b);
            mp += get(p);
        }
        mb /= pairs.len() as f32;
        mp /= pairs.len() as f32;
        let mut var = 0.0f32;
        let mut cov = 0.0f32;
        for (b, p) in &pairs {
            let db = get(b) - mb;
            var += db * db;
            cov += db * (get(p) - mp);
        }
        let mut s = if var > 1e-6 { cov / var } else { 1.0 };
        if s < 0.7 || s > 1.3 {
            s = 1.0;
        }
        scale[axis] = s;
        offset[axis] = mp - s * mb;
    }

    let unmap = |p: &mut Vec3| {
        p.x = (p.x - offset[0]) / scale[0];
        p.y = (p.y - offset[1]) / scale[1];
        p.z = (p.z - offset[2]) / scale[2];
    };
    for pv in patch.vertices.iter_mut() {
        unmap(&mut pv.position);
    }
    if let Some(item) = item_model {
        for batch in item.triangle_batches.iter_mut() {
            for v in batch.vertices.iter_mut() {
                unmap(&mut v.position);
            }
        }
    }
    true
}

/// Moves each item vertex by the inverse-distance-weighted displacement of its 3 nearest legacy-body vertices.
fn warp_by_nearest(item: &mut Model, cur_pos: &[Vec3], old_pos: &[Vec3]) {
    for batch in item.triangle_batches.iter_mut() {
        for v in batch.vertices.iter_mut() {
            let (mut n0, mut n1, mut n2) = (0usize, 0usize, 0usize);
            let (mut d0, mut d1, mut d2) = (1e9f32, 1e9f32, 1e9f32);
            for (i, o) in old_pos.iter().enumerate() {
                let dx = o.x - v.position.x;
                let dy = o.y - v.position.y;
                let dz = o.z - v.position.z;
                let d = dx * dx + dy * dy + dz * dz;
                if d < d0 {
                    d2 = d1;
                    n2 = n1;
                    d1 = d0;
                    n1 = n0;
                    d0 = d;
                    n0 = i;
                } else if d < d1 {
                    d2 = d1;
                    n2 = n1;
                    d1 = d;
                    n1 = i;
                } else if d < d2 {
                    d2 = d;
                    n2 = i;
                }
            }
            let w0 = 1.0 / (d0.sqrt() + 1e-4);
            let w1 = 1.0 / (d1.sqrt() + 1e-4);
            let w2 = 1.0 / (d2.sqrt() + 1e-4);
            let wsum = w0 + w1 + w2;
            v.position.x += (w0 * (cur_pos[n0].x - old_pos[n0].x)
                + w1 * (cur_pos[n1].x - old_pos[n1].x)
                + w2 * (cur_pos[n2].x - old_pos[n2].x))
                / wsum;
            v.position.y += (w0 * (cur_pos[n0].y - old_pos[n0].y)
                + w1 * (cur_pos[n1].y - old_pos[n1].y)
                + w2 * (cur_pos[n2].y - old_pos[n2].y))
                / wsum;
            v.position.z += (w0 * (cur_pos[n0].z - old_pos[n0].z)
                + w1 * (cur_pos[n1].z - old_pos[n1].z)
                + w2 * (cur_pos[n2].z - old_pos[n2].z))
                / wsum;
        }
    }
}

/// Collapses hidden triangles and moves tucked vertices; false when the shape targets another model.
pub fn apply_blend_shape(blend_shape: &BlendShape, model_asset_id: &AssetId, model: &mut Model) -> bool {
    if !target_matches(&blend_shape.index_patch.original_asset_id, model_asset_id)
        || !target_matches(&blend_shape.vertex_patch.original_asset_id, model_asset_id)
    {
        return false;
    }

    let mut total_index_buffer_size = 0usize;
    let mut total_vertex_buffer_size = 0usize;
    for batch in &model.triangle_batches {
        total_index_buffer_size += batch.triangle_count as usize * 6;
        total_vertex_buffer_size += batch.vertices.len() * batch.vertex_size as usize;
    }
    total_vertex_buffer_size = (total_vertex_buffer_size + 127) & !127;

    if total_index_buffer_size != blend_shape.index_patch.total_buffer_size as usize
        || total_vertex_buffer_size != blend_shape.vertex_patch.total_buffer_size as usize
    {
        return false;
    }

    apply_index_patch(&blend_shape.index_patch, model);
    apply_vertex_patch(&blend_shape.vertex_patch, model);
    true
}
