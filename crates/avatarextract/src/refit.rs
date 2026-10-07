// Port of refit.cpp (--refit-female) of avatarextract.
//! Appends a female kShapeOverrides block to a male-only wearable, keeping the male one.
//! The stock bodies differ in topology, so a female triangle hides when its centroid's nearest male centroid
//! is hidden, and a female vertex copies its nearest male vertex's tuck unless the delta is implausibly large.

use std::io::Write;
use std::path::Path;

use avatar_formats::blend_shape::apply::apply_blend_shape;
use avatar_formats::blend_shape::{load_option as shape_option, BlendShapeVertex};
use avatar_formats::model::load_option as model_option;
use avatar_formats::strb::BlockId;
use avatar_formats::{AssetId, AssetPack, BlendShape, Model};
use glam::Vec3;

use crate::closet_import::{count_blocks, find_block_n};

/// LSB-first bit writer, the mirror of BitStream reads.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bit: u32,
}

impl BitWriter {
    pub fn write(&mut self, value: u64, bits: u32) {
        for i in 0..bits {
            if self.bit == 0 {
                self.bytes.push(0);
            }
            if i < 64 && (value >> i) & 1 != 0 {
                if let Some(last) = self.bytes.last_mut() {
                    *last |= 1 << self.bit;
                }
            }
            self.bit = (self.bit + 1) & 7;
        }
    }
    pub fn u32(&mut self, v: u32) {
        self.write(u64::from(v), 32);
    }
    pub fn u16(&mut self, v: u16) {
        self.write(u64::from(v), 16);
    }
    pub fn u8(&mut self, v: u8) {
        self.write(u64::from(v), 8);
    }
    pub fn f32(&mut self, v: f32) {
        self.write(u64::from(v.to_bits()), 32);
    }
    pub fn align(&mut self) {
        self.bit = 0;
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

fn write_asset_id(w: &mut BitWriter, id: &AssetId) {
    w.u32(id.a);
    w.u16(id.b);
    w.u16(id.c);
    for b in id.d {
        w.u8(b);
    }
}

fn bits_for(range: u64) -> u32 {
    let mut n = 1;
    while n < 63 && (range >> n) != 0 {
        n += 1;
    }
    n
}

/// ValueSerializer encoder: base = min, bit count covers max - min.
#[derive(Debug, Clone, Copy)]
struct ValueEnc {
    base: u32,
    bits: u32,
}

impl ValueEnc {
    fn fit(values: impl IntoIterator<Item = u32>) -> Self {
        let mut lo = u32::MAX;
        let mut hi = 0u32;
        let mut any = false;
        for v in values {
            lo = lo.min(v);
            hi = hi.max(v);
            any = true;
        }
        if !any {
            lo = 0;
            hi = 0;
        }
        Self {
            base: lo,
            bits: bits_for(u64::from(hi).wrapping_sub(u64::from(lo))),
        }
    }
    fn header(&self, w: &mut BitWriter) {
        w.u32(self.base);
        w.u32(self.bits);
    }
    fn put(&self, w: &mut BitWriter, v: u32) {
        w.write(u64::from(v.wrapping_sub(self.base)), self.bits);
    }
}

/// VectorSerializer encoder (the parity-lattice quantizer of serializers.h).
#[derive(Debug, Clone, Copy)]
struct VectorEnc {
    q: f32,
    bx: f32,
    by: f32,
    bz: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    nx: u32,
    ny: u32,
    nz: u32,
}

impl VectorEnc {
    fn fit(pts: &[Vec3]) -> Self {
        // quant_radius 0.0002 gives about 0.4 mm steps.
        let q = 0.0002f32;
        let dx = q * 2.0;
        let dy = q * (2.0f32 / 3.0f32) * 6.0f32.sqrt();
        let dz = q * 3.0f32.sqrt();
        let (mut lo, mut hi) = ([1e9f32; 3], [-1e9f32; 3]);
        for p in pts {
            for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                if v < lo[k] {
                    lo[k] = v;
                }
                if hi[k] < v {
                    hi[k] = v;
                }
            }
        }
        if pts.is_empty() {
            lo = [0.0; 3];
            hi = [0.0; 3];
        }
        // A margin of a few steps below the minimum keeps every index non-negative.
        let bx = lo[0] - 4.0 * dx;
        let by = lo[1] - 4.0 * dy;
        let bz = lo[2] - 4.0 * dz;
        let n = |hi: f32, b: f32, d: f32| 30u32.min(bits_for(((hi - b) / d) as u64 + 8));
        Self {
            q,
            bx,
            by,
            bz,
            dx,
            dy,
            dz,
            nx: n(hi[0], bx, dx),
            ny: n(hi[1], by, dy),
            nz: n(hi[2], bz, dz),
        }
    }
    fn header(&self, w: &mut BitWriter) {
        w.f32(self.q);
        w.f32(self.bx);
        w.f32(self.by);
        w.f32(self.bz);
        w.write(u64::from(self.nx), 6);
        w.write(u64::from(self.ny), 6);
        w.write(u64::from(self.nz), 6);
    }
    /// Mirrors VectorSerializer::read: y first, z depends on y parity, x on both.
    fn put(&self, w: &mut BitWriter, p: Vec3) {
        let clamp = |v: i64, bits: u32| v.clamp(0, (1i64 << bits) - 1);
        let iy = clamp(((p.y - self.by) / self.dy).round() as i64, self.ny);
        let zb = self.bz
            + if iy & 1 != 0 {
                (1.0f32 / 3.0f32) * self.dz
            } else {
                0.0
            };
        let iz = clamp(((p.z - zb) / self.dz).round() as i64, self.nz);
        let xb = self.bx + if (iy & 1) != (iz & 1) { 0.5 * self.dx } else { 0.0 };
        let ix = clamp(((p.x - xb) / self.dx).round() as i64, self.nx);
        w.write(ix as u64, self.nx);
        w.write(iy as u64, self.ny);
        w.write(iz as u64, self.nz);
    }
}

/// A body flattened across batches.
#[derive(Debug, Default, Clone)]
pub struct FlatBody {
    pub pos: Vec<Vec3>,
    pub normal: Vec<u32>,
    pub weight: Vec<u32>,
    pub bindex: Vec<u32>,
    pub color: Vec<u32>,
    pub vert_byte_offset: Vec<usize>,
    pub tri_centroid: Vec<Vec3>,
    pub index_buffer_size: usize,
    /// `(sum + 127) & !127`, ApplyBlendShape's gate.
    pub vertex_buffer_size: usize,
}

/// Flattens a model; `None` when an index points past its batch.
pub fn flatten(m: &Model) -> Option<FlatBody> {
    let mut f = FlatBody::default();
    let mut byte_off = 0usize;
    let mut ibytes = 0usize;
    for b in &m.triangle_batches {
        for v in &b.vertices {
            f.pos.push(v.position);
            f.normal.push(v.normal);
            f.weight.push(v.blend_weight);
            f.bindex.push(v.blend_indices);
            f.color.push(v.color);
            f.vert_byte_offset.push(byte_off);
            byte_off += b.vertex_size as usize;
        }
        for t in b.indices.as_chunks::<3>().0 {
            let p = |i: u16| b.vertices.get(usize::from(i)).map(|v| v.position);
            let (a, c, d) = (p(t[0])?, p(t[1])?, p(t[2])?);
            f.tri_centroid.push(Vec3::new(
                (a.x + c.x + d.x) / 3.0,
                (a.y + c.y + d.y) / 3.0,
                (a.z + c.z + d.z) / 3.0,
            ));
        }
        ibytes += b.indices.len() * 2;
    }
    f.index_buffer_size = ibytes;
    f.vertex_buffer_size = (byte_off + 127) & !127usize;
    Some(f)
}

fn dist2(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    dx * dx + dy * dy + dz * dz
}

fn nearest(set: &[Vec3], p: Vec3) -> usize {
    let mut best = 0;
    let mut bd = 1e30f32;
    for (i, &s) in set.iter().enumerate() {
        let d = dist2(s, p);
        if d < bd {
            bd = d;
            best = i;
        }
    }
    best
}

/// Serializes index patch + vertex patch as a raw kShapeOverrides payload in BlendShape's bit layout.
pub fn encode_shape(
    hidden_tris: &[i32],
    index_buffer_size: usize,
    target: &AssetId,
    tucks: &[BlendShapeVertex],
    vertex_buffer_size: usize,
) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.u32(hidden_tris.len() as u32);
    w.u32(index_buffer_size as u32);
    write_asset_id(&mut w, target);
    let ienc = ValueEnc::fit(hidden_tris.iter().map(|&t| t as u32));
    ienc.header(&mut w);
    for &t in hidden_tris {
        ienc.put(&mut w, t as u32);
    }
    w.align();

