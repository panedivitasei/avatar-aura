// Window shell from web/index.html: header, Import and Export tabs, footer action row, message dispatch and settings.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, pos2, Color32, Rect, RichText, Sense, Stroke};

use crate::catalog::Catalog;
use crate::export_tab::ExportTab;
use crate::import_tab::ImportTab;
use crate::jobs::{Jobs, Msg};
use crate::settings::Settings;
use crate::viewport::Viewport;
use crate::widgets::{self, c, Kind, Paint, W};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Import,
    Export,
}

pub struct AuraApp {
    settings: Settings,
    saved: Settings,
    last_save: Instant,
    tab: Tab,
    jobs: Jobs,
    catalog: Catalog,
    thumbs: Thumbs,
    import: ImportTab,
    export: ExportTab,
    errors: Vec<String>,
    /// `--load-avatar`: open the Export tab and load the saved avatar on the first frame.
    load_avatar: bool,
    /// Raw window handle for the dark caption, re-applied over the first frames once the window is shown.
    caption: Option<raw_window_handle::RawWindowHandle>,
    caption_frames: u8,
}

fn style(ctx: &egui::Context) {
    widgets::install_fonts(ctx);
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |style| {
        let v = &mut style.visuals;
        v.selection.bg_fill = c::GREEN.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, c::TEXT);
        v.hyperlink_color = c::GREEN;
        v.panel_fill = c::ROOT_BG;
        v.window_fill = c::INPUT_BG;
        v.extreme_bg_color = c::INPUT_BG;
        v.override_text_color = Some(c::TEXT);
        v.text_cursor.stroke = Stroke::new(1.5, c::INPUT_TEXT);
        style.spacing.item_spacing = egui::Vec2::ZERO;
        style.spacing.button_padding = egui::vec2(10.0, 8.0);
        style.spacing.scroll = egui::style::ScrollStyle::thin();
        style
            .text_styles
            .insert(egui::TextStyle::Body, widgets::font(14.0, W::Regular));
        style
            .text_styles
            .insert(egui::TextStyle::Button, widgets::font(13.0, W::Regular));
    });
}

/// `.masthead`: gradient band, the gradient-filled Yesterday title and the short rule under it.
fn masthead(painter: &egui::Painter, rect: Rect) {
    widgets::fill(
        painter,
        rect,
        0.0,
        &Paint::Linear(
            110.0,
            &[
                (0.0, widgets::rgba(0xFFFF_FF90)),
                (0.5, widgets::rgba(0xD4F0_B644)),
                (1.0, widgets::rgba(0xBEDC_EC44)),
            ],
        ),
    );
    let mut job = egui::text::LayoutJob::single_section(
        "avatar aura".into(),
        egui::TextFormat {
            font_id: widgets::yesterday(42.0),
            extra_letter_spacing: 42.0 * 0.015,
            color: Color32::WHITE,
            ..Default::default()
        },
    );
    job.wrap.max_width = f32::INFINITY;
    let galley = painter.layout_job(job);
    // h1 box: 2px top padding, 46.2px line box, 7px bottom padding.
    let top = rect.min.y + 22.0;
    let line_box = Rect::from_min_size(
        pos2(rect.min.x + 32.0, top + 2.0),
        egui::vec2(galley.size().x, 46.2),
    );
    let pos = pos2(line_box.min.x, line_box.center().y - galley.size().y / 2.0);
    let box_rect = Rect::from_min_max(
        pos2(line_box.min.x - 2.0, top),
        pos2(line_box.max.x + 2.0, top + 55.2),
    );
    painter.galley_with_override_text_color(
        pos + egui::vec2(0.0, 2.5),
        galley.clone(),
        widgets::rgba(0x526B_3526),
    );
    painter.galley_with_override_text_color(pos + egui::vec2(0.0, 1.0), galley.clone(), Color32::WHITE);
    let stops = [
        (0.03, widgets::rgb(0xBADB7D)),
        (0.39, widgets::rgb(0x71983C)),
        (0.48, widgets::rgb(0x3F621E)),
        (0.52, widgets::rgb(0x7AA541)),
        (0.91, widgets::rgb(0x456B24)),
    ];
    let bands = box_rect.height().ceil() as usize;
    for b in 0..bands {
        let y0 = box_rect.min.y + b as f32;
        let t = (b as f32 + 0.5) / box_rect.height();
        let color = stop_color(&stops, t);
        let band = Rect::from_min_max(
            pos2(box_rect.min.x - 4.0, y0),
            pos2(box_rect.max.x + 8.0, y0 + 1.0),
        );
        painter
            .with_clip_rect(band.intersect(painter.clip_rect()))
            .galley_with_override_text_color(pos, galley.clone(), color);
    }
    let rule = Rect::from_min_size(
        pos2(rect.min.x + 30.0, rect.max.y - 1.0),
        egui::vec2(330.0f32.min(rect.width() * 0.55), 1.0),
    );
    widgets::fill(
        painter,
        rule,
        0.0,
        &Paint::Linear(
            90.0,
            &[
                (0.0, widgets::rgba(0x759C_42A0)),
                (1.0, widgets::rgba(0xE1EF_C800)),
            ],
        ),
    );
}

