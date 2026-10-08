// Painter-built widgets reproducing web/css/aura.css and the 360css widget kit: gradients, cards, title bars,
// buttons, inputs, checkboxes, sliders, strips and the modal dialogs.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::epaint::text::{FontTweak, VariationCoords};
use eframe::egui::epaint::{Mesh, Vertex, WHITE_UV};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Margin,
    Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2,
};

/// Colours of aura.css `:root` and the rules that use them.
pub mod c {
    use eframe::egui::Color32;

    const fn hex(v: u32) -> Color32 {
        Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    pub const ROOT_BG: Color32 = hex(0xE8ECE4);
    pub const TEXT: Color32 = hex(0x33422D);
    pub const GREEN: Color32 = hex(0x477719);
    pub const LINE: Color32 = hex(0xCDD8C5);
    pub const MUTED: Color32 = hex(0x708064);
    pub const LABEL: Color32 = hex(0x657359);
    pub const TAB: Color32 = hex(0x6E7966);
    pub const TAB_ON: Color32 = hex(0x335C14);
    pub const INPUT_BG: Color32 = hex(0xFBFDF9);
    pub const INPUT_TEXT: Color32 = hex(0x384B2B);
    pub const PLACEHOLDER: Color32 = hex(0x757575);
    pub const FOCUS: Color32 = hex(0x598D2C);
    pub const STAGE_TEXT: Color32 = hex(0x738A60);
    pub const CAPTION: Color32 = hex(0x627650);
    pub const BUTTON_TEXT: Color32 = hex(0x525252);
    pub const ERROR: Color32 = hex(0xB03020);
}

const NOTO: &[u8] = include_bytes!("../fonts/NotoSans.ttf");
const YESTERDAY: &[u8] = include_bytes!("../fonts/Yesterday-Bold.ttf");

/// Noto Sans weights the stylesheet asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum W {
    Regular,
    Medium,
    Semibold,
    Heavy,
    Bold,
}

impl W {
    fn family(self) -> Option<&'static str> {
        match self {
            W::Regular => None,
            W::Medium => Some("noto-450"),
            W::Semibold => Some("noto-600"),
            W::Heavy => Some("noto-650"),
            W::Bold => Some("noto-700"),
        }
    }
}

pub fn font(size: f32, weight: W) -> FontId {
    match weight.family() {
        None => FontId::proportional(size),
        Some(name) => FontId::new(size, FontFamily::Name(name.into())),
    }
}

pub fn yesterday(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("yesterday".into()))
}

/// Noto Sans as the body font at each used weight, Yesterday Bold for the masthead, Consolas for logs when present.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let noto = |weight: f32| {
        FontData::from_static(NOTO).tweak(FontTweak {
            coords: VariationCoords::new([(b"wght", weight)]),
            ..Default::default()
        })
    };
    let fallbacks = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    defs.font_data.insert("noto-400".into(), Arc::new(noto(400.0)));
    if let Some(list) = defs.families.get_mut(&FontFamily::Proportional) {
        list.insert(0, "noto-400".into());
    }
    for (name, weight) in [
        ("noto-450", 450.0),
        ("noto-600", 600.0),
        ("noto-650", 650.0),
        ("noto-700", 700.0),
    ] {
        defs.font_data.insert(name.into(), Arc::new(noto(weight)));
        let mut list = vec![name.to_string()];
        list.extend(fallbacks.iter().cloned());
        defs.families.insert(FontFamily::Name(name.into()), list);
    }
    defs.font_data
        .insert("yesterday".into(), Arc::new(FontData::from_static(YESTERDAY)));
    let mut list = vec!["yesterday".to_string()];
    list.extend(fallbacks.iter().cloned());
    defs.families.insert(FontFamily::Name("yesterday".into()), list);
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\consola.ttf") {
        defs.font_data
            .insert("consolas".into(), Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = defs.families.get_mut(&FontFamily::Monospace) {
            list.insert(0, "consolas".into());
        }
    }
    ctx.set_fonts(defs);
}

