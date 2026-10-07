//! Ports the Json writer of avatar_export.cpp and the avatar.json / --avatar-info layouts of Run.
//! Keys come out in the C++ order with the same fixed decimals per field, from a `scene::Scene`.

use std::fmt::Write as _;

use avatar_export::scene::{self, Scene};

/// JsonEscape: quotes, backslashes and control characters; everything else passes through.
fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Mirrors the C++ Json class: comma bookkeeping per nesting level, `%.*f` numbers.
pub struct Json {
    out: String,
    first: bool,
    stack: Vec<bool>,
}

impl Default for Json {
    fn default() -> Self {
        Json {
            out: String::new(),
            first: true,
            stack: Vec::new(),
        }
    }
}

impl Json {
    pub fn into_string(self) -> String {
        self.out
    }

    fn str_raw(&mut self, s: &str) {
        self.out.push('"');
        escape(s, &mut self.out);
        self.out.push('"');
    }

    fn comma(&mut self) {
        if !self.first {
            self.out.push(',');
        }
        self.first = false;
    }

    pub fn key(&mut self, k: &str) {
        self.comma();
        self.str_raw(k);
        self.out.push(':');
        self.first = true;
    }

    pub fn begin_obj(&mut self) {
        self.out.push('{');
        self.stack.push(self.first);
        self.first = true;
    }

    pub fn end_obj(&mut self) {
        self.out.push('}');
        self.first = false;
        self.stack.pop();
    }

    pub fn begin_arr(&mut self) {
        self.out.push('[');
        self.stack.push(self.first);
        self.first = true;
    }

    pub fn end_arr(&mut self) {
        self.out.push(']');
        self.first = false;
        self.stack.pop();
    }

    /// An element that opens a nested object or array.
    pub fn elem_obj(&mut self) {
        self.comma();
        self.begin_obj();
    }

    pub fn elem_arr(&mut self) {
        self.comma();
        self.begin_arr();
    }

    pub fn int(&mut self, v: i64) {
        self.comma();
        let _ = write!(self.out, "{v}");
    }

    pub fn boolean(&mut self, v: bool) {
        self.comma();
        self.out.push_str(if v { "true" } else { "false" });
    }

    pub fn null(&mut self) {
        self.comma();
        self.out.push_str("null");
    }

    fn push_num(&mut self, v: f64, decimals: usize) {
        let v = if v.is_finite() { v } else { 0.0 };
        let _ = write!(self.out, "{v:.decimals$}");
    }

    pub fn num(&mut self, v: f64, decimals: usize) {
        self.comma();
        self.push_num(v, decimals);
    }

    pub fn s(&mut self, v: &str) {
        self.comma();
        self.str_raw(v);
    }

    pub fn key_str(&mut self, k: &str, v: &str) {
        self.key(k);
        self.s(v);
    }

    pub fn key_int(&mut self, k: &str, v: i64) {
        self.key(k);
        self.int(v);
    }

    pub fn key_bool(&mut self, k: &str, v: bool) {
        self.key(k);
        self.boolean(v);
    }

    pub fn key_num(&mut self, k: &str, v: f64, decimals: usize) {
        self.key(k);
        self.num(v, decimals);
    }

    pub fn vec(&mut self, v: &[f32], decimals: usize) {
        self.comma();
        self.out.push('[');
        for (i, &x) in v.iter().enumerate() {
            if i != 0 {
                self.out.push(',');
            }
            self.push_num(f64::from(x), decimals);
        }
        self.out.push(']');
    }

    pub fn key_vec(&mut self, k: &str, v: &[f32], decimals: usize) {
        self.key(k);
        self.vec(v, decimals);
    }
}

const FACE_SLOT_ORDER: [&str; 6] = ["mouth", "eyes", "brows", "face_paint", "eye_shadow", "face"];
const LAYER_NAME_ORDER: [&str; 3] = ["eyes", "mouth", "brows"];

fn write_track(j: &mut Json, joint: Option<u32>, t: &[[f32; 3]], r: &[[f32; 4]], s: Option<&[[f32; 3]]>) {
    j.elem_obj();
    if let Some(joint) = joint {
        j.key_int("joint", i64::from(joint));
    }
    j.key("t");
    j.begin_arr();
    for v in t {
        j.vec(v, 5);
    }
    j.end_arr();
    j.key("r");
    j.begin_arr();
    for v in r {
        j.vec(v, 6);
    }
    j.end_arr();
    if let Some(s) = s {
        j.key("s");
        j.begin_arr();
        for v in s {
            j.vec(v, 5);
        }
        j.end_arr();
    }
    j.end_obj();
}

