//! Ports the baked-materials region of avatar_export.cpp (Sampler, MaterialRecipe, LayerDensity, BakeMaterial,
//! BuildAtlas, BakeBatch, WritePng) plus texdecode.h's layer decode and the face/ layer and composite writer.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;

use avatar_formats::Texture;
use bcdec::{decode_xenos_layer, Format, XenosLayout};
use glam::{DVec4, Vec2, Vec3, Vec4};

use crate::glmath::{
    clamp3, clampf, cross3, fmax, fmax3, fmin, fmin3, length2, length3, max2, min2, mix3, mix4,
};
use crate::manifest::{color_to_float4, Float4, WHITE};
use crate::msvc_sort;
use crate::skin::{BakedMesh, SkinVertex};

/// texdec::Image: RGBA8 rows, top row first; `ok` is false when the decode failed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub ok: bool,
}

impl Image {
    pub fn texel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    fn put(&mut self, x: u32, y: u32, p: [u8; 4]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba[i..i + 4].copy_from_slice(&p);
    }
}

/// texdec::KindName for the log.
pub fn kind_name(format: u32) -> &'static str {
    Format::from_d3d(format).map_or("UNKNOWN", Format::name)
}

/// texdec::DecodeTextureLayer.
pub fn decode_texture_layer(tex: &Texture, layer: u32) -> Image {
    let mut out = Image {
        width: tex.width,
        height: tex.height,
        ..Default::default()
    };
    if tex.is_empty || tex.data_bytes.is_empty() || tex.width == 0 || tex.height == 0 {
        return out;
    }
    let layout = XenosLayout {
        format: tex.format,
        width: tex.width,
        height: tex.height,
        data_stride: tex.data_stride,
        data_rows: tex.data_rows,
    };
    if let Ok(rgba) = decode_xenos_layer(&layout, layer, &tex.data_bytes) {
        out.rgba = rgba;
        out.ok = true;
    }
    out
}

/// WritePng: false for a failed image or a failed write.
pub fn write_png(path: &Path, img: &Image) -> bool {
    if !img.ok {
        return false;
    }
    image::save_buffer(
        path,
        &img.rgba,
        img.width,
        img.height,
        image::ExtendedColorType::Rgba8,
    )
    .is_ok()
}

fn texel_vec(p: [u8; 4]) -> Vec4 {
    Vec4::new(f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])) / 255.0
}

#[derive(Clone, Debug, Default)]
pub struct Sampler {
    pub img: Option<Rc<Image>>,
    pub uv_layer: usize,
    /// XAVATAR_TEXTURE_FLAGS: bit 0 wraps U, bit 1 wraps V.
    pub flags: u32,
    /// Decal layers: outside [0,1] contributes nothing.
    pub transparent_border: bool,
    /// Magnified by the bake: keep the source texel grid.
    pub point: bool,
}

impl Sampler {
    fn wrap_u(&self) -> bool {
        self.flags & 1 != 0
    }

    fn wrap_v(&self) -> bool {
        self.flags & 2 != 0
    }

    fn ready(&self) -> Option<&Image> {
        self.img.as_deref().filter(|i| i.ok)
    }