fn stop_color(stops: &[(f32, Color32)], t: f32) -> Color32 {
    let mut prev = stops[0];
    if t <= prev.0 {
        return prev.1;
    }
    for &(at, color) in &stops[1..] {
        if t <= at {
            let k = (t - prev.0) / (at - prev.0);
            return prev.1.lerp_to_gamma(color, k);
        }
        prev = (at, color);
    }
    prev.1
}

/// Strip tiles rendered for one export session; empty until Load All has run for the avatar on screen.
#[derive(Default)]
struct Thumbs {
    shown: HashMap<String, egui::TextureHandle>,
    session: Option<u64>,
}

impl Thumbs {
    fn add(&mut self, session: u64, tiles: Vec<(String, egui::TextureHandle)>) {
        self.sync(session);
        self.session = Some(session);
        self.shown.extend(tiles);
    }

    /// Clears the tiles once the export tab shows a different avatar.
    fn sync(&mut self, session: u64) {
        if self.session.is_some_and(|s| s != session) {
            self.session = None;
            self.shown.clear();
        }
    }
}

impl AuraApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        renderer_note: String,
        load_avatar: bool,
    ) -> Result<Self, String> {
        let state = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("the wgpu renderer is not available")?;
        style(&cc.egui_ctx);
        let jobs = Jobs::new(&cc.egui_ctx);
        let mut errors = Vec::new();
        let caption = crate::caption::handle(cc);
        if let Some(e) = caption.and_then(|h| crate::caption::darken(h).err()) {
            errors.push(format!("caption: {e}"));
        }
        let catalog = Catalog::load().unwrap_or_else(|e| {
            errors.push(format!("bundled catalog: {e}"));
            Catalog::default()
        });
        let settings = Settings::load();
        let settings_tab_is_export = settings.tab == "export";
        let info = state.adapter.get_info();
        let mut export = ExportTab::new(Viewport::new(state));
        export.note_line(format!(
            "renderer: {} ({:?}); {renderer_note}",
            info.name, info.backend
        ));
        let import = ImportTab::new(Viewport::new(state));
        import.refresh_titles(&jobs, &settings);
        Ok(Self {
            saved: settings.clone(),
            settings,
            last_save: Instant::now(),
            tab: if settings_tab_is_export {
                Tab::Export
            } else {
                Tab::Import
            },
            jobs,
            catalog,
            thumbs: Thumbs::default(),
            import,
            export,
            errors,
            load_avatar,
            caption,
            caption_frames: 3,
        })
    }

    fn dispatch(&mut self, ctx: &egui::Context) {
        for msg in self.jobs.drain() {
            match msg {
                Msg::Tiles { session, images } => {
                    if session != self.export.session() {
                        continue;
                    }
                    let tiles = images
                        .into_iter()
                        .map(|(name, img)| {
                            let tex =
                                ctx.load_texture(format!("tile-{name}"), img, egui::TextureOptions::LINEAR);
                            (name, tex)
                        })
                        .collect();
                    self.thumbs.add(session, tiles);
                }
                Msg::Failed(e) => {
                    self.export.status.clone_from(&e);
                    self.errors.push(e);
                }
                msg @ (Msg::ImportAnalyzed { .. }
                | Msg::ImportWritten(_)
                | Msg::TitleIcon(_)
                | Msg::Titles(_)
                | Msg::ImportPreview { .. }) => self.import.handle(msg, &self.jobs, &self.settings, ctx),
                msg => self.export.handle(msg, &self.jobs),
            }
        }
        self.thumbs.sync(self.export.session());
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.tab = Tab::Import;
            self.import.take_paths(dropped, &self.jobs, &self.settings);
        }
    }

    fn persist(&mut self, ctx: &egui::Context) {
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            self.settings.window = Some([rect.width(), rect.height()]);
        }
        self.settings.tab = match self.tab {
            Tab::Import => "import".into(),
            Tab::Export => "export".into(),
        };
        if self.settings != self.saved {
            if self.last_save.elapsed() > Duration::from_secs(5) {
                self.save();
            } else {
                ctx.request_repaint_after(Duration::from_secs(5));
            }
        }
    }

    /// The `nav` tab row: plain text tabs with a 4px underline, then the 1px line under the row.
    fn nav(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let mut x = rect.min.x + 30.0;
        for (tab, label) in [(Tab::Import, "Import"), (Tab::Export, "Export")] {
            let on = self.tab == tab;
            let (weight, color) = if on {
                (W::Heavy, c::TAB_ON)
            } else {
                (W::Regular, c::TAB)
            };
            let galley = ui
                .painter()
                .layout_no_wrap(label.into(), widgets::font(18.0, weight), color);
            let tab_rect = Rect::from_min_size(pos2(x, rect.min.y), egui::vec2(galley.size().x + 8.0, 48.0));
            let response = ui.interact(tab_rect, egui::Id::new(("tab", label)), Sense::click());
            if response.clicked() {
                self.tab = tab;
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            ui.painter().galley(
                pos2(
                    x + 4.0,
                    tab_rect.min.y + 10.0 + (widgets::line(18.0) - galley.size().y) / 2.0,
                ),
                galley,
                color,
            );
            if on {
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(tab_rect.min.x, tab_rect.max.y - 4.0), tab_rect.max),
                    0.0,
                    c::GREEN,
                );
            }
            x = tab_rect.max.x + 28.0;
        }
        ui.painter()
            .hline(rect.x_range(), rect.max.y - 0.5, Stroke::new(1.0, c::LINE));
        if let Some(last) = self.errors.last().cloned() {
            let row = Rect::from_min_max(
                pos2(x + 20.0, rect.min.y + 8.0),
                pos2(rect.max.x - 30.0, rect.max.y - 9.0),
            );
            widgets::in_rect(ui, row, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, "Dismiss", Kind::Short, true).clicked() {
                        self.errors.clear();
                    }
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(last)
                                .font(widgets::font(12.0, W::Regular))
                                .color(c::ERROR),
                        )
                        .truncate(),
                    );
                });
            });
        }
    }

    fn save(&mut self) {
        match self.settings.save() {
            Ok(()) => self.saved = self.settings.clone(),
            Err(e) => self.errors.push(format!("saving settings: {e}")),
        }
        self.last_save = Instant::now();
    }
}