pub fn rgba(v: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

// ---- gradients ----

/// A CSS background image: `linear-gradient(angle, ...)` or an ellipse `radial-gradient` at a fractional centre.
pub enum Paint<'a> {
    Linear(f32, &'a [(f32, Color32)]),
    Radial(Vec2, &'a [(f32, Color32)]),
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn eval(stops: &[(f32, Color32)], t: f32) -> Color32 {
    let Some(first) = stops.first() else {
        return Color32::TRANSPARENT;
    };
    if t <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            let span = (t1 - t0).max(1e-6);
            return mix(c0, c1, (t - t0) / span);
        }
    }
    stops[stops.len() - 1].1
}

/// Rounded-rect outline, clockwise from the top-left arc, with straight edges subdivided.
fn outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let corners = [
        (pos2(rect.min.x + r, rect.min.y + r), 180.0f32),
        (pos2(rect.max.x - r, rect.min.y + r), 270.0),
        (pos2(rect.max.x - r, rect.max.y - r), 0.0),
        (pos2(rect.min.x + r, rect.max.y - r), 90.0),
    ];
    let mut points = Vec::with_capacity(96);
    for (i, (center, start)) in corners.iter().enumerate() {
        let arc = if r > 0.0 { 8 } else { 0 };
        for k in 0..=arc {
            let a = (start + 90.0 * k as f32 / arc.max(1) as f32).to_radians();
            points.push(*center + vec2(a.cos(), a.sin()) * r);
        }
        let next = corners[(i + 1) % 4];
        let from = *points.last().unwrap_or(center);
        let a = next.1.to_radians();
        let to = next.0 + vec2(a.cos(), a.sin()) * r;
        for k in 1..12 {
            points.push(from + (to - from) * (k as f32 / 12.0));
        }
    }
    points
}

/// Gradient (or textured) fill of a rounded rect as one mesh of concentric rings around `center`.
fn ring_mesh(
    rect: Rect,
    radius: f32,
    center: Pos2,
    rings: usize,
    texture: Option<egui::TextureId>,
    color: impl Fn(Pos2) -> Color32,
) -> Mesh {
    let boundary = outline(rect, radius);
    let mut mesh = texture.map_or_else(Mesh::default, Mesh::with_texture);
    let uv = |p: Pos2| {
        if texture.is_some() {
            pos2(
                (p.x - rect.min.x) / rect.width(),
                (p.y - rect.min.y) / rect.height(),
            )
        } else {
            WHITE_UV
        }
    };
    mesh.vertices.push(Vertex {
        pos: center,
        uv: uv(center),
        color: color(center),
    });
    let n = boundary.len() as u32;
    for ring in 1..=rings {
        let s = ring as f32 / rings as f32;
        for b in &boundary {
            let p = center + (*b - center) * s;
            mesh.vertices.push(Vertex {
                pos: p,
                uv: uv(p),
                color: color(p),
            });
        }
    }
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.add_triangle(0, 1 + i, 1 + j);
    }
    for ring in 1..rings as u32 {
        let a = 1 + (ring - 1) * n;
        let b = 1 + ring * n;
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.add_triangle(a + i, b + i, b + j);
            mesh.add_triangle(a + i, b + j, a + j);
        }
    }
    mesh
}

/// The CSS gradient as a shape covering `rect` with corner `radius`.
pub fn gradient(rect: Rect, radius: f32, paint: &Paint<'_>) -> Shape {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return Shape::Noop;
    }
    match *paint {
        Paint::Linear(angle, stops) => {
            let a = angle.to_radians();
            let dir = vec2(a.sin(), -a.cos());
            let len = (rect.width() * a.sin()).abs() + (rect.height() * a.cos()).abs();
            let c = rect.center();
            Shape::mesh(ring_mesh(rect, radius, c, 16, None, |p| {
                eval(stops, (p - c).dot(dir) / len + 0.5)
            }))
        }
        Paint::Radial(at, stops) => {
            let c = rect.min + at * rect.size();
            let fx = (c.x - rect.min.x).max(rect.max.x - c.x).max(1.0);
            let fy = (c.y - rect.min.y).max(rect.max.y - c.y).max(1.0);
            let (rx, ry) = (fx * std::f32::consts::SQRT_2, fy * std::f32::consts::SQRT_2);
            Shape::mesh(ring_mesh(rect, radius, c, 16, None, |p| {
                let d = p - c;
                eval(stops, ((d.x / rx).powi(2) + (d.y / ry).powi(2)).sqrt())
            }))
        }
    }
}

pub fn fill(painter: &egui::Painter, rect: Rect, radius: f32, paint: &Paint<'_>) {
    painter.add(gradient(rect, radius, paint));
}

/// A texture clipped to a rounded rect.
pub fn image_rounded(painter: &egui::Painter, rect: Rect, radius: f32, texture: egui::TextureId) {
    painter.add(Shape::mesh(ring_mesh(
        rect,
        radius,
        rect.center(),
        1,
        Some(texture),
        |_| Color32::WHITE,
    )));
}

// ---- 360css palettes ----