    pub fn sample(&self, mut uv: Vec2) -> Vec4 {
        let Some(img) = self.img.as_deref() else {
            return Vec4::ZERO;
        };
        if !img.ok || img.width == 0 || img.height == 0 {
            return Vec4::ZERO;
        }
        if self.transparent_border && (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            return Vec4::ZERO;
        }
        if !self.wrap_u() {
            uv.x = clampf(uv.x, 0.0, 1.0);
        }
        if !self.wrap_v() {
            uv.y = clampf(uv.y, 0.0, 1.0);
        }
        let (w, h) = (img.width as i32, img.height as i32);
        if self.point {
            let px = (uv.x * img.width as f32).floor() as i32;
            let py = (uv.y * img.height as f32).floor() as i32;
            let px = if self.wrap_u() {
                px.rem_euclid(w)
            } else {
                px.clamp(0, w - 1)
            };
            let py = if self.wrap_v() {
                py.rem_euclid(h)
            } else {
                py.clamp(0, h - 1)
            };
            return texel_vec(img.texel(px as u32, py as u32));
        }
        let fx = uv.x * img.width as f32 - 0.5;
        let fy = uv.y * img.height as f32 - 0.5;
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let clampi = |v: i32, n: i32| v.clamp(0, n - 1);
        let xa = if self.wrap_u() {
            x0.rem_euclid(w)
        } else {
            clampi(x0, w)
        };
        let xb = if self.wrap_u() {
            x0.wrapping_add(1).rem_euclid(w)
        } else {
            clampi(x0.saturating_add(1), w)
        };
        let ya = if self.wrap_v() {
            y0.rem_euclid(h)
        } else {
            clampi(y0, h)
        };
        let yb = if self.wrap_v() {
            y0.wrapping_add(1).rem_euclid(h)
        } else {
            clampi(y0.saturating_add(1), h)
        };
        let px = |x: i32, y: i32| texel_vec(img.texel(x as u32, y as u32));
        let c00 = px(xa, ya);
        let c10 = px(xb, ya);
        let c01 = px(xa, yb);
        let c11 = px(xb, yb);
        mix4(mix4(c00, c10, tx), mix4(c01, c11, tx), ty)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaderKind {
    #[default]
    Body,
    Head,
}

fn f4(c: &Float4) -> Vec3 {
    Vec3::new(c[0], c[1], c[2])
}

#[derive(Clone, Debug)]
pub struct MaterialRecipe {
    pub kind: ShaderKind,
    pub shader_id: u32,
    pub out_uv_layer: usize,
    pub color: Sampler,
    pub intensity: Sampler,
    pub decal: Sampler,
    pub custom: [Float4; 3],
    pub base: Sampler,
    pub eyeshadow: Sampler,
    pub mouth: Sampler,
    pub eye: Sampler,
    pub facial_hair: Sampler,
    pub brow: Sampler,
    /// colors[0..8].
    pub tint: [Float4; 9],
}

impl Default for MaterialRecipe {
    fn default() -> Self {
        MaterialRecipe {
            kind: ShaderKind::Body,
            shader_id: 0,
            out_uv_layer: 0,
            color: Sampler::default(),
            intensity: Sampler::default(),
            decal: Sampler::default(),
            custom: [WHITE; 3],
            base: Sampler::default(),
            eyeshadow: Sampler::default(),
            mouth: Sampler::default(),
            eye: Sampler::default(),
            facial_hair: Sampler::default(),
            brow: Sampler::default(),
            tint: [WHITE; 9],
        }
    }
}

impl MaterialRecipe {
    fn all(&self) -> [&Sampler; 9] {
        [
            &self.color,
            &self.intensity,
            &self.decal,
            &self.base,
            &self.eyeshadow,
            &self.mouth,
            &self.eye,
            &self.facial_hair,
            &self.brow,
        ]
    }

    fn all_mut(&mut self) -> [&mut Sampler; 9] {
        [
            &mut self.color,
            &mut self.intensity,
            &mut self.decal,
            &mut self.base,
            &mut self.eyeshadow,
            &mut self.mouth,
            &mut self.eye,
            &mut self.facial_hair,
            &mut self.brow,
        ]
    }

    pub fn evaluate(&self, uv: &[Vec2; 6]) -> Vec4 {
        if self.kind == ShaderKind::Body {
            let c = if self.color.img.is_some() {
                self.color.sample(uv[self.color.uv_layer])
            } else {
                Vec4::ONE
            };
            let mut rgb = c.truncate();
            if self.intensity.img.is_some() {
                let i = self.intensity.sample(uv[self.intensity.uv_layer]);
                let recolor =
                    f4(&self.custom[0]) * i.x + f4(&self.custom[1]) * i.y + f4(&self.custom[2]) * i.z;
                rgb = mix3(rgb, recolor, i.w);
            }
            if self.decal.img.is_some() {
                let d = self.decal.sample(uv[self.decal.uv_layer]);
                rgb = mix3(rgb, d.truncate(), d.w);
            }
            return clamp3(rgb, 0.0, 1.0).extend(c.w);
        }
        let skin = f4(&self.tint[0]);
        let mut rgb = skin;
        if self.base.img.is_some() {
            let b = self.base.sample(uv[self.base.uv_layer]);
            rgb = mix3(
                skin,
                f4(&self.tint[7]) * b.x + f4(&self.tint[8]) * b.y + skin * b.z,
                b.w,
            );
        }
        if self.eyeshadow.img.is_some() {
            let s = self.eyeshadow.sample(uv[self.eyeshadow.uv_layer]);
            rgb = mix3(rgb, f4(&self.tint[5]) * s.x, s.w);
        }
        let feature = |rgb: Vec3, s: &Sampler, tint: &Float4| -> Vec3 {
            let m = s.sample(uv[s.uv_layer]);
            mix3(rgb, f4(tint) * m.x + Vec3::splat(m.y) + skin * m.z, m.w)
        };
        if self.mouth.img.is_some() {
            rgb = feature(rgb, &self.mouth, &self.tint[2]);
        }
        if self.eye.img.is_some() {
            rgb = feature(rgb, &self.eye, &self.tint[3]);
        }
        if self.facial_hair.img.is_some() {
            rgb = feature(rgb, &self.facial_hair, &self.tint[6]);
        }
        if self.brow.img.is_some() {
            rgb = feature(rgb, &self.brow, &self.tint[4]);
        }
        clamp3(rgb, 0.0, 1.0).extend(1.0)
    }

    /// How strongly an overlay owns this texel, so a decal wins a texel two folded patches share.
    pub fn overlay_alpha(&self, uv: &[Vec2; 6]) -> f32 {
        if self.kind == ShaderKind::Body {
            return if self.decal.img.is_some() {
                self.decal.sample(uv[self.decal.uv_layer]).w
            } else {
                0.0
            };
        }
        let mut a = 0.0f32;
        for s in [
            &self.eyeshadow,
            &self.mouth,
            &self.eye,
            &self.facial_hair,
            &self.brow,
        ] {
            if s.img.is_some() {
                a = fmax(a, s.sample(uv[s.uv_layer]).w);
            }
        }
        a
    }

    /// Every bound layer reads the output UV set, so the bake can go texel by texel.
    pub fn single_uv_set(&self) -> bool {
        self.all()
            .iter()
            .all(|s| s.img.is_none() || s.uv_layer == self.out_uv_layer)
    }

    pub fn native_size(&self) -> (i32, i32) {
        let mut w = 0i32;
        let mut h = 0i32;
        for s in self.all() {
            if let Some(img) = s.ready() {
                w = w.max(img.width as i32);
                h = h.max(img.height as i32);
            }
        }
        (w.max(4), h.max(4))
    }
}

/// The density read at the top 2% of the UV area, so one sliver cannot blow the bake up.
fn percentile(v: &mut [(f32, f32)], total: f32) -> f32 {
    if v.is_empty() || total <= 0.0 {
        return 0.0;
    }
    msvc_sort::sort_by(v, |a, b| a.0 > b.0);
    let mut acc = 0.0f32;
    for e in v.iter() {
        acc += e.1;
        if acc >= 0.02 * total {
            return e.0;
        }
    }
    v[v.len() - 1].0
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - b.x * a.y
}

/// Texel gradients of one layer against a reference parameterization, per triangle, with their areas.
#[allow(clippy::too_many_arguments)]
fn gradients(
    du_area: &mut Vec<(f32, f32)>,
    dv_area: &mut Vec<(f32, f32)>,
    e1: Vec2,
    e2: Vec2,
    det: f32,
    f1: Vec2,
    f2: Vec2,
    tex: Vec2,
) -> f32 {
    let du = (f1 * e2.y - f2 * e1.y) / det * tex;
    let dv = (f2 * e1.x - f1 * e2.x) / det * tex;
    let area = 0.5 * det.abs();
    du_area.push((length2(du), area));
    dv_area.push((length2(dv), area));
    area
}

fn tri(verts: &[SkinVertex], indices: &[u16], t: usize) -> Option<[SkinVertex; 3]> {
    Some([
        *verts.get(usize::from(*indices.get(t)?))?,
        *verts.get(usize::from(*indices.get(t + 1)?))?,
        *verts.get(usize::from(*indices.get(t + 2)?))?,
    ])
}

/// LayerDensity: layer texels per unit of output UV, the busiest bound layer deciding.
pub fn layer_density(recipe: &MaterialRecipe, verts: &[SkinVertex], indices: &[u16]) -> Vec2 {
    let mut need = Vec2::ZERO;
    let l = recipe.out_uv_layer;
    let mut du_area = Vec::new();
    let mut dv_area = Vec::new();
    for s in recipe.all() {
        let Some(img) = s.ready() else {
            continue;
        };
        let tex = Vec2::new(img.width as f32, img.height as f32);
        if s.uv_layer == l {
            need = max2(need, tex);
            continue;
        }
        du_area.clear();
        dv_area.clear();
        let mut total = 0.0f32;
        let mut t = 0;
        while t + 2 < indices.len() {
            let Some(v) = tri(verts, indices, t) else {
                t += 3;
                continue;
            };
            t += 3;
            let e1 = v[1].uvs[l] - v[0].uvs[l];
            let e2 = v[2].uvs[l] - v[0].uvs[l];
            let det = e1.x * e2.y - e2.x * e1.y;
            if det.abs() < 1e-9 {
                continue;
            }
            let f1 = v[1].uvs[s.uv_layer] - v[0].uvs[s.uv_layer];
            let f2 = v[2].uvs[s.uv_layer] - v[0].uvs[s.uv_layer];
            total += gradients(&mut du_area, &mut dv_area, e1, e2, det, f1, f2, tex);
        }
        need = max2(
            need,
            Vec2::new(percentile(&mut du_area, total), percentile(&mut dv_area, total)),
        );
    }
    need
}

/// A generated atlas goes in the last UV slot: body shaders bind four sets at most.
pub const ATLAS_UV_LAYER: usize = 5;
const ATLAS_PADDING: i32 = 3;
const CLASH_TOLERANCE: f32 = 0.1;
const ATLAS_CLASH: f32 = 0.03;

fn store(out: &mut Image, x: i32, y: i32, c: Vec4) {
    let q = |v: f32| (clampf(v, 0.0, 1.0) * 255.0).round() as u8;
    out.put(x as u32, y as u32, [q(c.x), q(c.y), q(c.z), q(c.w)]);
}

/// BakeMaterial: rasterizes the batch in the output UV space and evaluates the recipe per texel; uncovered
/// texels are dilated from covered ones, then filled from the layers living in the output UV set.
pub fn bake_material(
    mut recipe: MaterialRecipe,
    verts: &[SkinVertex],
    indices: &[u16],
    size_hint: i32,
    atlas_size: (i32, i32),
    mut clash: Option<&mut f32>,
) -> Image {
    let native = recipe.native_size();
    let atlas = atlas_size.0 > 0 && atlas_size.1 > 0;
    if let Some(c) = clash.as_deref_mut() {
        *c = 0.0;
    }
    let (w, h, base_w, base_h);
    if atlas {
        (w, h, base_w, base_h) = (atlas_size.0, atlas_size.1, 0, 0);
    } else if recipe.single_uv_set() {
        (w, h, base_w, base_h) = (native.0, native.1, native.0, native.1);
    } else {
        // The [0,1]^2 UV space of the output layer: square unless the output layer's own texture is not.
        let primary = if recipe.kind == ShaderKind::Head {
            &recipe.base
        } else {
            &recipe.color
        };
        let mut target = size_hint;
        if recipe.kind == ShaderKind::Body {
            target = size_hint.min(256.max(4 * native.0.max(native.1)));
        }
        let (mut tw, mut th) = (target, target);
        if let Some(img) = primary.ready() {
            if img.width != img.height {
                if img.width > img.height {
                    th = 4.max(target * img.height as i32 / img.width as i32);
                } else {
                    tw = 4.max(target * img.width as i32 / img.height as i32);
                }
            }
        }
        // The busiest layer then gets room for its own texels, up to --bake-size.
        let need = layer_density(&recipe, verts, indices);
        let raise = |have: i32, want: f32| {
            let mut n = 4;
            while n < want.ceil() as i32 && n < size_hint {
                n *= 2;
            }
            have.max(size_hint.min(n))
        };
        (w, h, base_w, base_h) = (raise(tw, need.x), raise(th, need.y), tw, th);
    }
    // A layer mapped 1:1 into the output is only magnified: replicate its texels instead of pre-blurring.
    if !atlas && recipe.kind == ShaderKind::Body && w == base_w && h == base_h {
        let out_uv = recipe.out_uv_layer;
        for s in [&mut recipe.color, &mut recipe.intensity, &mut recipe.decal] {
            let magnified = s
                .ready()
                .is_some_and(|img| w > img.width as i32 && h > img.height as i32);
            if magnified && s.uv_layer == out_uv {
                s.point = true;
            }
        }
    }
    let w = w.max(0);
    let h = h.max(0);
    let mut out = Image {
        width: w as u32,
        height: h as u32,
        rgba: vec![0; w as usize * h as usize * 4],
        ok: false,
    };
    let n = w as usize * h as usize;
    if n == 0 {
        out.ok = true;
        return out;
    }
    let (wf, hf) = (w as f32, h as f32);

    if !atlas && recipe.single_uv_set() {
        for y in 0..h {
            for x in 0..w {
                let t = Vec2::new((x as f32 + 0.5) / wf, (y as f32 + 0.5) / hf);
                let value = recipe.evaluate(&[t; 6]);
                store(&mut out, x, y, value);
            }
        }
        out.ok = true;
        return out;
    }

    let mut covered = vec![0u8; n];
    let mut owner = vec![0u8; n];
    let mut clashed = vec![0u8; if clash.is_some() { n } else { 0 }];
    let l = recipe.out_uv_layer;
    let mut t = 0;
    while t + 2 < indices.len() {
        let tv = tri(verts, indices, t);
        t += 3;
        let Some(v) = tv else {
            continue;
        };
        let p = v.map(|v| Vec2::new(v.uvs[l].x * wf, v.uvs[l].y * hf));
        let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
        if area.abs() < 1e-8 {
            continue;
        }
        let minx = fmin3(p[0].x, p[1].x, p[2].x);
        let maxx = fmax3(p[0].x, p[1].x, p[2].x);
        let miny = fmin3(p[0].y, p[1].y, p[2].y);
        let maxy = fmax3(p[0].y, p[1].y, p[2].y);
        if maxx - minx > 4.0 * wf || maxy - miny > 4.0 * hf {
            continue;
        }
        let x0 = (minx.floor() as i32).saturating_sub(1);
        let x1 = (maxx.ceil() as i32).saturating_add(1);
        let y0 = (miny.floor() as i32).saturating_sub(1);
        let y1 = (maxy.ceil() as i32).saturating_add(1);
        let inv_area = 1.0 / area;
        // Distance-based tolerance (about 0.75 texel) so seams get covered.
        let eps = 0.75 / area.abs().sqrt() * 2.0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1].x - c.x) * (p[2].y - c.y) - (p[2].x - c.x) * (p[1].y - c.y)) * inv_area;
                let w1 = ((p[2].x - c.x) * (p[0].y - c.y) - (p[0].x - c.x) * (p[2].y - c.y)) * inv_area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -eps || w1 < -eps || w2 < -eps {
                    continue;
                }
                let ox = x.rem_euclid(w);
                let oy = y.rem_euclid(h);
                let oi = oy as usize * w as usize + ox as usize;
                let inside = w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0;
                if !inside && covered[oi] != 0 {
                    continue;
                }
                let b = Vec3::new(clampf(w0, 0.0, 1.0), clampf(w1, 0.0, 1.0), clampf(w2, 0.0, 1.0));
                let b = b / (b.x + b.y + b.z);
                let mut uv = [Vec2::ZERO; 6];
                for (k, u) in uv.iter_mut().enumerate() {
                    *u = v[0].uvs[k] * b.x + v[1].uvs[k] * b.y + v[2].uvs[k] * b.z;
                }
                let value = recipe.evaluate(&uv);
                if clash.is_some() && inside && covered[oi] == 2 {
                    // Folded halves agree on the colour; anything else is two patches wanting different art.
                    let p = out.texel(ox as u32, oy as u32);
                    let was = Vec3::new(f32::from(p[0]), f32::from(p[1]), f32::from(p[2]));
                    let d = (was / 255.0 - clamp3(value.truncate(), 0.0, 1.0)).abs();
                    if fmax3(d.x, d.y, d.z) > CLASH_TOLERANCE {
                        clashed[oi] = 1;
                    }
                }
                let own = (clampf(recipe.overlay_alpha(&uv), 0.0, 1.0) * 255.0).round() as u8;
                if covered[oi] != 0 && owner[oi] > own {
                    continue;
                }
                store(&mut out, ox, oy, value);
                owner[oi] = own;
                covered[oi] = if inside { 2 } else { 1 };
            }
        }
    }
    if let Some(c) = clash {
        let hit = covered.iter().filter(|&&c| c == 2).count();
        let bad = clashed.iter().filter(|&&c| c != 0).count();
        *c = if hit != 0 { bad as f32 / hit as f32 } else { 0.0 };
    }
    // Dilate covered texels outward to hide filtering seams.
    let passes = if atlas { 2 * ATLAS_PADDING } else { 6 };
    for _ in 0..passes {
        let mut next = covered.clone();
        for y in 0..h {
            for x in 0..w {
                if covered[y as usize * w as usize + x as usize] != 0 {
                    continue;
                }
                let mut n = 0i32;
                let mut acc = [0i32; 4];
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let sx = (x + dx).rem_euclid(w);
                        let sy = (y + dy).rem_euclid(h);
                        if covered[sy as usize * w as usize + sx as usize] == 0 {
                            continue;
                        }
                        let p = out.texel(sx as u32, sy as u32);
                        for (a, &c) in acc.iter_mut().zip(p.iter()) {
                            *a += i32::from(c);
                        }
                        n += 1;
                    }
                }
                if n != 0 {
                    out.put(x as u32, y as u32, acc.map(|a| (a / n) as u8));
                    next[y as usize * w as usize + x as usize] = 1;
                }
            }
        }
        covered = next;
    }
    // Space this batch does not own: only layers living in the output UV set, never decal or feature layers,
    // which would otherwise paint stretched eyes over the unused half of a head texture.
    let mut background = recipe.clone();
    let out_uv = recipe.out_uv_layer;
    for s in background.all_mut() {
        if s.uv_layer != out_uv || s.transparent_border {
            s.img = None;
        }
    }
    background.decal.img = None;
    background.eyeshadow.img = None;
    background.mouth.img = None;
    background.eye.img = None;
    background.facial_hair.img = None;
    background.brow.img = None;
    // An atlas has no layer of its own, so the gaps between charts take the average colour.
    let mut average = Vec4::ONE;
    if atlas {
        let mut sum = DVec4::ZERO;
        let mut count = 0usize;
        for (i, &c) in covered.iter().enumerate() {
            if c != 2 {
                continue;
            }
            let p = &out.rgba[i * 4..i * 4 + 4];
            sum += DVec4::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]), f64::from(p[3]));
            count += 1;
        }
        if count != 0 {
            average = (sum / (255.0 * count as f64)).as_vec4();
        }
    }
    for y in 0..h {
        for x in 0..w {
            if covered[y as usize * w as usize + x as usize] != 0 {
                continue;
            }
            let value = if atlas {
                average
            } else {
                let t = Vec2::new((x as f32 + 0.5) / wf, (y as f32 + 0.5) / hf);
                background.evaluate(&[t; 6])
            };
            store(&mut out, x, y, value);
        }
    }
    out.ok = true;
    out
}