    w.u32(tucks.len() as u32);
    w.u32(vertex_buffer_size as u32);
    write_asset_id(&mut w, target);
    let oenc = ValueEnc::fit(tucks.iter().map(|t| t.original_offset as u32));
    let nenc = ValueEnc::fit(tucks.iter().map(|t| t.normal));
    let wenc = ValueEnc::fit(tucks.iter().map(|t| t.blend_weight));
    let xenc = ValueEnc::fit(tucks.iter().map(|t| t.blend_indices));
    let cenc = ValueEnc::fit(tucks.iter().map(|t| t.color));
    let pts: Vec<Vec3> = tucks.iter().map(|t| t.position).collect();
    let venc = VectorEnc::fit(&pts);
    oenc.header(&mut w);
    venc.header(&mut w);
    nenc.header(&mut w);
    wenc.header(&mut w);
    xenc.header(&mut w);
    cenc.header(&mut w);
    for t in tucks {
        oenc.put(&mut w, t.original_offset as u32);
        venc.put(&mut w, t.position);
        nenc.put(&mut w, t.normal);
        wenc.put(&mut w, t.blend_weight);
        xenc.put(&mut w, t.blend_indices);
        cenc.put(&mut w, t.color);
    }
    w.align();
    w.bytes
}

/// Appends one block to a bare STRB container (no YTGR wrapper); `false` when it is not one.
pub fn append_block(strb: &mut Vec<u8>, id: u64, payload: &[u8]) -> bool {
    if strb.len() < 26 || &strb[..4] != b"STRB" {
        return false;
    }
    let has_align = strb[4] != 0;
    let le = strb[5] != 0;
    let id_size = usize::from(strb[22]);
    let size_size = usize::from(strb[23]);
    let alignment = if has_align {
        strb.get(26).map_or(0, |&a| usize::from(a))
    } else {
        1
    };
    if alignment == 0 {
        return false;
    }
    let align_up = |v: usize| v.div_ceil(alignment) * alignment;
    let byte_shift = |n: usize, i: usize| if le { i } else { n - 1 - i };
    let rd = |s: &[u8], o: usize, n: usize| -> u64 {
        (0..n).fold(0u64, |v, i| {
            let k = byte_shift(n, i);
            if k < 8 {
                v | (u64::from(s[o + i]) << (8 * k))
            } else {
                v
            }
        })
    };
    // Re-walk the existing blocks to find the true end, ignoring trailing junk.
    let header = align_up(id_size + 2 * size_size);
    let mut off = align_up(if has_align { 30 } else { 26 });
    let mut end = off;
    while off + header <= strb.len() {
        let data_size = rd(strb, off + id_size, size_size);
        off += header;
        let Some(next) = usize::try_from(data_size)
            .ok()
            .and_then(|d| d.checked_add(alignment - 1))
            .map(|d| d / alignment * alignment)
            .and_then(|d| off.checked_add(d))
        else {
            break;
        };
        off = next;
        if off > strb.len() {
            break;
        }
        end = off;
    }
    strb.truncate(end);
    let put = |s: &mut Vec<u8>, v: u64, n: usize| {
        for i in 0..n {
            let k = byte_shift(n, i);
            s.push(if k < 8 { (v >> (8 * k)) as u8 } else { 0 });
        }
    };
    put(strb, id, id_size);
    put(strb, payload.len() as u64, size_size);
    put(strb, 1, size_size);
    while !strb.len().is_multiple_of(alignment) {
        strb.push(0);
    }
    strb.extend_from_slice(payload);
    while !strb.len().is_multiple_of(alignment) {
        strb.push(0);
    }
    true
}