fn write_animation(j: &mut Json, a: &scene::Animation) {
    j.elem_obj();
    j.key_str("name", &a.name);
    j.key_num("fps", f64::from(a.fps), 3);
    j.key_int("frame_count", i64::from(a.frame_count));
    j.key_int("joint_count", i64::from(a.joint_count));
    j.key_int("carryable_joint_count", i64::from(a.carryable_joint_count));
    j.key_int("motion_count", i64::from(a.motion_count));
    j.key_int("texture_count", i64::from(a.texture_count));
    j.key_num("duration", f64::from(a.duration), 4);
    j.key_str("translation_mode", &a.translation_mode);
    j.key("tracks");
    j.begin_arr();
    for t in &a.tracks {
        write_track(j, Some(t.joint), &t.t, &t.r, Some(&t.s));
    }
    j.end_arr();
    j.key("carryable_tracks");
    j.begin_arr();
    for t in &a.carryable_tracks {
        write_track(j, Some(t.joint), &t.t, &t.r, Some(&t.s));
    }
    j.end_arr();
    j.key("motion");
    j.begin_arr();
    for m in &a.motion {
        write_track(j, None, &m.t, &m.r, None);
    }
    j.end_arr();
    j.key("face");
    j.begin_obj();
    if let Some(face) = &a.face {
        let channels: [(&str, &Vec<u32>); 5] = [
            ("mouth", &face.mouth),
            ("brow_left", &face.brow_left),
            ("brow_right", &face.brow_right),
            ("eye_left", &face.eye_left),
            ("eye_right", &face.eye_right),
        ];
        for (name, values) in channels.iter().take((a.texture_count as usize).min(5)) {
            j.key(name);
            j.begin_arr();
            for &v in values.iter() {
                j.int(i64::from(v));
            }
            j.end_arr();
        }
    }
    j.end_obj();
    j.end_obj();
}

fn write_mesh(j: &mut Json, m: &scene::Mesh) {
    j.elem_obj();
    j.key_str("name", &m.name);
    j.key_int("component", i64::from(m.component));
    j.key_int("material", i64::from(m.material));
    j.key_bool("is_prop", m.is_prop);
    j.key_int("vertex_count", i64::from(m.vertex_count));
    j.key_int("triangle_count", i64::from(m.triangle_count));
    j.key_int("uv_count", i64::from(m.uv_count));
    let floats = |j: &mut Json, key: &str, v: &[f32], d: usize| {
        j.key(key);
        j.begin_arr();
        for &x in v {
            j.num(f64::from(x), d);
        }
        j.end_arr();
    };
    floats(j, "positions", &m.positions, 5);
    floats(j, "normals", &m.normals, 4);
    floats(j, "uv", &m.uv, 5);
    if m.uv_count > 1 {
        j.key("uv_layers");
        j.begin_arr();
        for layer in &m.uv_layers {
            j.elem_arr();
            for &x in layer {
                j.num(f64::from(x), 5);
            }
            j.end_arr();
        }
        j.end_arr();
    }
    j.key("colors");
    j.begin_arr();
    for &c in &m.colors {
        j.int(i64::from(c));
    }
    j.end_arr();
    j.key("joints");
    j.begin_arr();
    for &c in &m.joints {
        j.int(i64::from(c));
    }
    j.end_arr();
    floats(j, "weights", &m.weights, 4);
    j.key("indices");
    j.begin_arr();
    for &i in &m.indices {
        j.int(i64::from(i));
    }
    j.end_arr();
    j.end_obj();
}