#[derive(Clone, Debug, Default)]
struct Chart {
    tris: Vec<u32>,
    lo: Vec2,
    extent: Vec2,
    density: Vec2,
    pos: (i32, i32),
    size: (i32, i32),
}

/// BuildAtlas: lays the batch out again in UV slot 5. Charts are the connected, unfolded patches of whichever
/// bound layer follows the surface best, shelf-packed; triangles that layer squashes are laid flat from 3D.
pub fn build_atlas(
    recipe: &MaterialRecipe,
    verts: &mut Vec<SkinVertex>,
    indices: &mut Vec<u16>,
    size_cap: i32,
) -> Option<(i32, i32)> {
    let tri_count = indices.len() / 3;
    if tri_count == 0 || indices.iter().any(|&i| usize::from(i) >= verts.len()) {
        return None;
    }
    let mut layers: Vec<&Sampler> = Vec::new();
    for s in [&recipe.color, &recipe.intensity, &recipe.decal] {
        if s.ready().is_none() {
            continue;
        }
        if s.uv_layer == ATLAS_UV_LAYER {
            return None;
        }
        layers.push(s);
    }
    if layers.is_empty() {
        return None;
    }
    let vert = |t: usize, k: usize| -> &SkinVertex { &verts[usize::from(indices[t * 3 + k])] };

    let mut area3 = vec![0.0f32; tri_count];
    let mut total3 = 0.0f32;
    for (t, a) in area3.iter_mut().enumerate() {
        *a = 0.5
            * length3(cross3(
                vert(t, 1).orig_position - vert(t, 0).orig_position,
                vert(t, 2).orig_position - vert(t, 0).orig_position,
            ));
        total3 += *a;
    }
    if total3 <= 0.0 {
        return None;
    }

    // The layout layer is the one whose scale holds steady over the most surface.
    const STRETCH: f32 = 16.0;
    let mut layout: Option<usize> = None;
    let mut layout_scale = 0.0f32;
    let mut best = 0.0f32;
    for s in &layers {
        let l = s.uv_layer;
        let mut ratio: Vec<(f32, f32)> = Vec::new();
        for (t, &a3) in area3.iter().enumerate() {
            if a3 <= 0.0 {
                continue;
            }
            let a = 0.5
                * cross2(
                    vert(t, 1).uvs[l] - vert(t, 0).uvs[l],
                    vert(t, 2).uvs[l] - vert(t, 0).uvs[l],
                )
                .abs();
            ratio.push((a / a3, a3));
        }
        msvc_sort::sort_by(&mut ratio, |a, b| {
            a.0 < b.0 || (a.0.partial_cmp(&b.0) != Some(std::cmp::Ordering::Greater) && a.1 < b.1)
        });
        let mut acc = 0.0f32;
        let mut median = 0.0f32;
        for r in &ratio {
            acc += r.1;
            if acc >= 0.5 * total3 {
                median = r.0;
                break;
            }
        }
        if median <= 0.0 {
            continue;
        }
        let mut steady = 0.0f32;
        for r in &ratio {
            if r.0 >= median / STRETCH && r.0 <= median * STRETCH {
                steady += r.1;
            }
        }
        if steady > best + 1e-3 * total3 {
            best = steady;
            layout = Some(l);
            layout_scale = median;
        }
    }
    let layout = layout?;

    // Per-corner layout coordinates, in layout-layer units.
    let mut at = vec![Vec2::ZERO; tri_count * 3];
    let mut flat = vec![false; tri_count];
    let mut facing = vec![0i8; tri_count];
    for t in 0..tri_count {
        let p = [
            vert(t, 0).uvs[layout],
            vert(t, 1).uvs[layout],
            vert(t, 2).uvs[layout],
        ];
        let a = 0.5 * cross2(p[1] - p[0], p[2] - p[0]);
        let r = if area3[t] > 0.0 { a.abs() / area3[t] } else { 0.0 };
        if r >= layout_scale / STRETCH && r <= layout_scale * STRETCH {
            at[t * 3..t * 3 + 3].copy_from_slice(&p);
            facing[t] = if a > 0.0 { 1 } else { -1 };
            continue;
        }
        flat[t] = true;
        let e1 = vert(t, 1).orig_position - vert(t, 0).orig_position;
        let e2 = vert(t, 2).orig_position - vert(t, 0).orig_position;
        let len = length3(e1);
        let unit = layout_scale.sqrt();
        at[t * 3] = Vec2::ZERO;
        at[t * 3 + 1] = Vec2::new(len, 0.0) * unit;
        at[t * 3 + 2] = if len > 0.0 {
            Vec2::new(crate::glmath::dot3(e2, e1), length3(cross3(e1, e2))) / len * unit
        } else {
            Vec2::ZERO
        };
    }

    let mut edges: BTreeMap<(u16, u16), Vec<u32>> = BTreeMap::new();
    for t in 0..tri_count {
        for k in 0..3 {
            let a = indices[t * 3 + k];
            let b = indices[t * 3 + (k + 1) % 3];
            edges.entry((a.min(b), a.max(b))).or_default().push(t as u32);
        }
    }
    let inside = |q: Vec2, t: usize| -> bool {
        let (a, b, c) = (at[t * 3], at[t * 3 + 1], at[t * 3 + 2]);
        let area = cross2(b - a, c - a);
        if area.abs() < 1e-12 {
            return false;
        }
        let w0 = cross2(b - q, c - q) / area;
        let w1 = cross2(c - q, a - q) / area;
        w0 > 1e-3 && w1 > 1e-3 && 1.0 - w0 - w1 > 1e-3
    };
    let centre = |t: usize| (at[t * 3] + at[t * 3 + 1] + at[t * 3 + 2]) / 3.0;
    let mut charts: Vec<Chart> = Vec::new();
    let mut chart_of = vec![-1i32; tri_count];
    for seed in 0..tri_count {
        if chart_of[seed] >= 0 {
            continue;
        }
        let id = charts.len();
        let mut tris = vec![seed as u32];
        chart_of[seed] = id as i32;
        if !flat[seed] {
            let mut next = 0;
            while next < tris.len() {
                let t = tris[next] as usize;
                for k in 0..3 {
                    let a = indices[t * 3 + k];
                    let b = indices[t * 3 + (k + 1) % 3];
                    let Some(neighbours) = edges.get(&(a.min(b), a.max(b))) else {
                        continue;
                    };
                    for &n in neighbours {
                        let n = n as usize;
                        if chart_of[n] >= 0 || flat[n] || facing[n] != facing[seed] {
                            continue;
                        }
                        let folded = tris.iter().any(|&o| {
                            let o = o as usize;
                            inside(centre(n), o) || inside(centre(o), n)
                        });
                        if folded {
                            continue;
                        }
                        chart_of[n] = id as i32;
                        tris.push(n as u32);
                    }
                }
                next += 1;
            }
        }
        charts.push(Chart {
            tris,
            ..Default::default()
        });
    }

    // Texels each chart wants per layout unit, read the way LayerDensity reads them.
    let density = |tris: &[u32]| -> Vec2 {
        let mut need = Vec2::ZERO;
        let mut du_area = Vec::new();
        let mut dv_area = Vec::new();
        for s in &layers {
            let Some(img) = s.ready() else {
                continue;
            };
            let tex = Vec2::new(img.width as f32, img.height as f32);
            du_area.clear();
            dv_area.clear();
            let mut total = 0.0f32;
            for &t in tris {
                let t = t as usize;
                let e1 = at[t * 3 + 1] - at[t * 3];
                let e2 = at[t * 3 + 2] - at[t * 3];
                let det = cross2(e1, e2);
                if det.abs() < 1e-12 {
                    continue;
                }
                let f1 = vert(t, 1).uvs[s.uv_layer] - vert(t, 0).uvs[s.uv_layer];
                let f2 = vert(t, 2).uvs[s.uv_layer] - vert(t, 0).uvs[s.uv_layer];
                total += gradients(&mut du_area, &mut dv_area, e1, e2, det, f1, f2, tex);
            }
            need = max2(
                need,
                Vec2::new(percentile(&mut du_area, total), percentile(&mut dv_area, total)),
            );
        }
        need
    };
    let every: Vec<u32> = (0..tri_count as u32).collect();
    let overall = max2(density(&every), Vec2::ONE);
    let mut wanted = 0.0f64;
    for chart in charts.iter_mut() {
        let mut lo = Vec2::splat(1e30);
        let mut hi = Vec2::splat(-1e30);
        for &t in &chart.tris {
            for k in 0..3 {
                lo = min2(lo, at[t as usize * 3 + k]);
                hi = max2(hi, at[t as usize * 3 + k]);
            }
        }
        chart.lo = lo;
        chart.extent = hi - lo;
        // One sliver with a wild gradient must not shrink everything else.
        let d = density(&chart.tris);
        let (dlo, dhi) = (overall * 0.25, overall * 2.0);
        chart.density = Vec2::new(fmin(fmax(d.x, dlo.x), dhi.x), fmin(fmax(d.y, dlo.y), dhi.y));
        let px = chart.extent * chart.density;
        wanted += (f64::from(px.x) + f64::from(2 * ATLAS_PADDING))
            * (f64::from(px.y) + f64::from(2 * ATLAS_PADDING));
    }

    // Shelf packing, shrinking everything together until the lot fits.
    let mut order: Vec<usize> = (0..charts.len()).collect();
    let mut side = 64i32;
    while side < size_cap && f64::from(side) * f64::from(side) * 0.6 < wanted {
        side *= 2;
    }
    side = side.min(64.max(size_cap));
    let mut scale = (f64::from(side) * f64::from(side) * 0.8 / wanted.max(1.0))
        .sqrt()
        .min(1.0) as f32;
    let mut used_h;
    let mut attempt = 0;
    loop {
        for chart in charts.iter_mut() {
            let s = (chart.extent * chart.density * scale).ceil();
            chart.size = (
                (s.x as i32).max(1) + 2 * ATLAS_PADDING,
                (s.y as i32).max(1) + 2 * ATLAS_PADDING,
            );
        }
        msvc_sort::sort_by(&mut order, |&a, &b| charts[a].size.1 > charts[b].size.1);
        let (mut x, mut y, mut shelf) = (0i32, 0i32, 0i32);
        let mut fits = true;
        for &i in &order {
            let chart = &mut charts[i];
            if chart.size.0 > side {
                fits = false;
                break;
            }
            if x + chart.size.0 > side {
                x = 0;
                y += shelf;
                shelf = 0;
            }
            chart.pos = (x, y);
            x += chart.size.0;
            shelf = shelf.max(chart.size.1);
        }
        used_h = y + shelf;
        if fits && used_h <= side {
            break;
        }
        if attempt == 63 {
            return None;
        }
        attempt += 1;
        scale *= 0.93;
    }
    let mut height = 64i32;
    while height < used_h {
        height *= 2;
    }
    let atlas_size = (side, height);

    let mut out_verts: Vec<SkinVertex> = Vec::with_capacity(verts.len() + verts.len() / 4);
    let mut out_indices = vec![0u16; indices.len()];
    for chart in &charts {
        let mut moved: HashMap<u16, u16> = HashMap::new();
        let inner = Vec2::new(
            (chart.size.0 - 2 * ATLAS_PADDING) as f32,
            (chart.size.1 - 2 * ATLAS_PADDING) as f32,
        );
        for &t in &chart.tris {
            let t = t as usize;
            for k in 0..3 {
                let from = indices[t * 3 + k];
                let existing = if flat[t] { None } else { moved.get(&from).copied() };
                if let Some(to) = existing {
                    out_indices[t * 3 + k] = to;
                    continue;
                }
                if out_verts.len() >= 65535 {
                    return None;
                }
                let mut v = verts[usize::from(from)];
                let mut q = at[t * 3 + k] - chart.lo;
                if chart.extent.x > 0.0 {
                    q.x *= inner.x / chart.extent.x;
                }
                if chart.extent.y > 0.0 {
                    q.y *= inner.y / chart.extent.y;
                }
                v.uvs[ATLAS_UV_LAYER] = (Vec2::new(
                    (chart.pos.0 + ATLAS_PADDING) as f32,
                    (chart.pos.1 + ATLAS_PADDING) as f32,
                ) + q)
                    / Vec2::new(atlas_size.0 as f32, atlas_size.1 as f32);
                let to = out_verts.len() as u16;
                out_verts.push(v);
                out_indices[t * 3 + k] = to;
                if !flat[t] {
                    moved.insert(from, to);
                }
            }
        }
    }
    *verts = out_verts;
    *indices = out_indices;
    Some(atlas_size)
}