fn load_model(bytes: &[u8]) -> Option<Model> {
    Model::load(bytes, model_option::NONE).ok().flatten()
}

/// --refit-female: reads `in_path`, writes the item with an added female body shape to `out_path`.
pub fn run_refit_female(in_path: &Path, out_path: &Path, toc_path: &Path, log: &mut dyn Write) -> i32 {
    say!(log, "(mode: refit female)\n\n");
    let toc = std::fs::read(toc_path).unwrap_or_default();
    if toc.is_empty() {
        say!(log, "ERROR: cannot read asset pack {}\n", toc_path.display());
        return 1;
    }
    let Ok(pack) = AssetPack::load(toc) else {
        say!(log, "ERROR: AssetPack::Load failed\n");
        return 1;
    };
    let body = |i: usize| pack.asset_data_by_index(i).and_then(load_model);
    let (Some(male), Some(female)) = (body(0), body(1)) else {
        say!(log, "ERROR: cannot decode the stock bodies (pack indices 0/1)\n");
        return 1;
    };
    let (Some(fm), Some(ff)) = (flatten(&male), flatten(&female)) else {
        say!(log, "ERROR: cannot decode the stock bodies (pack indices 0/1)\n");
        return 1;
    };
    say!(
        log,
        "male body:   {} verts / {} tris (vb {}, ib {})\n",
        fm.pos.len(),
        fm.tri_centroid.len(),
        fm.vertex_buffer_size,
        fm.index_buffer_size
    );
    say!(
        log,
        "female body: {} verts / {} tris (vb {}, ib {})\n",
        ff.pos.len(),
        ff.tri_centroid.len(),
        ff.vertex_buffer_size,
        ff.index_buffer_size
    );

    let mut item = match std::fs::read(in_path) {
        Ok(b) if b.len() >= 30 => b,
        _ => {
            say!(log, "ERROR: cannot read {}\n", in_path.display());
            return 1;
        }
    };
    if &item[..4] == b"YTGR" {
        if item.len() < 0x140 {
            say!(log, "ERROR: cannot read {}\n", in_path.display());
            return 1;
        }
        // Drop the signature wrapper.
        item.drain(..0x140);
        say!(log, "note: YTGR wrapper dropped\n");
    }
    let shape_count = count_blocks(&item, BlockId::ShapeOverrides);
    let mut male_shape: Option<BlendShape> = None;
    let mut has_female = false;
    for i in 0..shape_count {
        let Ok(Some((blk, strb))) = find_block_n(&item, BlockId::ShapeOverrides, i) else {
            break;
        };
        let Some(data) = blk.data(strb) else {
            continue;
        };
        let Ok(shape) = BlendShape::read_data(data, shape_option::NONE) else {
            continue;
        };
        let t = shape.index_patch.original_asset_id;
        say!(
            log,
            "shape {i}: target {t}  hidden tris {}  tucks {}\n",
            shape.index_patch.indices.len(),
            shape.vertex_patch.vertices.len()
        );
        if t.a == 2 && t.c == 2 {
            has_female = true;
        }
        if t.a == 2 && t.c == 1 {
            male_shape = Some(shape);
        }
    }
    if has_female {
        say!(log, "item already carries a female body shape; nothing to do\n");
        return 0;
    }
    let Some(male_shape) = male_shape else {
        say!(log, "ERROR: no male body-hiding shape in this item\n");
        return 2;
    };
    let mi = &male_shape.index_patch;
    let mv = &male_shape.vertex_patch;
    if mi.total_buffer_size as usize != fm.index_buffer_size
        || mv.total_buffer_size as usize != fm.vertex_buffer_size
    {
        say!(
            log,
            "WARN: male shape buffer sizes ({}/{}) differ from the current male body ({}/{}); mapping by geometry anyway\n",
            mi.total_buffer_size,
            mv.total_buffer_size,
            fm.index_buffer_size,
            fm.vertex_buffer_size
        );
    }

    // Hidden triangles: the nearest male centroid decides.
    let mut male_hidden = vec![false; fm.tri_centroid.len()];
    for &t in &mi.indices {
        if let Some(h) = usize::try_from(t).ok().and_then(|t| male_hidden.get_mut(t)) {
            *h = true;
        }
    }
    let female_hidden: Vec<i32> = (0..ff.tri_centroid.len())
        .filter(|&t| {
            male_hidden
                .get(nearest(&fm.tri_centroid, ff.tri_centroid[t]))
                .copied()
                .unwrap_or(false)
        })
        .map(|t| t as i32)
        .collect();

    // Tucks: each female vertex copies the delta of its nearest tucked male vertex.
    let mut male_tuck_delta = vec![Vec3::ZERO; fm.pos.len()];
    let mut male_tucked = vec![false; fm.pos.len()];
    let vsize = male
        .triangle_batches
        .first()
        .map_or(0, |b| b.vertex_size as usize);
    if vsize == 0 {
        say!(log, "ERROR: cannot decode the stock bodies (pack indices 0/1)\n");
        return 1;
    }
    for pv in &mv.vertices {
        let Some(vi) = usize::try_from(pv.original_offset)
            .ok()
            .map(|o| o / vsize)
            .filter(|&vi| vi < fm.pos.len())
        else {
            continue;
        };
        let d = Vec3::new(
            pv.position.x - fm.pos[vi].x,
            pv.position.y - fm.pos[vi].y,
            pv.position.z - fm.pos[vi].z,
        );
        if dist2(pv.position, fm.pos[vi]).sqrt() > 0.04 {
            continue;
        }
        male_tuck_delta[vi] = d;
        male_tucked[vi] = true;
    }
    let mut tucks = Vec::new();
    for v in 0..ff.pos.len() {
        let m = nearest(&fm.pos, ff.pos[v]);
        if !male_tucked.get(m).copied().unwrap_or(false) {
            continue;
        }
        if dist2(fm.pos[m], ff.pos[v]).sqrt() > 0.06 {
            continue;
        }
        let d = male_tuck_delta[m];
        tucks.push(BlendShapeVertex {
            original_offset: ff.vert_byte_offset[v] as i32,
            position: Vec3::new(ff.pos[v].x + d.x, ff.pos[v].y + d.y, ff.pos[v].z + d.z),
            normal: ff.normal[v],
            blend_weight: ff.weight[v],
            blend_indices: ff.bindex[v],
            color: ff.color[v],
        });
    }
    say!(
        log,
        "female shape: {} hidden tris (male had {}), {} tucks (male had {})\n",
        female_hidden.len(),
        mi.indices.len(),
        tucks.len(),
        mv.vertices.len()
    );

    // Target: the canonical female body {a=2, b=0, c=2, stock tail}.
    let mut target = mi.original_asset_id;
    target.b = 0;
    target.c = 2;
    let payload = encode_shape(
        &female_hidden,
        ff.index_buffer_size,
        &target,
        &tucks,
        ff.vertex_buffer_size,
    );

    // The encoding must round-trip and apply.
    let check = match BlendShape::read_data(&payload, shape_option::NONE) {
        Ok(c)
            if c.index_patch.indices.len() == female_hidden.len()
                && c.vertex_patch.vertices.len() == tucks.len() =>
        {
            c
        }
        _ => {
            say!(log, "ERROR: encoded shape does not round-trip\n");
            return 3;
        }
    };
    let mut max_err = 0.0f32;
    for (i, (got, want)) in check.vertex_patch.vertices.iter().zip(&tucks).enumerate() {
        max_err = max_err.max(dist2(got.position, want.position).sqrt());
        if got.original_offset != want.original_offset {
            say!(log, "ERROR: tuck {i} offset mismatch\n");
            return 3;
        }
    }
    let mut worn_female = target;
    // Manifests name the worn female body with b = 1.
    worn_female.b = 1;
    let mut female_copy = female.clone();
    let applies = check.matches(&worn_female) && apply_blend_shape(&check, &worn_female, &mut female_copy);
    let degenerate: usize = female_copy
        .triangle_batches
        .iter()
        .map(|b| {
            b.indices
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|t| t[0] == t[1] && t[1] == t[2])
                .count()
        })
        .sum();
    say!(
        log,
        "round-trip OK: max tuck quantization error {:.4} mm; applies to the female body: {} ({} triangles collapsed)\n",
        max_err * 1000.0,
        if applies { "yes" } else { "no" },
        degenerate
    );
    if !applies {
        return 4;
    }

    let mut out = item.clone();
    if !append_block(&mut out, 4, &payload) {
        say!(log, "ERROR: cannot append the block\n");
        return 5;
    }
    // The appended block must parse back from the container.
    let n_after = count_blocks(&out, BlockId::ShapeOverrides);
    let model_after = load_model(&out);
    if n_after != shape_count + 1 || model_after.is_none() {
        say!(
            log,
            "ERROR: container check failed (shapes {shape_count} -> {n_after}, model {})\n",
            if model_after.is_some() { "ok" } else { "LOST" }
        );
        return 6;
    }
    if std::fs::write(out_path, &out).is_err() {
        say!(log, "ERROR: cannot write {}\n", out_path.display());
        return 7;
    }
    say!(
        log,
        "wrote {} ({} -> {} bytes, {} shape blocks)\n",
        out_path.display(),
        item.len(),
        out.len(),
        n_after
    );
    0
}