const BTN: &[(f32, Color32)] = &[(0.0, rgb_c(0xE6E6E6)), (0.9, rgb_c(0xD4D4D4))];
const GREEN_UP: &[(f32, Color32)] = &[(0.0, rgb_c(0x738F32)), (1.0, rgb_c(0x8EDB46))];
const ORANGE_UP: &[(f32, Color32)] = &[(0.0, rgb_c(0xD57920)), (1.0, rgb_c(0xFFCB78))];
const RED_UP: &[(f32, Color32)] = &[(0.0, rgb_c(0xA83A2E)), (1.0, rgb_c(0xE5604F))];
const TITLE: &[(f32, Color32)] = &[
    (0.07, rgb_c(0x707070)),
    (0.63, rgb_c(0x999999)),
    (0.8, rgb_c(0xA3A3A3)),
];
const WELL: &[(f32, Color32)] = &[
    (0.0, rgb_c(0x8C8C8C)),
    (0.24, rgb_c(0xB8B8B8)),
    (0.62, rgb_c(0xE3E3E3)),
    (0.88, rgb_c(0xF0F0F0)),
];
const TRACK: &[(f32, Color32)] = &[
    (0.08, rgb_c(0xC4C4C4)),
    (0.46, rgb_c(0xC2C2C2)),
    (0.85, rgb_c(0xE6E6E6)),
];
const THUMB: &[(f32, Color32)] = &[
    (0.03, rgb_c(0x669636)),
    (0.5, rgb_c(0x97D14B)),
    (1.0, rgb_c(0xB8E031)),
];

const fn rgb_c(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The `.window` light grey body gradient.
pub fn window_body(rect: Rect) -> Shape {
    gradient(rect, 0.0, &Paint::Linear(180.0, BTN))
}

/// The `.window-title` bar with its caption.
pub fn title_bar(painter: &egui::Painter, rect: Rect, title: &str) {
    fill(painter, rect, 0.0, &Paint::Linear(0.0, TITLE));
    painter.text(
        pos2(rect.min.x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        font(13.0, W::Regular),
        Color32::WHITE,
    );
}

// ---- text ----

/// Row height of Noto Sans at `size`, the CSS `line-height: normal`.
pub fn line(size: f32) -> f32 {
    (size * 1.362).round()
}

pub fn text(ui: &mut Ui, text: impl Into<String>, size: f32, weight: W, color: Color32) -> Response {
    ui.add(egui::Label::new(RichText::new(text).font(font(size, weight)).color(color)).wrap())
}

/// `.muted` paragraph text: 12px, line height 1.5.
pub fn muted(ui: &mut Ui, s: impl Into<String>) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(s)
                .font(font(12.0, W::Regular))
                .color(c::MUTED)
                .line_height(Some(18.0)),
        )
        .wrap(),
    )
}

/// An `h3`: 15px, weight 600, 10px below.
pub fn h3(ui: &mut Ui, s: &str) {
    text(ui, s, 15.0, W::Semibold, c::TEXT);
    ui.add_space(10.0);
}

/// `label` text above a field.
pub fn field_label(ui: &mut Ui, s: &str) {
    text(ui, s, 12.0, W::Regular, c::LABEL);
}

/// The `.section-heading` block: h2 and its blurb.
pub fn section_heading(ui: &mut Ui, title: &str, blurb: &str) {
    ui.add(
        egui::Label::new(
            RichText::new(title)
                .font(font(24.0, W::Medium))
                .color(c::TEXT)
                .extra_letter_spacing(-0.5),
        )
        .truncate(),
    );
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(blurb).font(font(12.0, W::Regular)).color(c::MUTED)).truncate());
}

// ---- buttons ----

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Short,
    Primary,
    /// Dismiss buttons: red on hover.
    Close,
}

/// `.btn-short` (or `.btn-short.primary`) at the default 8px 10px padding.
pub fn button(ui: &mut Ui, label: &str, kind: Kind, enabled: bool) -> Response {
    button_padded(ui, label, kind, enabled, vec2(10.0, 8.0))
}

pub fn button_padded(ui: &mut Ui, label: &str, kind: Kind, enabled: bool, pad: Vec2) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font(13.0, W::Regular), c::BUTTON_TEXT);
    let size = vec2(galley.size().x + 2.0 * pad.x, line(13.0) + 2.0 * pad.y);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    paint_button(ui, rect, &galley, kind, enabled, &response);
    response
}

/// The size `button` would give `label`, for buttons that keep one width across label changes.
pub fn button_size(ui: &Ui, label: &str) -> Vec2 {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font(13.0, W::Regular), c::BUTTON_TEXT);
    vec2(galley.size().x + 20.0, line(13.0) + 16.0)
}

/// A button stretched to `rect`.
pub fn button_in(ui: &mut Ui, rect: Rect, label: &str, kind: Kind, enabled: bool) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font(13.0, W::Regular), c::BUTTON_TEXT);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, ui.id().with(("btn", label, rect.min.x as i32)), sense);
    paint_button(ui, rect, &galley, kind, enabled, &response);
    response
}