/// BakeBatch: bakes in the recipe's own UV set, or in an atlas when patches sharing its texels want different art.
pub fn bake_batch(
    recipe: &mut MaterialRecipe,
    verts: &mut Vec<SkinVertex>,
    indices: &mut Vec<u16>,
    size_hint: i32,
    label: Option<&str>,
) -> Image {
    if recipe.kind != ShaderKind::Body || recipe.single_uv_set() {
        return bake_material(recipe.clone(), verts, indices, size_hint, (0, 0), None);
    }
    // Clashes show in a small bake as well, and a tiled layer makes the full one slow.
    let mut clash = 0.0f32;
    bake_material(
        recipe.clone(),
        verts,
        indices,
        size_hint.min(128),
        (0, 0),
        Some(&mut clash),
    );
    let before = verts.len();
    if clash < ATLAS_CLASH {
        return bake_material(recipe.clone(), verts, indices, size_hint, (0, 0), None);
    }
    let Some(size) = build_atlas(recipe, verts, indices, size_hint.max(64)) else {
        return bake_material(recipe.clone(), verts, indices, size_hint, (0, 0), None);
    };
    if let Some(label) = label {
        println!(
            "  {label}: uv{} patches disagree on {:.0}% of the texels, rebaked as a {}x{} atlas ({before} -> {} vertices)",
            recipe.out_uv_layer,
            f64::from(clash * 100.0),
            size.0,
            size.1,
            verts.len()
        );
    }
    recipe.out_uv_layer = ATLAS_UV_LAYER;
    bake_material(recipe.clone(), verts, indices, size_hint, size, None)
}