/// Parses `--refit-female` arguments (the flag itself already removed) and runs the mode.
pub fn run_refit_args(args: &[String], log: &mut dyn Write) -> i32 {
    let mut toc = None;
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--toc" && i + 1 < args.len() {
            toc = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        pos.push(args[i].clone());
        i += 1;
    }
    if pos.len() < 2 {
        say!(
            log,
            "usage: avatarextract --refit-female <item.bin> <out.bin> [--toc <AvatarAssetPack.toc>]\n"
        );
        return 1;
    }
    let Some(toc) = toc.filter(|t| !t.is_empty()) else {
        say!(
            log,
            "ERROR: --toc <AvatarAssetPack.toc> is required (the stock bodies come from the pack).\n"
        );
        return 1;
    };
    run_refit_female(Path::new(&pos[0]), Path::new(&pos[1]), Path::new(&toc), log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use avatar_formats::bits::BitStream;

    #[test]
    fn bit_writer_is_lsb_first() {
        let mut w = BitWriter::default();
        w.write(0b101, 3);
        w.write(0x1F, 5);
        w.u16(0xBEEF);
        let mut s = BitStream::new(w.bytes());
        assert_eq!(s.read(3).unwrap(), 0b101);
        assert_eq!(s.read(5).unwrap(), 0x1F);
        assert_eq!(s.read_u16().unwrap(), 0xBEEF);
    }

    #[test]
    fn encoded_shape_round_trips() {
        let target = AssetId {
            a: 2,
            b: 0,
            c: 2,
            d: [0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0],
        };
        let tucks = vec![
            BlendShapeVertex {
                original_offset: 0,
                position: Vec3::new(0.1, 0.9, -0.05),
                normal: 7,
                blend_weight: 0xFF00,
                blend_indices: 3,
                color: 0xFFFF_FFFF,
            },
            BlendShapeVertex {
                original_offset: 40,
                position: Vec3::new(-0.12, 1.1, 0.02),
                normal: 9,
                blend_weight: 0x00FF,
                blend_indices: 5,
                color: 0x8080_8080,
            },
        ];
        let hidden = [3, 17, 4000];
        let payload = encode_shape(&hidden, 4908, &target, &tucks, 49280);
        let shape = BlendShape::read_data(&payload, shape_option::NONE).unwrap();
        assert_eq!(shape.index_patch.indices, hidden);
        assert_eq!(shape.index_patch.total_buffer_size, 4908);
        assert_eq!(shape.vertex_patch.original_asset_id, target);
        for (got, want) in shape.vertex_patch.vertices.iter().zip(&tucks) {
            assert_eq!(got.original_offset, want.original_offset);
            assert_eq!(got.color, want.color);
            assert!(dist2(got.position, want.position).sqrt() < 0.0005);
        }
    }

    #[test]
    fn append_block_adds_a_parseable_block() {
        // Bare STRB, little endian, 4-byte ids and sizes, no alignment, one 2-byte block.
        let mut strb = b"STRB".to_vec();
        strb.extend_from_slice(&[0, 1]);
        strb.extend_from_slice(&[0; 16]);
        strb.extend_from_slice(&[4, 4, 0, 0]);
        strb.extend_from_slice(&7u32.to_le_bytes());
        strb.extend_from_slice(&2u32.to_le_bytes());
        strb.extend_from_slice(&1u32.to_le_bytes());
        strb.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        assert!(append_block(&mut strb, 4, &[1, 2, 3]));
        let blocks = avatar_formats::strb::blocks(&strb).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].data, &[0xAA, 0xBB]);
        assert_eq!(blocks[1].id, BlockId::ShapeOverrides);
        assert_eq!(blocks[1].data, &[1, 2, 3]);
        assert!(!append_block(&mut b"YTGR".to_vec(), 4, &[]));
    }
}
