// Port of mode 5 (raw STRB/YTGR blob, RunRawStrbMode) of avatarextract/main.cpp.
//! Summarizes a marketplace item's asset_v2.bin and decodes its model to OBJ.

use std::io::Write;
use std::path::Path;

use avatar_formats::model::load_option as model_option;
use avatar_formats::strb::BlockId;
use avatar_formats::{BlendShape, Model};

use crate::closet_import::{count_blocks, detect_item_bodies};
use crate::objwrite::write_model;

/// Mode 5: prints the import verdict, block counts and first shape, then writes `item.obj` when a model decodes.
pub fn run_raw_blob(in_path: &Path, out_dir: &Path, log: &mut dyn Write) -> i32 {
    let bytes = match std::fs::read(in_path) {
        Ok(b) => b,
        Err(_) => {
            say!(log, "ERROR: cannot open: {}\n", in_path.display());
            return 1;
        }
    };
    if bytes.is_empty() {
        say!(log, "ERROR: bad read.\n");
        return 1;
    }
    say!(log, "=== raw STRB/YTGR blob: {} bytes ===\n", bytes.len());
    // printf evaluates the verdict first, so a fault note precedes the verdict line.
    let verdict = detect_item_bodies(&bytes, log);
    say!(
        log,
        "  closet-import bodies verdict: {verdict} (0=malformed 1=male 2=female 3=both)\n"
    );
    say!(
        log,
        "  blocks: model={} texture={} skeleton={} animation={} shape={}\n",
        count_blocks(&bytes, BlockId::Model),
        count_blocks(&bytes, BlockId::Texture),
        count_blocks(&bytes, BlockId::Skeleton),
        count_blocks(&bytes, BlockId::Animation),
        count_blocks(&bytes, BlockId::ShapeOverrides)
    );
    if let Ok(Some(shape)) = BlendShape::load(&bytes, avatar_formats::blend_shape::load_option::NONE) {
        let ip = &shape.index_patch;
        let vp = &shape.vertex_patch;
        say!(
            log,
            "  shape: index target={} ({} idx, tbs={}), vertex target={} ({} verts, tbs={})\n",
            ip.original_asset_id,
            ip.indices.len(),
            ip.total_buffer_size,
            vp.original_asset_id,
            vp.vertices.len(),
            vp.total_buffer_size
        );
    }
    let Ok(Some(model)) = Model::load(&bytes, model_option::NONE) else {
        say!(log, "  (no decodable model)\n");
        return 0;
    };
    say!(
        log,
        "  model: {} batches, textures={}\n",
        model.triangle_batches.len(),
        model.textures.len()
    );
    if let Some(t) = write_model(&model, out_dir, "item", log) {
        say!(
            log,
            "  wrote OBJ: {} verts, {} tris -> {}\n",
            t.vertices,
            t.triangles,
            out_dir.display()
        );
    }
    0
}