fn paint_button(
    ui: &Ui,
    rect: Rect,
    galley: &Arc<egui::Galley>,
    kind: Kind,
    enabled: bool,
    response: &Response,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let mut painter = ui.painter().clone();
    if !enabled {
        painter.multiply_opacity(0.45);
    }
    let hot = enabled && response.hovered();
    let (stops, angle, color) = match (hot, kind) {
        (true, Kind::Short) => (GREEN_UP, 0.0, Color32::WHITE),
        (true, Kind::Primary) => (ORANGE_UP, 0.0, rgb(0x261706)),
        (true, Kind::Close) => (RED_UP, 0.0, Color32::WHITE),
        _ => (BTN, 180.0, c::BUTTON_TEXT),
    };
    fill(&painter, rect, 0.0, &Paint::Linear(angle, stops));
    painter.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley.clone(), color);
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            0.0,
            Stroke::new(3.0, c::FOCUS),
            StrokeKind::Outside,
        );
    }
    if hot {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
}

// ---- inputs ----

/// Inputs sit inside their `label`, so they inherit its 12px font: 16px text row, 8px padding, 1px border.
pub const INPUT_H: f32 = 34.0;
const INPUT_FONT: f32 = 12.0;

/// Input styling for egui's own widgets (combo boxes, drag values) so they read as `select`/`input` fields.
pub fn input_style(ui: &mut Ui) {
    let v = ui.visuals_mut();
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = c::INPUT_BG;
        w.weak_bg_fill = c::INPUT_BG;
        w.bg_stroke = Stroke::new(1.0, c::LINE);
        w.corner_radius = egui::CornerRadius::same(4);
        w.fg_stroke = Stroke::new(1.0, c::INPUT_TEXT);
        w.expansion = 0.0;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, rgb(0xA8BC92));
    v.widgets.active.bg_stroke = Stroke::new(1.0, c::FOCUS);
    let s = ui.spacing_mut();
    s.interact_size.y = INPUT_H;
    s.button_padding = vec2(8.0, 8.0);
    ui.style_mut().override_font_id = Some(font(INPUT_FONT, W::Regular));
}

/// A text `input`: 1px line border, 4px radius, 8px padding.
pub fn input(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, INPUT_H), Sense::hover());
    let edit = egui::TextEdit::singleline(value)
        .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(9, 8)))
        .font(font(INPUT_FONT, W::Regular))
        .text_color(c::INPUT_TEXT)
        .hint_text(
            RichText::new(hint)
                .color(c::PLACEHOLDER)
                .font(font(INPUT_FONT, W::Regular)),
        )
        .desired_width(width - 18.0)
        .vertical_align(egui::Align::Center);
    ui.painter().rect(
        rect,
        4.0,
        c::INPUT_BG,
        Stroke::new(1.0, c::LINE),
        StrokeKind::Inside,
    );
    let response = ui.put(rect, edit);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            6.0,
            Stroke::new(3.0, c::FOCUS),
            StrokeKind::Outside,
        );
    }
    response
}

/// `label` with an input under it, 5px apart.
pub fn labelled_input(ui: &mut Ui, label: &str, value: &mut String, hint: &str) -> Response {
    field_label(ui, label);
    ui.add_space(5.0);
    let w = ui.available_width();
    input(ui, value, hint, w)
}

/// What a browse button opens.
#[derive(Clone, Copy)]
pub enum Pick {
    File,
    Folder,
}

/// `.field-row`: a labelled input taking the width and a Browse button on its baseline; true on change.
pub fn path_row(ui: &mut Ui, label: &str, value: &mut String, pick: Pick, hint: &str) -> bool {
    let mut changed = false;
    let clicked = field_row(ui, label, "Browse", true, |ui, w| {
        changed |= input(ui, value, hint, w).changed();
    });
    if clicked {
        if let Some(path) = browse(pick, value) {
            *value = path.display().to_string();
            changed = true;
        }
    }
    changed
}

/// `.field-row` with arbitrary field content of the given width; returns whether the button was clicked.
pub fn field_row(
    ui: &mut Ui,
    label: &str,
    button_label: &str,
    enabled: bool,
    field: impl FnOnce(&mut Ui, f32),
) -> bool {
    let bw = ui
        .painter()
        .layout_no_wrap(button_label.to_string(), font(13.0, W::Regular), c::BUTTON_TEXT)
        .size()
        .x
        + 18.0;
    let total = ui.available_width();
    let fw = (total - bw - 8.0).max(40.0);
    let top = ui.cursor().min;
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(fw);
            field_label(ui, label);
            ui.add_space(5.0);
            field(ui, fw);
        });
        let bottom = ui.min_rect().max.y;
        let h = line(13.0) + 18.0;
        let rect = Rect::from_min_size(pos2(top.x + fw + 8.0, bottom - h), vec2(bw, h));
        clicked = button_in(ui, rect, button_label, Kind::Short, enabled).clicked();
    });
    clicked
}

