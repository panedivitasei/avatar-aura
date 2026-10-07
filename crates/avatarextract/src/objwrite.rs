// Port of WriteModel, DecodeTexture and UvHalf from avatarextract/main.cpp, shared by modes 1, 2 and 5.
//! OBJ, MTL and PNG emitters for a decoded avatar model.

use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use avatar_formats::model::{Model, ShaderParameterType, TriangleBatch};
use avatar_formats::Texture;

/// One texture decoded to tightly packed RGBA8 rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Vertex and triangle totals of a written model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelTotals {
    pub vertices: usize,
    pub triangles: usize,
}

/// Display name of a texture format word, `UNKNOWN` for unsupported formats.
pub fn kind_name(format: u32) -> &'static str {
    bcdec::Format::from_d3d(format).map_or("UNKNOWN", |f| f.name())
}

/// Xenos half float: no denormals, no infinities, exponent 31 is an ordinary exponent.
pub fn xenos_half_to_f32(h: u16) -> f32 {
    let mut mantissa = u32::from(h & 0x3FF);
    let mut exponent = u32::from((h >> 10) & 0x1F);
    if exponent == 0 {
        mantissa = 0;
        exponent = 0u32.wrapping_sub(112);
    }
    let bits = (u32::from(h & 0x8000) << 16) | (exponent.wrapping_add(112) << 23) | (mantissa << 13);
    f32::from_bits(bits)
}

/// Decodes one avatar texture to RGBA8; `None` when it is empty, unsupported or short.
/// The payload is 16-bit word swapped and in linear block order even when `is_tiled` is set.
pub fn decode_texture(tex: &Texture) -> Option<DecodedTexture> {
    if tex.is_empty || tex.data_bytes.is_empty() {
        return None;
    }
    let format = bcdec::Format::from_d3d(tex.format)?;
    let mut swapped = tex.data_bytes.clone();
    bcdec::swap16(&mut swapped);
    // A zero-sized texture decodes to an empty image, which the PNG writer then refuses.
    let rgba = if tex.width == 0 || tex.height == 0 {
        Vec::new()
    } else {
        bcdec::decode(format, tex.width, tex.height, &swapped).ok()?
    };
    Some(DecodedTexture {
        width: tex.width,
        height: tex.height,
        rgba,
    })
}