/// avatar.json text (with the trailing newline the C++ writes).
pub fn scene_to_json(scene: &Scene) -> String {
    let mut j = Json::default();
    j.begin_obj();
    j.key_str("format", &scene.format);
    j.key_int("version", i64::from(scene.version));
    j.key("source");
    j.begin_obj();
    j.key_str("manifest", &scene.source.manifest);
    j.key_str("pack", &scene.source.pack);
    j.key_str("closet", &scene.source.closet);
    j.end_obj();
    j.key("axes");
    j.begin_obj();
    j.key_str("units", &scene.axes.units);
    j.key_str("up", &scene.axes.up);
    j.key_str("forward", &scene.axes.forward);
    j.key_str("left", &scene.axes.left);
    j.key_str("handedness", &scene.axes.handedness);
    j.key_str("uv_origin", &scene.axes.uv_origin);
    j.end_obj();
    j.key("avatar");
    j.begin_obj();
    j.key_str("body_type", &scene.avatar.body_type);
    j.key_num("height_factor", f64::from(scene.avatar.height_factor), 4);
    j.key_num("weight_factor", f64::from(scene.avatar.weight_factor), 4);
    j.key_bool("scale_applied", scene.avatar.scale_applied);
    j.key("colors");
    j.begin_arr();
    for c in &scene.avatar.colors {
        j.s(c);
    }
    j.end_arr();
    j.key("color_names");
    j.begin_arr();
    for c in &scene.avatar.color_names {
        j.s(c);
    }
    j.end_arr();
    j.end_obj();

    j.key("skeleton");
    j.begin_obj();
    j.key_int("version", i64::from(scene.skeleton.version));
    j.key("joints");
    j.begin_arr();
    for jt in &scene.skeleton.joints {
        j.elem_obj();
        j.key_str("name", &jt.name);
        j.key_int("parent", i64::from(jt.parent));
        j.key_vec("bind_world", &jt.bind_world, 5);
        j.key_vec("rest_world", &jt.rest_world, 5);
        j.key_vec("rest_world_rot", &jt.rest_world_rot, 6);
        j.key_vec("rest_local", &jt.rest_local, 5);
        j.key_vec("scale", &jt.scale, 4);
        j.end_obj();
    }
    j.end_arr();
    j.end_obj();

    j.key("components");
    j.begin_arr();
    for c in &scene.components {
        j.elem_obj();
        j.key_int("index", i64::from(c.index));
        j.key_str("slot", &c.slot);
        j.key_str("guid", &c.guid);
        j.key_str("name", &c.name);
        j.key_int("categories", i64::from(c.categories));
        j.key_str("category_names", &c.category_names);
        j.key_str("source", &c.source);
        j.key("notes");
        j.begin_arr();
        for n in &c.notes {
            j.s(n);
        }
        j.end_arr();
        j.end_obj();
    }
    j.end_arr();

    j.key("materials");
    j.begin_arr();
    for m in &scene.materials {
        j.elem_obj();
        j.key_str("name", &m.name);
        j.key_str("diffuse", &m.diffuse);
        j.key_int("shader", i64::from(m.shader));
        j.key_str("shader_name", &m.shader_name);
        j.key_bool("has_alpha", m.has_alpha);
        j.key_bool("alpha_mask", m.alpha_mask);
        j.key_bool("double_sided", m.double_sided);
        j.key_int("uv_layer", i64::from(m.uv_layer));
        j.key_str("component_guid", &m.component_guid);
        j.end_obj();
    }
    j.end_arr();

    j.key("meshes");
    j.begin_arr();
    for m in &scene.meshes {
        write_mesh(&mut j, m);
    }
    j.end_arr();

    j.key("prop_skeleton");
    match &scene.prop_skeleton {
        Some(sk) => {
            j.begin_obj();
            j.key("joints");
            j.begin_arr();
            for jt in &sk.joints {
                j.elem_obj();
                j.key_str("name", &jt.name);
                j.key_int("parent", i64::from(jt.parent));
                j.key_vec("rest_world", &jt.rest_world, 5);
                j.key_vec("rest_world_rot", &jt.rest_world_rot, 6);
                j.key_vec("rest_local", &jt.rest_local, 5);
                j.end_obj();
            }
            j.end_arr();
            j.end_obj();
        }
        None => j.null(),
    }

    j.key("face");
    j.begin_obj();
    let face = scene.face.clone().unwrap_or_default();
    j.key("head_materials");
    j.begin_arr();
    for n in &face.head_materials {
        j.s(n);
    }
    j.end_arr();
    j.key("slots");
    j.begin_obj();
    for name in FACE_SLOT_ORDER {
        let Some(slot) = face.slots.get(name) else {
            continue;
        };
        j.key(name);
        j.begin_obj();
        j.key_str("guid", &slot.guid);
        j.key_str("name", &slot.name);
        j.key_int("layers", i64::from(slot.layers));
        j.key_int("width", i64::from(slot.width));
        j.key_int("height", i64::from(slot.height));
        j.end_obj();
    }
    j.end_obj();
    for (key, files) in [
        ("layer_files", &face.layer_files),
        ("composite_files", &face.composite_files),
    ] {
        j.key(key);
        j.begin_arr();
        for f in files {
            j.elem_obj();
            j.key_str("channel", &f.channel);
            j.key_int("frame", i64::from(f.frame));
            j.key_str("file", &f.file);
            j.end_obj();
        }
        j.end_arr();
    }
    j.key("layer_names");
    j.begin_obj();
    for name in LAYER_NAME_ORDER {
        j.key(name);
        j.begin_arr();
        for n in face.layer_names.get(name).into_iter().flatten() {
            j.s(n);
        }
        j.end_arr();
    }
    j.end_obj();
    j.end_obj();

    j.key("animations");
    j.begin_arr();
    for a in &scene.animations {
        write_animation(&mut j, a);
    }
    j.end_arr();

    j.key("log");
    j.begin_arr();
    for l in &scene.log {
        j.s(l);
    }
    j.end_arr();
    j.end_obj();
    let mut out = j.into_string();
    out.push('\n');
    out
}