/// A baked diffuse plus what avatar.json says about it; the image stays for the preview renderer.
#[derive(Clone, Debug, Default)]
pub struct BakedMaterial {
    pub name: String,
    pub file: String,
    pub image: Rc<Image>,
    pub shader_id: u32,
    pub has_alpha: bool,
    pub uv_layer: usize,
    pub component_guid: String,
}

pub fn shader_name(id: u32) -> &'static str {
    match id {
        0 => "BODY_OPAQUE",
        1 => "BODY_TRANSPARENT",
        2 => "BODY_SHINY_OPAQUE",
        3 => "BODY_SHINY_TRANSPARENT",
        4 => "HEAD_OPAQUE",
        _ => "UNKNOWN",
    }
}

/// Head batch bookkeeping for face frames and per-clip face composites.
#[derive(Clone, Debug)]
pub struct HeadBatchInfo {
    pub mesh_index: usize,
    pub recipe: MaterialRecipe,
    pub material_index: usize,
    /// 9 = left eye, 10 = right eye.
    pub eye_usage: u32,
    /// 7 = left brow, 8 = right brow.
    pub brow_usage: u32,
}

/// One written face PNG: channel, frame and path relative to the output directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameFile {
    pub channel: String,
    pub frame: u32,
    pub file: String,
}