/// Writes an RGBA8 image as PNG.
pub fn write_png(path: &Path, tex: &DecodedTexture) -> anyhow::Result<()> {
    if tex.rgba.is_empty() {
        anyhow::bail!("empty image");
    }
    image::save_buffer_with_format(
        path,
        &tex.rgba,
        tex.width,
        tex.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    Ok(())
}

/// Texture slot of the first texture shader parameter, or -1.
fn batch_texture_index(b: &TriangleBatch) -> i32 {
    b.shader_parameters
        .iter()
        .find(|sp| sp.r#type == ShaderParameterType::Texture)
        .map_or(-1, |sp| i32::from(sp.texture.index))
}

/// The `<prefix>.mtl` text; `png_names[i]` is empty for textures that were not written.
pub fn mtl_text(model: &Model, png_names: &[String]) -> String {
    let mut s = String::new();
    for (bi, b) in model.triangle_batches.iter().enumerate() {
        let _ = writeln!(s, "newmtl mat{bi}");
        s.push_str("Ka 1 1 1\nKd 1 1 1\nKs 0 0 0\nd 1\nillum 1\n");
        let ti = batch_texture_index(b);
        if let Some(name) = usize::try_from(ti).ok().and_then(|t| png_names.get(t)) {
            if !name.is_empty() {
                let _ = writeln!(s, "map_Kd {name}");
            }
        }
        s.push('\n');
    }
    s
}

/// The `<prefix>.obj` text with area-weighted per-vertex normals and V flipped for OBJ.
pub fn obj_text(model: &Model, prefix: &str) -> String {
    let mut s = String::new();
    s.push_str("# avatarextract OBJ export\n");
    let _ = write!(s, "mtllib {prefix}.mtl\n\n");
    // OBJ indices are 1-based and global.
    let mut vbase: u32 = 1;
    for (bi, b) in model.triangle_batches.iter().enumerate() {
        let _ = writeln!(s, "o batch{bi}");
        for v in &b.vertices {
            let p = v.position;
            let _ = writeln!(s, "v {:.6} {:.6} {:.6}", p.x, p.y, p.z);
        }
        for v in &b.vertices {
            let (u, vv) = v.uvs.first().map_or((0.0, 0.0), |uv| {
                (xenos_half_to_f32(uv.x), xenos_half_to_f32(uv.y))
            });
            let _ = writeln!(s, "vt {:.6} {:.6}", u, 1.0f32 - vv);
        }
        // The vertex packs an FMT_10_11_11 normal the decoder never unpacks, so normals come from the faces.
        let n = b.vertices.len();
        let mut normals = vec![[0.0f32; 3]; n];
        for tri in b.indices.as_chunks::<3>().0 {
            let (i0, i1, i2) = (usize::from(tri[0]), usize::from(tri[1]), usize::from(tri[2]));
            if i0 >= n || i1 >= n || i2 >= n {
                continue;
            }
            let p0 = b.vertices[i0].position;
            let p1 = b.vertices[i1].position;
            let p2 = b.vertices[i2].position;
            let (ex1, ey1, ez1) = (p1.x - p0.x, p1.y - p0.y, p1.z - p0.z);
            let (ex2, ey2, ez2) = (p2.x - p0.x, p2.y - p0.y, p2.z - p0.z);
            let nx = ey1 * ez2 - ez1 * ey2;
            let ny = ez1 * ex2 - ex1 * ez2;
            let nz = ex1 * ey2 - ey1 * ex2;
            for i in [i0, i1, i2] {
                normals[i][0] += nx;
                normals[i][1] += ny;
                normals[i][2] += nz;
            }
        }
        for nrm in &mut normals {
            let len = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt();
            if len > 1e-12f32 {
                nrm[0] /= len;
                nrm[1] /= len;
                nrm[2] /= len;
            } else {
                *nrm = [0.0, 0.0, 1.0];
            }
            let _ = writeln!(s, "vn {:.6} {:.6} {:.6}", nrm[0], nrm[1], nrm[2]);
        }
        let _ = writeln!(s, "usemtl mat{bi}");
        for tri in b.indices.as_chunks::<3>().0 {
            let a = vbase.wrapping_add(u32::from(tri[0]));
            let c = vbase.wrapping_add(u32::from(tri[1]));
            let d = vbase.wrapping_add(u32::from(tri[2]));
            let _ = writeln!(s, "f {a}/{a}/{a} {c}/{c}/{c} {d}/{d}/{d}");
        }
        vbase = vbase.wrapping_add(b.vertices.len() as u32);
        s.push('\n');
    }
    s
}

/// Writes `<prefix>.obj`, `<prefix>.mtl` and `<prefix>_texture<i>.png` into `out_dir`, logging as the C++ does.
/// Returns `None` on a hard write error.
pub fn write_model(model: &Model, out_dir: &Path, prefix: &str, log: &mut dyn Write) -> Option<ModelTotals> {
    let totals = ModelTotals {
        vertices: model.vertex_count(),
        triangles: model.triangle_count(),
    };
    say!(log, "\n=== Model ({prefix}) ===\n");
    say!(log, "  triangle batches: {}\n", model.triangle_batches.len());
    say!(log, "  total vertices  : {}\n", totals.vertices);
    say!(log, "  total triangles : {}\n", totals.triangles);
    say!(log, "  textures        : {}\n", model.textures.len());

    if let Err(e) = std::fs::create_dir_all(out_dir) {
        say!(log, "ERROR: cannot create {}: {e}\n", out_dir.display());
        return None;
    }

    let mut png_names = vec![String::new(); model.textures.len()];
    say!(log, "\n=== Textures ({prefix}) ===\n");
    for (i, mt) in model.textures.iter().enumerate() {
        let tex = &mt.texture;
        say!(
            log,
            "  [{i}] fmt=0x{:X} ({}) {}x{} tiled={} layers={} empty={} bytes={}\n",
            tex.format,
            kind_name(tex.format),
            tex.width,
            tex.height,
            u8::from(tex.is_tiled),
            tex.layer_count,
            u8::from(tex.is_empty),
            tex.data_bytes.len()
        );
        let Some(dec) = decode_texture(tex) else {
            say!(log, "       -> skipped (empty or unsupported format)\n");
            continue;
        };
        let png = format!("{prefix}_texture{i}.png");
        if write_png(&out_dir.join(&png), &dec).is_ok() {
            say!(log, "       -> {png}\n");
            png_names[i] = png;
        } else {
            say!(log, "       -> FAILED to write {png}\n");
        }
    }

    let obj_path = out_dir.join(format!("{prefix}.obj"));
    let mtl_path = out_dir.join(format!("{prefix}.mtl"));
    if std::fs::write(&mtl_path, mtl_text(model, &png_names)).is_err() {
        say!(log, "ERROR: cannot write {}\n", mtl_path.display());
        return None;
    }
    if std::fs::write(&obj_path, obj_text(model, prefix)).is_err() {
        say!(log, "ERROR: cannot write {}\n", obj_path.display());
        return None;
    }

    let mut bbox: Option<([f32; 3], [f32; 3])> = None;
    for v in model.triangle_batches.iter().flat_map(|b| &b.vertices) {
        let p = [v.position.x, v.position.y, v.position.z];
        bbox = Some(match bbox {
            None => (p, p),
            Some((mut lo, mut hi)) => {
                // Same comparisons as std::min / std::max, so NaN handling matches.
                for k in 0..3 {
                    if p[k] < lo[k] {
                        lo[k] = p[k];
                    }
                    if hi[k] < p[k] {
                        hi[k] = p[k];
                    }
                }
                (lo, hi)
            }
        });
    }
    if let Some((lo, hi)) = bbox {
        say!(
            log,
            "  bbox: X [{} .. {}] Y [{} .. {}] Z [{} .. {}]\n",
            crate::space_f4(lo[0]),
            crate::space_f4(hi[0]),
            crate::space_f4(lo[1]),
            crate::space_f4(hi[1]),
            crate::space_f4(lo[2]),
            crate::space_f4(hi[2])
        );
    }
    say!(log, "  -> {}\n  -> {}\n", obj_path.display(), mtl_path.display());
    Some(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xenos_half_has_no_denormals_or_infinities() {
        assert_eq!(xenos_half_to_f32(0x3C00), 1.0);
        assert_eq!(xenos_half_to_f32(0x3800), 0.5);
        assert_eq!(xenos_half_to_f32(0xBC00), -1.0);
        assert_eq!(xenos_half_to_f32(0x0001), 0.0);
        assert_eq!(xenos_half_to_f32(0x7C00), 65536.0);
    }

    #[test]
    fn decode_skips_empty_unknown_and_short() {
        let mut t = Texture {
            format: 18,
            width: 4,
            height: 4,
            data_bytes: vec![0; 8],
            ..Default::default()
        };
        assert!(decode_texture(&t).is_some());
        t.format = 7;
        assert!(decode_texture(&t).is_none());
        t.format = 18;
        t.data_bytes.truncate(6);
        assert!(decode_texture(&t).is_none());
        t.is_empty = true;
        assert!(decode_texture(&t).is_none());
    }

    #[test]
    fn space_flag_formatting() {
        assert_eq!(crate::space_f4(0.5), " 0.5000");
        assert_eq!(crate::space_f4(-0.5), "-0.5000");
        assert_eq!(crate::space_f4(-0.0), "-0.0000");
    }
}
