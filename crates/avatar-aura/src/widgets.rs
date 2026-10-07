// Small shared pieces of UI: paged thumbnail strips, path rows with a browse button, log windows, headings.

use std::path::PathBuf;

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

pub const ACCENT: Color32 = Color32::from_rgb(0x5E, 0x8F, 0x22);
const TILE: Vec2 = Vec2::new(78.0, 92.0);

/// One choice of a strip.
pub struct Tile {
    pub key: String,
    pub label: String,
    pub thumb: Option<egui::TextureId>,
    pub badge: Option<String>,
}

/// A paged row of thumbnail tiles with previous and next arrows; returns the clicked index.
pub fn strip(
    ui: &mut egui::Ui,
    id_salt: &str,
    tiles: &[Tile],
    selected: Option<&str>,
    enabled: bool,
) -> Option<usize> {
    let id = ui.id().with(id_salt);
    let arrow_w = 22.0;
    let spacing = ui.spacing().item_spacing.x;
    let per_page =
        (((ui.available_width() - 2.0 * (arrow_w + spacing)) / (TILE.x + spacing)).floor() as usize).max(1);
    let selected_index = selected.and_then(|s| tiles.iter().position(|t| t.key == s));
    let (mut page, last) = ui
        .data(|d| d.get_temp::<(usize, Option<usize>)>(id))
        .unwrap_or((selected_index.unwrap_or(0) / per_page, selected_index));
    if last != selected_index {
        if let Some(i) = selected_index {
            page = i / per_page;
        }
    }
    let pages = tiles.len().div_ceil(per_page).max(1);
    page = page.min(pages - 1);
    let mut clicked = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                page > 0,
                egui::Button::new("‹").min_size(Vec2::new(arrow_w, TILE.y)),
            )
            .on_hover_text("Previous choices")
            .clicked()
        {
            page -= 1;
        }
        let start = page * per_page;
        for (i, tile) in tiles.iter().enumerate().skip(start).take(per_page) {
            if tile_button(ui, tile, selected_index == Some(i), enabled).clicked() {
                clicked = Some(i);
            }
        }
        for _ in tiles.len().saturating_sub(start).min(per_page)..per_page {
            ui.add_space(TILE.x + spacing);
        }
        if ui
            .add_enabled(
                page + 1 < pages,
                egui::Button::new("›").min_size(Vec2::new(arrow_w, TILE.y)),
            )
            .on_hover_text("Next choices")
            .clicked()
        {
            page += 1;
        }
    });
    ui.data_mut(|d| d.insert_temp(id, (page, selected_index)));
    clicked
}

fn tile_button(ui: &mut egui::Ui, tile: &Tile, selected: bool, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(TILE, sense);
    let response = response.on_hover_text(&tile.label);
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let fill = if selected {
        visuals.selection.bg_fill.gamma_multiply(0.35)
    } else if response.hovered() && enabled {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.extreme_bg_color
    };
    let stroke = if selected {
        Stroke::new(2.0, ACCENT)
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    painter.rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
    let image_rect = Rect::from_min_size(rect.min + Vec2::new(7.0, 5.0), Vec2::splat(64.0));
    match tile.thumb {
        Some(tex) => {
            let tint = if enabled {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(140)
            };
            painter.image(
                tex,
                image_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
        }
        None => {
            painter.rect_filled(image_rect, 4.0, visuals.faint_bg_color);
        }
    }
    let text_color = if enabled {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    let galley = painter.layout(
        tile.label.clone(),
        egui::FontId::proportional(10.5),
        text_color,
        TILE.x - 6.0,
    );
    let row = galley.rows.first().map_or(galley.size().y, |r| r.height());
    let label_pos = egui::pos2(rect.min.x + 3.0, image_rect.max.y + 3.0);
    painter
        .with_clip_rect(Rect::from_min_size(
            label_pos,
            Vec2::new(TILE.x - 6.0, row * 1.0 + 1.0),
        ))
        .galley(label_pos, galley, text_color);
    if let Some(badge) = &tile.badge {
        painter.text(
            image_rect.right_top() + Vec2::new(-2.0, 2.0),
            egui::Align2::RIGHT_TOP,
            badge,
            egui::FontId::proportional(9.5),
            ACCENT,
        );
    }
    response
}

/// What a browse button opens.
#[derive(Clone, Copy)]
pub enum Pick {
    File,
    Folder,
}

/// A labelled path field with a Browse button; true when the value changed.
pub fn path_row(ui: &mut egui::Ui, label: &str, value: &mut String, pick: Pick, hint: &str) -> bool {
    let mut changed = false;
    ui.label(label);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 70.0).max(80.0);
        changed |= ui
            .add(
                egui::TextEdit::singleline(value)
                    .hint_text(hint)
                    .desired_width(width),
            )
            .changed();
        if ui.button("Browse").clicked() {
            if let Some(path) = browse(pick, value) {
                *value = path.display().to_string();
                changed = true;
            }
        }
    });
    changed
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

/// The eyebrow, title and blurb that open each tab.
pub fn section_heading(ui: &mut egui::Ui, eyebrow: &str, title: &str, blurb: &str) {
    ui.label(RichText::new(eyebrow).small().strong().color(ACCENT));
    ui.label(RichText::new(title).size(20.0).strong());
    ui.label(RichText::new(blurb).weak());
}

/// A scrolling monospace log in its own window.
pub fn log_window(ctx: &egui::Context, title: &str, open: &mut bool, lines: &[String]) {
    egui::Window::new(title)
        .open(open)
        .default_size(Vec2::new(640.0, 420.0))
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(lines.join("\n"));
                }
                ui.label(RichText::new(format!("{} lines", lines.len())).weak());
            });
            ui.separator();
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if lines.is_empty() {
                        ui.label(RichText::new("Nothing logged yet.").weak());
                    }
                    for line in lines {
                        ui.label(RichText::new(line).monospace().size(12.0));
                    }
                });
        });
}

/// A card with a title row, as the HTML `.window.card` blocks.
pub fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().panel_fill)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().size(15.0));
            ui.separator();
            add(ui)
        })
        .inner
}