/// The face/ stage: every mouth, brow and eye layer tinted with the avatar's colours, plus whole-head
/// composites per layer when `face_frame_size` is set.
pub fn write_face_frames(
    out_dir: &Path,
    colors: &[u32; 9],
    rep_layers: &[Vec<Rc<Image>>; 6],
    head_batches: &[HeadBatchInfo],
    meshes: &[BakedMesh],
    materials: &[BakedMaterial],
    face_frame_size: Option<i32>,
) -> (Vec<FrameFile>, Vec<FrameFile>) {
    struct Chan {
        name: &'static str,
        slot: usize,
        tint: usize,
    }
    const CHANS: [Chan; 3] = [
        Chan {
            name: "mouth",
            slot: 0,
            tint: 2,
        },
        Chan {
            name: "brows",
            slot: 2,
            tint: 4,
        },
        Chan {
            name: "eyes",
            slot: 1,
            tint: 3,
        },
    ];
    let mut layer_files = Vec::new();
    let mut composite_files = Vec::new();
    let skin = f4(&color_to_float4(colors[0]));
    for ch in &CHANS {
        let layers = &rep_layers[ch.slot];
        let tint = f4(&color_to_float4(colors[ch.tint]));
        for (f, layer) in layers.iter().enumerate() {
            if !layer.ok {
                continue;
            }
            let mut tinted = Image {
                width: layer.width,
                height: layer.height,
                rgba: vec![0; layer.rgba.len()],
                ok: true,
            };
            for (dst, src) in tinted
                .rgba
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(layer.rgba.as_chunks::<4>().0)
            {
                let t = Vec4::new(
                    f32::from(src[0]),
                    f32::from(src[1]),
                    f32::from(src[2]),
                    f32::from(src[3]),
                ) / 255.0;
                let rgb = clamp3(tint * t.x + Vec3::splat(t.y) + skin * t.z, 0.0, 1.0);
                dst[0] = (rgb.x * 255.0).round() as u8;
                dst[1] = (rgb.y * 255.0).round() as u8;
                dst[2] = (rgb.z * 255.0).round() as u8;
                dst[3] = src[3];
            }
            let rel = format!("face/{}_{:02}.png", ch.name, f);
            if write_png(&out_dir.join(&rel), &tinted) {
                layer_files.push(FrameFile {
                    channel: ch.name.into(),
                    frame: f as u32,
                    file: rel,
                });
            }
            // Whole-head composite with this channel at frame f.
            let Some(size) = face_frame_size else {
                continue;
            };
            for hb in head_batches {
                let mut r = hb.recipe.clone();
                let target = match ch.slot {
                    0 => &mut r.mouth,
                    2 => &mut r.brow,
                    _ => &mut r.eye,
                };
                if target.img.is_none() {
                    continue;
                }
                target.img = Some(layer.clone());
                let m = &meshes[hb.mesh_index];
                let baked = bake_material(r, &m.verts, &m.indices, size, (0, 0), None);
                let crel = format!(
                    "face/{}_{}_{:02}.png",
                    materials[hb.material_index].name, ch.name, f
                );
                if write_png(&out_dir.join(&crel), &baked) {
                    composite_files.push(FrameFile {
                        channel: ch.name.into(),
                        frame: f as u32,
                        file: crel,
                    });
                }
            }
        }
    }
    (layer_files, composite_files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, p: [u8; 4]) -> Rc<Image> {
        Rc::new(Image {
            width: w,
            height: h,
            rgba: p.repeat((w * h) as usize),
            ok: true,
        })
    }

    #[test]
    fn single_uv_bake_is_the_layer() {
        let recipe = MaterialRecipe {
            color: Sampler {
                img: Some(solid(8, 4, [10, 20, 30, 255])),
                flags: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        let img = bake_material(recipe, &[], &[], 64, (0, 0), None);
        assert_eq!((img.width, img.height), (8, 4));
        assert!(img.rgba.chunks(4).all(|p| p == [10, 20, 30, 255]));
    }

    #[test]
    fn sampler_clamps_and_borders() {
        let s = Sampler {
            img: Some(solid(2, 2, [255, 0, 0, 255])),
            transparent_border: true,
            ..Default::default()
        };
        assert_eq!(s.sample(Vec2::new(1.5, 0.5)), Vec4::ZERO);
        assert_eq!(s.sample(Vec2::new(0.5, 0.5)), Vec4::new(1.0, 0.0, 0.0, 1.0));
    }
}
