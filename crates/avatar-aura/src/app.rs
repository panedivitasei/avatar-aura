// Window shell from web/index.html: header, Import and Export tabs, footer action row, message dispatch and settings.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use crate::catalog::{self, Catalog};
use crate::export_tab::ExportTab;
use crate::import_tab::ImportTab;
use crate::jobs::{rgba_to_color, Jobs, Msg};
use crate::paths;
use crate::settings::Settings;
use crate::viewport::Viewport;
use crate::widgets::ACCENT;

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
    thumbs: HashMap<String, egui::TextureHandle>,
    import: ImportTab,
    export: ExportTab,
    errors: Vec<String>,
    /// `--load-avatar`: open the Export tab and load the saved avatar on the first frame.
    load_avatar: bool,
}

fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |style| {
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.selection.stroke.color = egui::Color32::WHITE;
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.panel_fill = egui::Color32::from_rgb(0xE8, 0xEC, 0xE4);
        style.visuals.window_fill = egui::Color32::from_rgb(0xF7, 0xF8, 0xF5);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
    });
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
        let catalog = Catalog::load().unwrap_or_else(|e| {
            errors.push(format!("bundled catalog: {e}"));
            Catalog::default()
        });
        let settings = Settings::load();
        let settings_tab_is_export = settings.tab == "export";
        jobs.spawn("thumbnails", |reply| {
            let thumbs = catalog::decode_thumbs(&paths::asset(&["thumbs"]))
                .into_iter()
                .map(|(name, img)| (name, rgba_to_color(&img)))
                .collect();
            reply.send(Msg::Thumbs(thumbs));
        });
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
            thumbs: HashMap::new(),
            import,
            export,
            errors,
            load_avatar,
        })
    }

    fn dispatch(&mut self, ctx: &egui::Context) {
        for msg in self.jobs.drain() {
            match msg {
                Msg::Thumbs(images) => {
                    for (name, img) in images {
                        let tex =
                            ctx.load_texture(format!("thumb-{name}"), img, egui::TextureOptions::LINEAR);
                        self.thumbs.insert(name, tex);
                    }
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
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let screen = ctx.content_rect();
            let painter =
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(90));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop avatar items to validate them",
                egui::FontId::proportional(22.0),
                egui::Color32::WHITE,
            );
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

        egui::Panel::top("header")
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0xF7, 0xF8, 0xF5))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("avatar aura").size(22.0).strong().color(ACCENT));
                    ui.add_space(24.0);
                    ui.selectable_value(&mut self.tab, Tab::Import, RichText::new("Import").size(15.0));
                    ui.selectable_value(&mut self.tab, Tab::Export, RichText::new("Export").size(15.0));
                    if !self.errors.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let last = self.errors.last().cloned().unwrap_or_default();
                            if ui.small_button("Dismiss").clicked() {
                                self.errors.clear();
                            }
                            ui.label(RichText::new(last).color(egui::Color32::from_rgb(0xB0, 0x30, 0x20)));
                        });
                    }
                });
            });
        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(0xF7, 0xF8, 0xF5))
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ui, |ui| match self.tab {
                Tab::Import => self.import.footer(ui, &self.jobs, &self.settings),
                Tab::Export => self.export.footer(ui, &self.jobs, &self.settings, &self.catalog),
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(16)))
            .show(ui, |ui| match self.tab {
                Tab::Import => self.import.central(ui, &self.jobs, &mut self.settings),
                Tab::Export => {
                    self.export
                        .central(ui, &self.jobs, &mut self.settings, &self.catalog, &self.thumbs)
                }
            });
        self.import.windows(&ctx, &self.jobs, &mut self.settings);
        self.export.windows(&ctx, &mut self.settings);
    }

    fn on_exit(&mut self) {
        self.save();
    }
}