fn browse(pick: Pick, current: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new();
    let current = crate::paths::expand(current);
    let start = if current.is_dir() {
        Some(current)
    } else {
        current.parent().filter(|p| p.is_dir()).map(PathBuf::from)
    };
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    match pick {
        Pick::File => dialog.pick_file(),
        Pick::Folder => dialog.pick_folder(),
    }
}

/// A `select` showing `current` that lists `options`; returns the index picked.
pub fn select(ui: &mut Ui, id: &str, current: &str, options: &[String], width: f32) -> Option<usize> {
    let mut picked = None;
    ui.scope(|ui| {
        input_style(ui);
        egui::ComboBox::from_id_salt(id)
            .selected_text(
                RichText::new(current)
                    .font(font(INPUT_FONT, W::Regular))
                    .color(c::INPUT_TEXT),
            )
            .width(width)
            .height(400.0)
            .show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, option) in options.iter().enumerate() {
                    if ui
                        .selectable_label(
                            option == current,
                            RichText::new(option).font(font(INPUT_FONT, W::Regular)),
                        )
                        .clicked()
                    {
                        picked = Some(i);
                    }
                }
            });
    });
    picked
}

// ---- checkbox, slider, progress ----

/// `.checkbox-wrapper`: the 360css round checkbox, then the label text after a space.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str, size: f32, enabled: bool) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(format!(" {label}"), font(size, W::Regular), c::LABEL);
    let h = line(size).max(15.0);
    let width = 15.0 + 5.0 + galley.size().x;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, h), sense);
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    let mut painter = ui.painter().clone();
    let color = if size >= 14.0 { c::TEXT } else { c::LABEL };
    painter.galley_with_override_text_color(
        pos2(rect.min.x + 20.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if !enabled {
        painter.multiply_opacity(0.45);
    }
    let dot = Rect::from_min_size(pos2(rect.min.x, rect.center().y - 7.5), vec2(15.0, 15.0));
    let (stops, border) = if *checked {
        (GREEN_UP, rgb(0x738F32))
    } else {
        (WELL, rgb(0x8C8C8C))
    };
    fill(&painter, dot, 7.5, &Paint::Linear(0.0, stops));
    painter.circle_stroke(dot.center(), 6.5, Stroke::new(2.0, border));
    if response.has_focus() {
        painter.circle_stroke(dot.center(), 12.0, Stroke::new(3.0, c::GREEN));
    }
    if enabled && response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// The 360css `.slider`: a 6px pill track with a 15x25 green thumb, full width.
pub fn slider(ui: &mut Ui, value: &mut u32, max: u32, enabled: bool) -> Response {
    let w = ui.available_width();
    let (rect, mut response) = ui.allocate_exact_size(
        vec2(w, 10.0),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let hit = rect.expand2(vec2(0.0, 8.0));
    let response_thumb = ui.interact(
        hit,
        response.id.with("thumb"),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let travel = (w - 15.0).max(1.0);
    let pointer = [&response, &response_thumb]
        .iter()
        .find_map(|r| r.interact_pointer_pos().filter(|_| r.is_pointer_button_down_on()));
    if let Some(p) = pointer {
        let t = ((p.x - rect.min.x - 7.5) / travel).clamp(0.0, 1.0);
        let v = (t * max as f32).round() as u32;
        if v != *value {
            *value = v;
            response.mark_changed();
        }
    }
    let mut painter = ui.painter().clone();
    if !enabled {
        painter.multiply_opacity(0.45);
    }
    fill(&painter, rect, 5.0, &Paint::Linear(0.0, TRACK));
    painter.rect_stroke(rect, 5.0, Stroke::new(2.0, rgb(0xC4C4C4)), StrokeKind::Inside);
    let t = if max == 0 { 0.0 } else { *value as f32 / max as f32 };
    let thumb = Rect::from_min_size(
        pos2(rect.min.x + t * travel, rect.center().y - 12.5),
        vec2(15.0, 25.0),
    );
    fill(&painter, thumb, 5.0, &Paint::Linear(0.0, THUMB));
    if enabled && (response.hovered() || response_thumb.hovered()) {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// The 360css `.progress-bar` at full width with its green fill.
pub fn progress(ui: &mut Ui, fraction: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    fill(ui.painter(), rect, 0.0, &Paint::Linear(0.0, WELL));
    let done = Rect::from_min_size(rect.min, vec2(w * fraction.clamp(0.0, 1.0), 22.0));
    fill(ui.painter(), done, 0.0, &Paint::Linear(0.0, GREEN_UP));
}

// ---- cards ----

/// `.window.card` filling `rect`: grey body, title bar, 18px inset, vertical scrolling.
pub fn window_card<R>(ui: &mut Ui, rect: Rect, title: &str, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    ui.painter().add(window_body(rect));
    let mut out = None;
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        egui::ScrollArea::vertical()
            .id_salt(title)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let w = ui.available_width();
                let (bar, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
                title_bar(ui.painter(), bar, title);
                ui.add_space(14.0);
                egui::Frame::NONE
                    .inner_margin(Margin {
                        left: 18,
                        right: 18,
                        top: 0,
                        bottom: 18,
                    })
                    .show(ui, |ui| {
                        ui.set_width(w - 36.0);
                        out = Some(add(ui));
                    });
            });
    });
    out
}

// ---- strips ----

/// One choice of a strip.
pub struct Tile {
    pub key: String,
    pub label: String,
    pub thumb: Option<egui::TextureId>,
    pub badge: Option<String>,
}

const PER_PAGE: usize = 3;

/// `.strip`: arrow, three-tile page, arrow; the wheel pages too. Tiles ignore clicks unless `enabled`; returns the clicked index.
/// Dashed rounded outline, as `border: 1px dashed`.
pub fn dashed_rect(painter: &egui::Painter, rect: Rect, radius: f32, stroke: Stroke) {
    let mut points = outline(rect.shrink(0.5), radius);
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    painter.extend(Shape::dashed_line(&points, stroke, 3.0, 3.0));
}

/// Height the strip needs for the first page of `tiles` at `width`.
pub fn strip_height(ui: &Ui, tiles: &[Tile], width: f32) -> f32 {
    let label_w = tile_width(width) - 12.0;
    tiles
        .iter()
        .take(PER_PAGE)
        .map(|t| {
            let galley = ui.painter().layout_job(label_job(&t.label, label_w));
            tile_content(t, galley.size().y) + 12.0
        })
        .fold(96.0f32, f32::max)
}

fn tile_width(width: f32) -> f32 {
    let track_w = (width - 2.0 * 26.0 - 12.0).max(30.0);
    ((track_w - 14.0) / 3.0).max(10.0)
}

fn label_job(label: &str, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat {
            font_id: font(10.0, W::Regular),
            color: rgb(0x516440),
            line_height: Some(13.0),
            ..Default::default()
        },
    );
    job.wrap.max_width = width;
    job.wrap.break_anywhere = true;
    job.halign = egui::Align::Center;
    job
}

fn tile_content(tile: &Tile, label_h: f32) -> f32 {
    let mut h = label_h + 4.0 + 58.0;
    if tile.badge.is_some() {
        h += 4.0 + 13.0 + 6.0;
    }
    h
}

pub fn strip(
    ui: &mut Ui,
    id_salt: &str,
    tiles: &[Tile],
    selected: Option<&str>,
    enabled: bool,
) -> Option<usize> {
    let id = ui.id().with(id_salt);
    let selected_index = selected.and_then(|s| tiles.iter().position(|t| t.key == s));
    let (mut page, last) = ui
        .data(|d| d.get_temp::<(usize, Option<usize>)>(id))
        .unwrap_or((selected_index.unwrap_or(0) / PER_PAGE, selected_index));
    if last != selected_index {
        if let Some(i) = selected_index {
            page = i / PER_PAGE;
        }
    }
    let pages = tiles.len().div_ceil(PER_PAGE).max(1);
    page = page.min(pages - 1);

    let width = ui.available_width();
    let tile_w = tile_width(width);
    let start = page * PER_PAGE;
    let visible: Vec<(usize, &Tile)> = tiles.iter().enumerate().skip(start).take(PER_PAGE).collect();
    let label_w = tile_w - 12.0;
    let galleys: Vec<_> = visible
        .iter()
        .map(|(_, t)| ui.painter().layout_job(label_job(&t.label, label_w)))
        .collect();
    let content_h = |i: usize| tile_content(visible[i].1, galleys[i].size().y);
    let height = (0..visible.len())
        .map(|i| content_h(i) + 12.0)
        .fold(96.0f32, f32::max);
    let (rect, strip_response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());

    let mut clicked = None;
    let prev = Rect::from_min_size(rect.min, vec2(26.0, height));
    let next = Rect::from_min_size(pos2(rect.max.x - 26.0, rect.min.y), vec2(26.0, height));
    for (r, glyph, can, step) in [
        (prev, "\u{2039}", page > 0, -1i32),
        (next, "\u{203a}", page + 1 < pages, 1),
    ] {
        let resp = ui.interact(
            r,
            id.with(glyph),
            if can { Sense::click() } else { Sense::hover() },
        );
        let mut painter = ui.painter().clone();
        if !can {
            painter.multiply_opacity(0.45);
        }
        painter.rect_filled(r, 5.0, rgb(0xE3EDDA));
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            glyph,
            font(14.0, W::Regular),
            rgb(0x56733B),
        );
        let resp = resp.on_hover_text(if step < 0 {
            "Previous choices"
        } else {
            "Next choices"
        });
        if can && resp.clicked() {
            page = (page as i32 + step) as usize;
        }
        if can && resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }
    if strip_response.hovered() && pages > 1 {
        let delta = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => Some(*delta),
                    _ => None,
                })
                .fold(Vec2::ZERO, |a, b| a + b)
        });
        let d = if delta.x.abs() > delta.y.abs() {
            delta.x
        } else {
            delta.y
        };
        if d != 0.0 {
            if d < 0.0 && page + 1 < pages {
                page += 1;
            } else if d > 0.0 && page > 0 {
                page -= 1;
            }
        }
        ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    }

    let track_x = rect.min.x + 32.0;
    for (slot, ((i, tile), galley)) in visible.iter().zip(galleys.iter()).enumerate() {
        let r = Rect::from_min_size(
            pos2(track_x + slot as f32 * (tile_w + 7.0), rect.min.y),
            vec2(tile_w, height),
        );
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let resp = ui
            .interact(r, id.with(("tile", *i)), sense)
            .on_hover_text(&tile.label);
        if resp.clicked() {
            clicked = Some(*i);
        }
        if enabled && resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let painter = ui.painter().clone();
        let on = selected_index == Some(*i);
        let (stops, border): (&[(f32, Color32)], Color32) = if on {
            (&[(0.0, rgb_c(0xE8F7CC)), (1.0, rgb_c(0xC5E695))], rgb(0x548B23))
        } else {
            (&[(0.0, Color32::WHITE), (1.0, rgb_c(0xE9F1E0))], rgb(0xD8E3CE))
        };
        fill(&painter, r, 8.0, &Paint::Linear(180.0, stops));
        painter.rect_stroke(
            r,
            8.0,
            Stroke::new(if on { 3.0 } else { 2.0 }, border),
            StrokeKind::Inside,
        );
        let h = content_h(slot);
        let mut y = r.center().y - h / 2.0;
        if let Some(tex) = tile.thumb {
            let side = 58.0f32.min(label_w);
            let img = Rect::from_center_size(pos2(r.center().x, y + 29.0), vec2(side, side));
            painter.image(
                tex,
                img,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            y += 58.0;
        }
        y += 4.0;
        let gs = galley.size();
        painter.galley(pos2(r.center().x, y), galley.clone(), rgb(0x516440));
        y += gs.y;
        if let Some(badge) = &tile.badge {
            let g = painter.layout_no_wrap(badge.clone(), font(10.0, W::Bold), rgb(0x354D26));
            let b = Rect::from_min_size(
                pos2(r.center().x - g.size().x / 2.0 - 7.0, y + 4.0),
                vec2(g.size().x + 14.0, 13.0 + 6.0),
            );
            painter.rect(
                b,
                3.0,
                rgb(0xF5FAEE),
                Stroke::new(1.0, rgb(0x92A97E)),
                StrokeKind::Inside,
            );
            painter.galley(b.center() - g.size() / 2.0, g, rgb(0x354D26));
        }
        if on {
            let center = pos2(r.max.x - 2.0 - 3.0 - 8.5, r.min.y + 2.0 + 2.0 + 8.5);
            painter.circle_filled(center, 8.5, rgb(0x548B23));
            let s = Stroke::new(1.6, Color32::WHITE);
            painter.line_segment([center + vec2(-3.6, 0.2), center + vec2(-1.0, 2.8)], s);
            painter.line_segment([center + vec2(-1.0, 2.8), center + vec2(3.8, -2.8)], s);
        }
    }
    ui.data_mut(|d| d.insert_temp(id, (page, selected_index)));
    clicked
}