/// One `components` row of the --avatar-info summary.
pub struct InfoComponent {
    pub slot: String,
    pub guid: String,
    pub name: String,
    pub categories: u32,
    pub category_names: String,
    pub source: String,
    pub batches: usize,
}

/// One `face_textures` row of the --avatar-info summary.
pub struct InfoFaceTexture {
    pub slot: String,
    pub guid: String,
    pub name: String,
    pub layers: u32,
}

/// The --avatar-info summary.
pub struct Info {
    pub manifest: String,
    pub body_type: String,
    pub height_factor: f32,
    pub weight_factor: f32,
    pub colors: Vec<String>,
    pub components: Vec<InfoComponent>,
    pub face_textures: Vec<InfoFaceTexture>,
    /// Carryable guid and name.
    pub prop: Option<(String, String)>,
}

/// The --avatar-info JSON, without the newline Run prints after it.
pub fn info_to_json(info: &Info) -> String {
    let mut j = Json::default();
    j.begin_obj();
    j.key_str("manifest", &info.manifest);
    j.key_str("body_type", &info.body_type);
    j.key_num("height_factor", f64::from(info.height_factor), 4);
    j.key_num("weight_factor", f64::from(info.weight_factor), 4);
    j.key("colors");
    j.begin_arr();
    for c in &info.colors {
        j.s(c);
    }
    j.end_arr();
    j.key("components");
    j.begin_arr();
    for c in &info.components {
        j.elem_obj();
        j.key_str("slot", &c.slot);
        j.key_str("guid", &c.guid);
        j.key_str("name", &c.name);
        j.key_int("categories", i64::from(c.categories));
        j.key_str("category_names", &c.category_names);
        j.key_str("source", &c.source);
        j.key_int("batches", c.batches as i64);
        j.end_obj();
    }
    j.end_arr();
    j.key("face_textures");
    j.begin_arr();
    for f in &info.face_textures {
        j.elem_obj();
        j.key_str("slot", &f.slot);
        j.key_str("guid", &f.guid);
        j.key_str("name", &f.name);
        j.key_int("layers", i64::from(f.layers));
        j.end_obj();
    }
    j.end_arr();
    j.key("prop");
    match &info.prop {
        Some((guid, name)) => {
            j.begin_obj();
            j.key_str("guid", guid);
            j.key_str("name", name);
            j.end_obj();
        }
        None => j.null(),
    }
    j.end_obj();
    j.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_matches_c_layout() {
        let mut j = Json::default();
        j.begin_obj();
        j.key_str("a", "x\"\n\u{1}");
        j.key_num("b", -0.000001, 4);
        j.key("c");
        j.begin_arr();
        j.vec(&[1.0, f32::NAN], 2);
        j.int(3);
        j.end_arr();
        j.key("d");
        j.null();
        j.end_obj();
        assert_eq!(
            j.into_string(),
            r#"{"a":"x\"\n\u0001","b":-0.0000,"c":[[1.00,0.00],3],"d":null}"#
        );
    }
}
