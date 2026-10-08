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
use crate::widgets::{self, c, Kind, Pick, Tile, W};

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
    log_dialog: widgets::LogDialog,
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
            log_dialog: widgets::LogDialog::default(),
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
        self.log_dialog.show(ctx, "Validation log", &self.log);
        if !self.award_open {
            return;
        }
        let mut install = false;
        self.award_open = widgets::dialog(ctx, "award-dialog", "Avatar award icons", |ui| {
            widgets::muted(
                ui,
                "Award imports require a game icon. You can also replace an installed game's icon here.",
            );
            ui.add_space(12.0);
            widgets::path_row(
                ui,
                "Game icon",
                &mut settings.icon_path,
                Pick::File,
                "PNG, JPEG, BMP or GIF",
            );
            ui.add_space(24.0);
            let label = |id: &str, name: &str| format!("{} ({id})", if name.is_empty() { id } else { name });
            let current = self
                .titles
                .iter()
                .find(|(id, _)| *id == self.award_title)
                .map(|(id, name)| label(id, name))
                .unwrap_or_else(|| "Choose an installed game".into());
            let mut options = vec!["Choose an installed game".to_string()];
            options.extend(self.titles.iter().map(|(id, name)| label(id, name)));
            let titles = &self.titles;
            let award_title = &mut self.award_title;
            install = widgets::field_row(ui, "Installed game", "Set icon", true, |ui, w| {
                if let Some(k) = widgets::select(ui, "award-title", &current, &options, w) {
                    *award_title = k
                        .checked_sub(1)
                        .and_then(|k| titles.get(k))
                        .map(|(id, _)| id.clone())
                        .unwrap_or_default();
                }
            });
            ui.add_space(12.0);
        });
        if install {
            self.install_icon(jobs, settings);
        }
    }

    pub fn central(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &mut Settings) {
        let rect = ui.max_rect();
        let right_w = ((rect.width() - 18.0) / 2.6).max(320.0);
        let stage = Rect::from_min_max(rect.min, egui::pos2(rect.max.x - right_w - 18.0, rect.max.y));
        let side = Rect::from_min_max(egui::pos2(rect.max.x - right_w, rect.min.y), rect.max);
        widgets::in_rect(ui, stage, |ui| self.stage(ui, stage, jobs, settings));
        widgets::in_rect(ui, side, |ui| self.side(ui, side, jobs, settings));
    }

    fn stage(&mut self, ui: &mut egui::Ui, full: Rect, jobs: &Jobs, settings: &Settings) {
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
        let strip_h = if tiles.is_empty() {
            96.0
        } else {
            96.0f32.max(widgets::strip_height(ui, &tiles, full.width()))
        };
        let view = Rect::from_min_max(
            full.min,
            egui::pos2(full.max.x, (full.max.y - strip_h - 10.0).max(full.min.y + 80.0)),
        );
        self.viewport.show(ui, view);
        let painter = ui.painter().clone();
        if self.viewport.viewer.is_none() {
            let total = widgets::line(15.0) + 10.0 + 12.0 + widgets::line(12.0);
            let y = view.center().y - total / 2.0;
            painter.text(
                egui::pos2(view.center().x, y),
                egui::Align2::CENTER_TOP,
                "Preview items on a mannequin",
                widgets::font(15.0, W::Semibold),
                c::STAGE_TEXT,
            );
            painter.text(
                egui::pos2(view.center().x, y + widgets::line(15.0) + 22.0),
                egui::Align2::CENTER_TOP,
                "Choose files, then select an item below.",
                widgets::font(12.0, W::Regular),
                c::STAGE_TEXT,
            );
        }
        if !self.preview_status.is_empty() {
            painter.text(
                view.left_bottom() + egui::vec2(14.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                &self.preview_status,
                widgets::font(11.0, W::Regular),
                c::CAPTION,
            );
        }
        let strip = Rect::from_min_max(egui::pos2(full.min.x, view.max.y + 10.0), full.max);
        if !tiles.is_empty() {
            let selected = self.selected.map(|s| s.to_string());
            let clicked = widgets::in_rect(ui, strip, |ui| {
                widgets::strip(ui, "import-items", &tiles, selected.as_deref(), true)
            });
            if let Some(i) = clicked {
                self.preview(i, jobs, settings);
            }
        }
    }

    fn side(&mut self, ui: &mut egui::Ui, rect: Rect, jobs: &Jobs, settings: &mut Settings) {
        let dragging = ui.input(|i| !i.raw.hovered_files.is_empty());
        widgets::window_card(ui, rect, "Import items", |ui| {
            let w = ui.available_width();
            let top = ui.cursor().min;
            let bg = ui.painter().add(egui::Shape::Noop);
            let mut choose = false;
            ui.vertical_centered(|ui| {
                ui.add_space(15.0);
                widgets::text(ui, "Choose items to import", 15.0, W::Semibold, c::TEXT);
                ui.add_space(10.0);
                widgets::text(
                    ui,
                    "STFS containers & raw .bin avatar items",
                    11.0,
                    W::Regular,
                    c::TEXT,
                );
                ui.add_space(12.0);
                choose = widgets::button(ui, "Choose items", Kind::Short, !self.busy).clicked();
                ui.add_space(15.0);
            });
            let target = Rect::from_min_max(top, egui::pos2(top.x + w, ui.cursor().min.y));
            let (fill, stroke) = if dragging {
                (widgets::rgb(0xE0F3C7), widgets::rgb(0x80B948))
            } else {
                (widgets::rgb(0xEFF7E6), widgets::rgb(0xA8BC92))
            };
            ui.painter().set(bg, egui::Shape::rect_filled(target, 7.0, fill));
            widgets::dashed_rect(ui.painter(), target, 7.0, egui::Stroke::new(1.0, stroke));
            if dragging {
                ui.painter().rect_stroke(
                    target,
                    7.0,
                    egui::Stroke::new(3.0, stroke),
                    egui::StrokeKind::Outside,
                );
            }
            if choose {
                if let Some(files) = rfd::FileDialog::new().pick_files() {
                    self.take_paths(files, jobs, settings);
                }
            }
            ui.add_space(12.0);
            let edit =
                widgets::labelled_input(ui, "Input file", &mut self.input, "Drop or choose an avatar item");
            if edit.lost_focus() && (edit.changed() || ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                let path = paths::expand(&self.input);
                if !path.as_os_str().is_empty() {
                    self.paths = vec![path];
                    self.analyze(jobs, settings);
                }
            }
            ui.add_space(12.0);
            if widgets::path_row(
                ui,
                "Closet destination",
                &mut settings.import_closet,
                Pick::Folder,
                "Choose a closet folder",
            ) {
                self.refresh_titles(jobs, settings);
            }
            ui.add_space(12.0);
            widgets::muted(ui, &self.summary);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, "Avatar award icons", Kind::Short, true).clicked() {
                    self.award_open = true;
                    self.refresh_titles(jobs, settings);
                }
                ui.add_space(8.0);
                if widgets::button(ui, "Validation log", Kind::Short, true).clicked() {
                    self.log_dialog.open();
                }
            });
        });
    }

    pub fn footer(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &Settings) {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::button(ui, "Import", Kind::Primary, !self.busy && self.validated).clicked() {
                self.import(jobs, settings);
            }
            ui.add_space(10.0);
            if widgets::button(ui, "Validate", Kind::Short, !self.busy && !self.paths.is_empty()).clicked() {
                self.analyze(jobs, settings);
            }
            ui.add_space(12.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&self.status)
                            .font(widgets::font(11.0, W::Regular))
                            .color(c::MUTED),
                    )
                    .truncate(),
                );
            });
        });
    }
}
