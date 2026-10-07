// The Import tab of web/js/import.js: choose or drop items, validate, preview each on a mannequin, install into a closet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use avatar_export::scene::Scene;
use eframe::egui::{self, Align, Layout, Rect, RichText};

use crate::import::{self, validate::ItemResult, ItemSummary, Outcome};
use crate::jobs::{color_image, ImportPreview, Jobs, Msg};
use crate::paths;
use crate::settings::Settings;
use crate::viewport::Viewport;
use crate::widgets::{self, Pick, Tile};

pub struct ImportTab {
    pub viewport: Viewport,
    paths: Vec<PathBuf>,
    input: String,
    busy: bool,
    session: u64,
    validated: bool,
    results: Arc<Vec<ItemResult>>,
    items: Vec<ItemSummary>,
    icons: Vec<Option<egui::TextureHandle>>,
    selected: Option<usize>,
    summary: String,
    pub status: String,
    log: Vec<String>,
    log_open: bool,
    award_open: bool,
    titles: Vec<(String, String)>,
    award_title: String,
    preview_status: String,
    preview_generation: u64,
    previewing: bool,
    pending_preview: Option<usize>,
    preview_scenes: HashMap<usize, (Arc<Scene>, PathBuf)>,
}

impl ImportTab {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            paths: Vec::new(),
            input: String::new(),
            busy: false,
            session: 0,
            validated: false,
            results: Arc::new(Vec::new()),
            items: Vec::new(),
            icons: Vec::new(),
            selected: None,
            summary: "Choose items to see validation results.".into(),
            status: String::new(),
            log: Vec::new(),
            log_open: false,
            award_open: false,
            titles: Vec::new(),
            award_title: String::new(),
            preview_status: String::new(),
            preview_generation: 0,
            previewing: false,
            pending_preview: None,
            preview_scenes: HashMap::new(),
        }
    }

    fn show_outcome(&mut self, outcome: &Outcome, written: bool) {
        self.log = if outcome.log.is_empty() && !outcome.error.is_empty() {
            vec![outcome.error.clone()]
        } else {
            outcome.log.clone()
        };
        self.summary = if outcome.ok {
            format!(
                "{} items validated. Open the log for details.",
                outcome.items.len()
            )
        } else {
            outcome.error.clone()
        };
        self.status = if written && outcome.ok {
            format!(
                "Imported {} items ({} files written).",
                outcome.items.len(),
                outcome.written.len()
            )
        } else if outcome.ok {
            "Select an item to preview it.".into()
        } else {
            "Validation refused the input.".into()
        };
    }

    /// Files dropped on the window or chosen in the picker.
    pub fn take_paths(&mut self, paths: Vec<PathBuf>, jobs: &Jobs, settings: &Settings) {
        if self.busy || paths.is_empty() {
            return;
        }
        self.input = if paths.len() == 1 {
            paths[0].display().to_string()
        } else {
            format!("{} selected files", paths.len())
        };
        self.paths = paths;
        self.analyze(jobs, settings);
    }

    fn analyze(&mut self, jobs: &Jobs, _settings: &Settings) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Validating...".into();
        self.session += 1;
        let session = self.session;
        let paths = self.paths.clone();
        jobs.spawn("validate", move |reply| {
            let outcome = import::perform(&paths, &PathBuf::new(), &PathBuf::new(), false);
            let icons = outcome
                .items
                .iter()
                .map(|i| i.icon.as_deref().and_then(color_image))
                .collect();
            reply.send(Msg::ImportAnalyzed {
                session,
                outcome,
                icons,
            });
        });
    }

    fn import(&mut self, jobs: &Jobs, settings: &Settings) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Importing...".into();
        let paths = self.paths.clone();
        let closet = paths::expand(&settings.import_closet);
        let icon = paths::expand(&settings.icon_path);
        jobs.spawn("import", move |reply| {
            reply.send(Msg::ImportWritten(import::perform(&paths, &closet, &icon, true)));
        });
    }

    pub fn refresh_titles(&self, jobs: &Jobs, settings: &Settings) {
        let closet = paths::expand(&settings.import_closet);
        jobs.spawn("award titles", move |reply| {
            reply.send(Msg::Titles(import::closet::list_award_titles(&closet)));
        });
    }

    fn install_icon(&self, jobs: &Jobs, settings: &Settings) {
        let closet = paths::expand(&settings.import_closet);
        let icon = paths::expand(&settings.icon_path);
        let title = self.award_title.clone();
        jobs.spawn("title icon", move |reply| {
            reply.send(Msg::TitleIcon(import::title_icon(&closet, &title, &icon)));
        });
    }

    fn preview(&mut self, index: usize, jobs: &Jobs, settings: &Settings) {
        self.selected = Some(index);
        self.preview_generation += 1;
        if self.previewing {
            self.pending_preview = Some(index);
            return;
        }
        let Some(item) = self.results.get(index).cloned() else {
            return;
        };
        self.previewing = true;
        self.preview_status = "Loading selected item...".into();
        let (session, generation) = (self.session, self.preview_generation);
        let gpu = self.viewport.gpu();
        let cached = self.preview_scenes.get(&index).cloned();
        let pack = paths::expand(&settings.pack);
        let dir = paths::cache_dir()
            .join("imports")
            .join(format!("{}-{session}", std::process::id()))
            .join(index.to_string());
        jobs.spawn("item preview", move |reply| {
            let mut log = Vec::new();
            let result = (|| -> anyhow::Result<ImportPreview> {
                let (scene, dir) = match cached {
                    Some(hit) => hit,
                    None => {
                        let _ = std::fs::remove_dir_all(&dir);
                        log.push(format!("baking {} on the mannequin", item.guid));
                        let json = import::bake_mannequin_preview(&item, &pack, &dir)?;
                        (Arc::new(crate::bake::read_scene(&json)?), dir)
                    }
                };
                let viewer = gpu.build_viewer(&scene, &dir)?;
                Ok(ImportPreview {
                    index,
                    viewer,
                    scene,
                    dir,
                })
            })()
            .map_err(|e| {
                log.push(format!("{e:#}"));
                format!("Could not render the selected item: {e}")
            });
            reply.send(Msg::ImportPreview {
                session,
                generation,
                log,
                result,
            });
        });
    }

    pub fn handle(&mut self, msg: Msg, jobs: &Jobs, settings: &Settings, ctx: &egui::Context) {
        match msg {
            Msg::ImportAnalyzed {
                session,
                outcome,
                icons,
            } => {
                self.busy = false;
                if session != self.session {
                    return;
                }
                self.show_outcome(&outcome, false);
                self.validated = outcome.ok;
                self.items = outcome.items.clone();
                self.results = Arc::new(outcome.results);
                self.preview_scenes.clear();
                self.icons = icons
                    .into_iter()
                    .enumerate()
                    .map(|(i, img)| {
                        img.map(|img| {
                            ctx.load_texture(format!("item-{session}-{i}"), img, Default::default())
                        })
                    })
                    .collect();
                self.pending_preview = None;
                self.selected = None;
                if outcome.ok && !self.items.is_empty() {
                    self.preview(0, jobs, settings);
                } else {
                    self.viewport.viewer = None;
                    self.preview_status.clear();
                }
            }
            Msg::ImportWritten(outcome) => {
                self.busy = false;
                self.show_outcome(&outcome, true);
                self.refresh_titles(jobs, settings);
            }
            Msg::TitleIcon(outcome) => {
                self.show_outcome(&outcome, false);
                self.status = if outcome.ok {
                    "Game icon installed.".into()
                } else {
                    outcome.error
                };
            }
            Msg::Titles(titles) => self.titles = titles,
            Msg::ImportPreview {
                session,
                generation,
                log,
                result,
            } => {
                self.previewing = false;
                if session == self.session {
                    self.log.extend(log);
                    match result {
                        Ok(preview) => {
                            self.preview_scenes
                                .insert(preview.index, (preview.scene.clone(), preview.dir.clone()));
                            if generation == self.preview_generation {
                                self.show_preview(preview);
                            }
                        }
                        Err(e) if generation == self.preview_generation => self.preview_status = e,
                        Err(_) => {}
                    }
                }
                if let Some(next) = self.pending_preview.take() {
                    self.preview(next, jobs, settings);
                }
            }
            _ => {}
        }
    }

    fn show_preview(&mut self, preview: ImportPreview) {
        let mut viewer = preview.viewer;
        let clips = viewer
            .clip_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let clip = clips
            .iter()
            .position(|n| n.to_lowercase().contains("idle"))
            .or(if clips.is_empty() { None } else { Some(0) });
        if let Some(c) = clip {
            let frames = preview
                .scene
                .animations
                .iter()
                .find(|a| a.name == clips[c])
                .map_or(1, |a| a.frame_count.max(1));
            let _ = viewer.set_clip_frame(c, (frames / 2) as f32);
        }
        self.viewport.replace(viewer, false);
        let name = self.items.get(preview.index).map_or("", |i| i.name.as_str());
        self.preview_status = format!("{name} - drag to orbit");
    }

    pub fn windows(&mut self, ctx: &egui::Context, jobs: &Jobs, settings: &mut Settings) {
        widgets::log_window(ctx, "Validation log", &mut self.log_open, &self.log);
        let mut open = self.award_open;
        let mut install = false;
        egui::Window::new("Avatar award icons")
            .open(&mut open)
            .default_width(460.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Award imports require a game icon. You can also replace an installed game's icon here.")
                        .weak(),
                );
                widgets::path_row(ui, "Game icon", &mut settings.icon_path, Pick::File, "PNG, JPEG, BMP or GIF");
                ui.label("Installed game");
                ui.horizontal(|ui| {
                    let current = self
                        .titles
                        .iter()
                        .find(|(id, _)| *id == self.award_title)
                        .map(|(id, name)| format!("{} ({id})", if name.is_empty() { id } else { name }))
                        .unwrap_or_else(|| "Choose an installed game".into());
                    egui::ComboBox::from_id_salt("award-title")
                        .selected_text(current)
                        .width((ui.available_width() - 80.0).max(120.0))
                        .show_ui(ui, |ui| {
                            for (id, name) in &self.titles {
                                let label = format!("{} ({id})", if name.is_empty() { id } else { name });
                                ui.selectable_value(&mut self.award_title, id.clone(), label);
                            }
                        });
                    install = ui.button("Set icon").clicked();
                });
            });
        self.award_open = open;
        if install {
            self.install_icon(jobs, settings);
        }
    }

    pub fn central(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &mut Settings) {
        widgets::section_heading(
            ui,
            "YOUR COLLECTION",
            "Import avatar items into your closet",
            "Validate STFS containers or raw .bin files, then add their items and awards to your closet.",
        );
        ui.add_space(6.0);
        egui::Panel::right("import-side")
            .resizable(false)
            .default_size(340.0)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: 10,
                ..Default::default()
            }))
            .show(ui, |ui| self.side(ui, jobs, settings));
        egui::CentralPanel::no_frame().show(ui, |ui| self.stage(ui, jobs, settings));
    }

    fn stage(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &Settings) {
        let full = ui.available_rect_before_wrap();
        let strip_h = 112.0;
        let view = Rect::from_min_max(
            full.min,
            egui::pos2(full.max.x, (full.max.y - strip_h - 8.0).max(full.min.y + 80.0)),
        );
        self.viewport.show(ui, view, &[]);
        if self.viewport.viewer.is_none() {
            ui.put(view, |ui: &mut egui::Ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(view.height() * 0.4);
                    ui.label(RichText::new("Preview items on a mannequin").size(17.0).strong());
                    ui.label(RichText::new("Choose files, then select an item below.").weak());
                })
                .response
            });
        }
        if !self.preview_status.is_empty() {
            ui.painter().text(
                view.left_bottom() + egui::vec2(10.0, -10.0),
                egui::Align2::LEFT_BOTTOM,
                &self.preview_status,
                egui::FontId::proportional(12.5),
                ui.visuals().weak_text_color(),
            );
        }
        ui.advance_cursor_after_rect(view);
        ui.add_space(8.0);
        let tiles: Vec<Tile> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| Tile {
                key: i.to_string(),
                label: format!(
                    "{} ({})",
                    item.name,
                    &item.guid[item.guid.len().saturating_sub(8)..]
                ),
                thumb: self.icons.get(i).and_then(|t| t.as_ref()).map(|t| t.id()),
                badge: Some(item.bodies.to_string()),
            })
            .collect();
        if tiles.is_empty() {
            ui.label(RichText::new("Validated items appear here.").weak());
        } else if let Some(i) = widgets::strip(
            ui,
            "import-items",
            &tiles,
            self.selected.map(|s| s.to_string()).as_deref(),
            true,
        ) {
            self.preview(i, jobs, settings);
        }
    }

    fn side(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &mut Settings) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            widgets::card(ui, "Import items", |ui| {
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Choose items to import").strong().size(15.0));
                            ui.label(
                                RichText::new(
                                    "STFS containers & raw .bin avatar items, or drop them on the window",
                                )
                                .weak(),
                            );
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("Choose items"))
                                .clicked()
                            {
                                if let Some(files) = rfd::FileDialog::new().pick_files() {
                                    self.take_paths(files, jobs, settings);
                                }
                            }
                        });
                    });
                ui.add_space(6.0);
                ui.label("Input file");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("Drop or choose an avatar item")
                        .desired_width(f32::INFINITY),
                );
                if edit.lost_focus() && edit.changed()
                    || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    let path = paths::expand(&self.input);
                    if !path.as_os_str().is_empty() {
                        self.paths = vec![path];
                        self.analyze(jobs, settings);
                    }
                }
                if widgets::path_row(
                    ui,
                    "Closet destination",
                    &mut settings.import_closet,
                    Pick::Folder,
                    "Choose a closet folder",
                ) {
                    self.refresh_titles(jobs, settings);
                }
                ui.add_space(6.0);
                ui.label(RichText::new(&self.summary).weak());
                if let Some(item) = self.selected.and_then(|i| self.items.get(i)) {
                    ui.label(RichText::new(&item.name).strong());
                    ui.label(format!("{} · {}", item.categories, item.bodies));
                    ui.label(RichText::new(&item.guid).monospace().size(11.0));
                    if item.award {
                        ui.label(
                            RichText::new("Avatar award: needs a game icon to import").color(widgets::ACCENT),
                        );
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Avatar award icons").clicked() {
                        self.award_open = true;
                        self.refresh_titles(jobs, settings);
                    }
                    if ui.button("Validation log").clicked() {
                        self.log_open = true;
                    }
                });
            });
        });
    }

    pub fn footer(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &Settings) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.status).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let import = egui::Button::new(RichText::new("Import").strong())
                    .fill(widgets::ACCENT.gamma_multiply(0.25));
                if ui.add_enabled(!self.busy && self.validated, import).clicked() {
                    self.import(jobs, settings);
                }
                if ui
                    .add_enabled(
                        !self.busy && !self.paths.is_empty(),
                        egui::Button::new("Validate"),
                    )
                    .clicked()
                {
                    self.analyze(jobs, settings);
                }
                if self.busy || self.previewing {
                    ui.spinner();
                }
            });
        });
    }
}