impl eframe::App for AuraApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.dispatch(&ctx);
        if self.caption_frames > 0 {
            self.caption_frames -= 1;
            if let Some(handle) = self.caption {
                let _ = crate::caption::darken(handle);
            }
            ctx.request_repaint();
        }
        if std::mem::take(&mut self.load_avatar) {
            self.tab = Tab::Export;
            self.export.load(&self.jobs, &self.settings, &self.catalog);
        }
        self.dropped_files(&ctx);
        let dt = f64::from(ctx.input(|i| i.unstable_dt)).min(0.1);
        if self.export.tick(dt) {
            ctx.request_repaint();
        }
        self.persist(&ctx);

        let full = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, c::ROOT_BG);
        widgets::fill(
            &painter,
            full,
            0.0,
            &Paint::Radial(
                egui::vec2(0.15, 0.0),
                &[(0.0, widgets::rgb(0xFBFFF8)), (0.65, Color32::TRANSPARENT)],
            ),
        );
        let mast = Rect::from_min_size(full.min, egui::vec2(full.width(), 83.0));
        masthead(&painter, mast);
        let nav = Rect::from_min_size(pos2(full.min.x, mast.max.y), egui::vec2(full.width(), 49.0));
        self.nav(ui, nav);

        let main_w = full.width().min(1800.0);
        let main = Rect::from_min_max(
            pos2(full.center().x - main_w / 2.0 + 30.0, nav.max.y + 14.0),
            pos2(full.center().x + main_w / 2.0 - 30.0, full.max.y - 18.0),
        );
        let heading_h = widgets::line(24.0) + 4.0 + widgets::line(12.0);
        let heading = Rect::from_min_size(main.min, egui::vec2(main.width(), heading_h));
        let footer = Rect::from_min_max(pos2(main.min.x, main.max.y - 42.0), main.max);
        let body = Rect::from_min_max(
            pos2(main.min.x, heading.max.y + 12.0),
            pos2(main.max.x, footer.min.y - 12.0),
        );
        let (title, blurb) = match self.tab {
            Tab::Import => (
                "Import avatar items into your closet",
                "Validate STFS containers or raw .bin files, then add their items and awards to your closet.",
            ),
            Tab::Export => (
                "Export your avatar as a 3D model",
                "Choose an animation frame and expression, then export to DAE, GLB, OBJ or SMD.",
            ),
        };
        widgets::in_rect(ui, heading, |ui| widgets::section_heading(ui, title, blurb));
        widgets::in_rect(ui, body, |ui| match self.tab {
            Tab::Import => self.import.central(ui, &self.jobs, &mut self.settings),
            Tab::Export => self.export.central(
                ui,
                &self.jobs,
                &mut self.settings,
                &self.catalog,
                &self.thumbs.shown,
            ),
        });
        widgets::in_rect(ui, footer, |ui| match self.tab {
            Tab::Import => self.import.footer(ui, &self.jobs, &self.settings),
            Tab::Export => self.export.footer(ui, &self.jobs, &self.settings, &self.catalog),
        });
        self.import.windows(&ctx, &self.jobs, &mut self.settings);
        self.export.windows(&ctx, &mut self.settings);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        c::ROOT_BG.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self) {
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
        ctx.load_texture(name, egui::ColorImage::example(), egui::TextureOptions::LINEAR)
    }

    #[test]
    fn rendered_tiles_last_for_their_session_only() {
        let ctx = egui::Context::default();
        let mut thumbs = Thumbs::default();
        let rendered = texture(&ctx, "rendered");
        thumbs.add(
            3,
            vec![
                ("mouth1.png".into(), rendered.clone()),
                ("clip_Custom.png".into(), rendered.clone()),
            ],
        );
        thumbs.sync(3);
        assert_eq!(thumbs.shown["mouth1.png"].id(), rendered.id());
        thumbs.sync(4);
        assert!(thumbs.shown.is_empty());
    }
}