// ---- dialogs ----

const BACKDROP: Color32 = Color32::from_rgba_premultiplied(0x0B, 0x13, 0x0B, 0x66);

/// A `dialog.window`: modal, light grey body, title bar, 16px sides, Close at the bottom right. False once closed.
pub fn dialog(ctx: &egui::Context, id: &str, title: &str, add: impl FnOnce(&mut Ui)) -> bool {
    let width = (ctx.content_rect().width() * 0.9).min(760.0);
    let mut open = true;
    let modal = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(BACKDROP)
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let bg = ui.painter().add(Shape::Noop);
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let (bar, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
            title_bar(ui.painter(), bar, title);
            ui.add_space(12.0);
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: 16,
                    right: 16,
                    top: 0,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.set_width(width - 32.0);
                    add(ui);
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if button(ui, "Close", Kind::Close, true).clicked() {
                            open = false;
                        }
                    });
                });
            let rect = ui.min_rect().with_min_y(bar.min.y);
            ui.painter().set(bg, window_body(rect));
        });
    if modal.should_close() {
        open = false;
    }
    open
}

/// The shared `#log-dialog`: a page of the log sized to the dialog, with Previous and Next.
#[derive(Default)]
pub struct LogDialog {
    pub open: bool,
    page: usize,
}

impl LogDialog {
    pub fn open(&mut self) {
        self.open = true;
        self.page = 0;
    }

    pub fn show(&mut self, ctx: &egui::Context, title: &str, lines: &[String]) {
        if !self.open {
            return;
        }
        let screen = ctx.content_rect();
        let size = vec2(screen.width() * 0.94, screen.height() * 0.9);
        let mut open = true;
        let modal = egui::Modal::new(egui::Id::new("log-dialog"))
            .backdrop_color(BACKDROP)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().add(window_body(rect));
                let bar = Rect::from_min_size(rect.min, vec2(size.x, 44.0));
                fill(ui.painter(), bar, 0.0, &Paint::Linear(0.0, TITLE));
                ui.painter().text(
                    pos2(bar.min.x + 10.0, bar.center().y),
                    Align2::LEFT_CENTER,
                    title,
                    font(13.0, W::Regular),
                    Color32::WHITE,
                );
                let close_w = ui
                    .painter()
                    .layout_no_wrap("Close".into(), font(13.0, W::Regular), c::BUTTON_TEXT)
                    .size()
                    .x
                    + 20.0;
                let close = Rect::from_min_size(
                    pos2(
                        bar.max.x - 10.0 - close_w,
                        bar.center().y - (line(13.0) + 16.0) / 2.0,
                    ),
                    vec2(close_w, line(13.0) + 16.0),
                );
                if button_in(ui, close, "Close", Kind::Close, true).clicked() {
                    open = false;
                }
                let foot_h = line(13.0) + 16.0;
                let pre = Rect::from_min_max(
                    pos2(rect.min.x + 16.0, bar.max.y + 12.0),
                    pos2(rect.max.x - 16.0, rect.max.y - 16.0 - foot_h - 12.0),
                );
                ui.painter().rect_filled(pre, 5.0, rgb(0xEDF4E5));
                let mono = FontId::monospace(12.0);
                let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, 'M')).max(1.0);
                let columns = (((pre.width() - 24.0) / char_w).floor() as usize).max(10);
                let rows = (((pre.height() - 24.0) / 18.0).floor() as usize).max(1);
                let wrapped: Vec<String> = lines
                    .iter()
                    .flat_map(|l| l.split('\n'))
                    .flat_map(|l| {
                        let chars: Vec<char> = l.replace('\t', "    ").replace('\r', "").chars().collect();
                        if chars.is_empty() {
                            vec![String::new()]
                        } else {
                            chars.chunks(columns).map(|c| c.iter().collect()).collect()
                        }
                    })
                    .collect();
                let pages = wrapped.len().div_ceil(rows).max(1);
                self.page = self.page.min(pages - 1);
                let body: Vec<&str> = wrapped
                    .iter()
                    .skip(self.page * rows)
                    .take(rows)
                    .map(String::as_str)
                    .collect();
                let job = egui::text::LayoutJob::single_section(
                    body.join("\n"),
                    egui::TextFormat {
                        font_id: mono,
                        color: rgb(0x425C31),
                        line_height: Some(18.0),
                        ..Default::default()
                    },
                );
                let galley = ui.painter().layout_job(job);
                ui.painter().with_clip_rect(pre.shrink(12.0)).galley(
                    pre.min + vec2(12.0, 12.0),
                    galley,
                    rgb(0x425C31),
                );
                let fy = rect.max.y - 16.0 - foot_h;
                let prev_w = ui
                    .painter()
                    .layout_no_wrap("Previous".into(), font(13.0, W::Regular), c::BUTTON_TEXT)
                    .size()
                    .x
                    + 20.0;
                let next_w = ui
                    .painter()
                    .layout_no_wrap("Next".into(), font(13.0, W::Regular), c::BUTTON_TEXT)
                    .size()
                    .x
                    + 20.0;
                let prev = Rect::from_min_size(pos2(rect.min.x + 16.0, fy), vec2(prev_w, foot_h));
                let next = Rect::from_min_size(pos2(rect.max.x - 16.0 - next_w, fy), vec2(next_w, foot_h));
                if button_in(ui, prev, "Previous", Kind::Short, self.page > 0).clicked() {
                    self.page -= 1;
                }
                if button_in(ui, next, "Next", Kind::Short, self.page + 1 < pages).clicked() {
                    self.page += 1;
                }
                ui.painter().text(
                    pos2(rect.center().x, fy + foot_h / 2.0),
                    Align2::CENTER_CENTER,
                    format!("Page {} of {pages}", self.page + 1),
                    font(14.0, W::Regular),
                    c::TEXT,
                );
            });
        if modal.should_close() {
            open = false;
        }
        self.open = open;
    }
}

/// Lays `add` out in `rect` as a child UI.
pub fn in_rect<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(UiBuilder::new().max_rect(rect), add).inner
}
