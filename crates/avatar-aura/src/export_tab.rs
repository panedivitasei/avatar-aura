// The Export tab of web/js/export.js, face-player.js, load-all.js and bones.js: load the avatar, pick a clip frame,
// expression or free pose, and export what is on screen.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use avatar_export::export::{Formats, PoseSource};
use avatar_export::faces::Expression;
use avatar_view::GizmoMode;
use eframe::egui::{self, Align, Layout, Rect, RichText};
use glam::{Mat4, Vec3};

use crate::bake::Inputs;
use crate::catalog::{self, Catalog};
use crate::jobs::{rgba_to_color, Jobs, LaneOutput, LaneTask, LoadAllResultParts, Msg, Rebuilt, TileOutcome};
use crate::paths;
use crate::session::{self, ExpressionTextures, Loaded, PreparedFace, TextureCache};
use crate::settings::Settings;
use crate::tiles;
use crate::viewport::Viewport;
use crate::widgets::{self, c, Kind, Pick, Tile, W};

const CHANNELS: [&str; 3] = ["mouth", "eyes", "brows"];

struct LoadAllRun {
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
}

/// The tile render that follows a complete Load All.
struct TileRun {
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
}

struct FacePlayer {
    face: Arc<PreparedFace>,
    playing: bool,
    time: f64,
    frame: u32,
    key: Option<usize>,
}

pub struct ExportTab {
    pub viewport: Viewport,
    textures: Arc<TextureCache>,
    log: Vec<String>,
    log_dialog: widgets::LogDialog,
    source_open: bool,
    pub status: String,
    loaded: Option<Loaded>,
    session: u64,
    load_generation: u64,
    loading: bool,
    exporting: bool,
    lane_busy: Option<LaneTask>,
    lane_queue: VecDeque<LaneTask>,
    load_all: Option<LoadAllRun>,
    tiles: Option<TileRun>,
    load_all_label: String,
    clip: Option<usize>,
    requested_clip: Option<usize>,
    no_animation: bool,
    playing: bool,
    playhead: f64,
    frame: u32,
    expression: [Option<u32>; 3],
    expression_request: u64,
    applied: Option<ExpressionTextures>,
    overridden: BTreeSet<usize>,
    faces: HashMap<usize, Arc<PreparedFace>>,
    /// Mirror of `Settings::split_animations` for the playback code.
    split: bool,
    face: Option<FacePlayer>,
    face_selected: Option<usize>,
    face_request: u64,
    face_status: String,
    free_pose: bool,
    /// Pose captured when free pose turned on, the gizmo's reset target, carried across viewer rebuilds.
    original: Vec<Mat4>,
    joint_names: Vec<String>,
}

fn inputs(settings: &Settings) -> Inputs {
    Inputs {
        manifest: paths::expand(&settings.manifest),
        pack: paths::expand(&settings.pack),
        closet: paths::expand(&settings.closet),
    }
}

impl ExportTab {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            textures: Arc::new(TextureCache::default()),
            log: Vec::new(),
            log_dialog: widgets::LogDialog::default(),
            source_open: false,
            status: String::new(),
            loaded: None,
            session: 0,
            load_generation: 0,
            loading: false,
            exporting: false,
            lane_busy: None,
            lane_queue: VecDeque::new(),
            load_all: None,
            tiles: None,
            load_all_label: String::new(),
            clip: None,
            requested_clip: None,
            no_animation: false,
            playing: false,
            playhead: 0.0,
            frame: 0,
            expression: [None; 3],
            expression_request: 0,
            applied: None,
            overridden: BTreeSet::new(),
            faces: HashMap::new(),
            split: false,
            face: None,
            face_selected: None,
            face_request: 0,
            face_status: "No face animation".into(),
            free_pose: false,
            original: Vec::new(),
            joint_names: Vec::new(),
        }
    }

    /// Activity log line, echoed to stdout when the app runs from a console.
    fn note(&mut self, line: impl Into<String>) {
        let line = line.into();
        println!("{line}");
        self.log.push(line);
    }

    pub fn note_line(&mut self, line: String) {
        self.note(line);
    }

    /// Counter that changes whenever a different avatar is shown; tile overrides belong to one value.
    pub fn session(&self) -> u64 {
        self.session
    }

    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.note(message.clone());
        self.status = message;
    }

    // ---- loading ----

    pub fn load(&mut self, jobs: &Jobs, settings: &Settings, catalog: &Catalog) {
        if self.loading || self.exporting {
            return;
        }
        self.loading = true;
        self.load_generation += 1;
        self.status = "Resolving your avatar...".into();
        self.note("=== load avatar ===");
        let generation = self.load_generation;
        let inputs = inputs(settings);
        let anim_dir = paths::expand(&settings.anim_dir);
        let catalog = catalog.clone();
        let gpu = self.viewport.gpu();
        jobs.spawn("load avatar", move |reply| {
            let result = (|| -> anyhow::Result<Rebuilt> {
                let loaded = session::load(&inputs, &anim_dir, &catalog, &mut |line| {
                    reply.send(Msg::ExportLog(line));
                })?;
                let viewer = gpu.build_viewer(&loaded.scene, &loaded.dir)?;
                Ok(Rebuilt { loaded, viewer })
            })()
            .map_err(|e| format!("{e:#}"));
            reply.send(Msg::AvatarLoaded { generation, result });
        });
    }

    fn on_loaded(&mut self, rebuilt: Rebuilt, jobs: &Jobs) {
        self.session += 1;
        self.lane_queue.clear();
        self.lane_busy = None;
        if let Some(run) = self.load_all.take() {
            run.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(run) = self.tiles.take() {
            run.cancel.store(true, Ordering::Relaxed);
        }
        self.load_all_label.clear();
        self.faces.clear();
        self.face = None;
        self.face_selected = None;
        self.face_status = "No face animation".into();
        self.expression = [None; 3];
        self.applied = None;
        self.overridden.clear();
        self.free_pose = false;
        self.joint_names = rebuilt
            .viewer
            .joint_names()
            .iter()
            .map(|s| s.to_string())
            .collect();
        self.viewport.replace(rebuilt.viewer, false);
        self.set_free_pose(false);
        let clips = rebuilt.loaded.clips.len();
        let stand = rebuilt
            .loaded
            .clips
            .iter()
            .position(|c| c.name == session::STAND_CLIP);
        self.loaded = Some(rebuilt.loaded);
        self.clip = None;
        self.requested_clip = None;
        self.no_animation = false;
        match stand {
            Some(i) => self.activate_clip(i, None),
            None => self.rest(),
        }
        self.status = format!("{clips} animations available.");
        self.note(self.status.clone());
        self.start_load_all(jobs);
    }

    /// Loading an avatar continues straight into the animation library and the strip tiles.
    fn start_load_all(&mut self, jobs: &Jobs) {
        let total = self.loaded.as_ref().map_or(0, |l| l.clips.len() * 2);
        self.load_all = Some(LoadAllRun {
            cancel: Arc::new(AtomicBool::new(false)),
            done: 0,
            total,
        });
        self.load_all_label = "Preparing animation library...".into();
        self.enqueue(LaneTask::LoadAll, jobs);
    }

    // ---- the scene lane ----

    fn enqueue(&mut self, task: LaneTask, jobs: &Jobs) {
        if let LaneTask::Clip(_) = task {
            self.lane_queue.retain(|t| !matches!(t, LaneTask::Clip(_)));
        }
        if self.lane_busy != Some(task) && !self.lane_queue.contains(&task) {
            self.lane_queue.push_back(task);
        }
        self.pump(jobs);
    }

    fn pump(&mut self, jobs: &Jobs) {
        if self.lane_busy.is_some() {
            return;
        }
        let Some(loaded) = self.loaded.clone() else {
            self.lane_queue.clear();
            return;
        };
        let Some(task) = self.lane_queue.pop_front() else {
            return;
        };
        self.lane_busy = Some(task);
        let session = self.session;
        let gpu = self.viewport.gpu();
        let textures = self.textures.clone();
        let cancel = self
            .load_all
            .as_ref()
            .map(|r| r.cancel.clone())
            .unwrap_or_default();
        jobs.spawn("scene lane", move |reply| {
            let rebuild = |next: Loaded| -> anyhow::Result<Rebuilt> {
                let viewer = gpu.build_viewer(&next.scene, &next.dir)?;
                Ok(Rebuilt { loaded: next, viewer })
            };
            let result = (|| -> anyhow::Result<LaneOutput> {
                match task {
                    LaneTask::Clip(i) => {
                        let anims = session::load_clips(&loaded, &[i])?;
                        Ok(LaneOutput::Clip(rebuild(loaded.with_animations(anims)?)?))
                    }
                    LaneTask::Face(i) => {
                        let rebuilt = if loaded.is_loaded(i) {
                            None
                        } else {
                            let anims = session::load_clips(&loaded, &[i])?;
                            Some(rebuild(loaded.with_animations(anims)?)?)
                        };
                        let source = rebuilt.as_ref().map_or(&loaded, |r| &r.loaded);
                        let face = session::prepare_face(source, i, &textures)?;
                        Ok(LaneOutput::Face { rebuilt, face })
                    }
                    LaneTask::LoadAll => {
                        let result =
                            session::load_all(&loaded, &cancel, &textures, &mut |done, total, label| {
                                reply.send(Msg::LoadAllProgress {
                                    session,
                                    done,
                                    total,
                                    label,
                                });
                            })?;
                        let (next, parts) = LoadAllResultParts::split(result);
                        Ok(LaneOutput::LoadAll {
                            rebuilt: rebuild(next)?,
                            result: parts,
                        })
                    }
                }
            })()
            .map_err(|e| format!("{e:#}"));
            reply.send(Msg::Lane {
                session,
                task,
                result,
            });
        });
    }

    /// Swaps in a rebuilt viewer with the same camera, pose and head textures.
    fn adopt(&mut self, rebuilt: Rebuilt) {
        let old = self.viewport.viewer.as_ref().map(|v| {
            let gizmo = v.gizmo();
            (v.joint_locals(), v.pose_edits(), gizmo.selected(), gizmo.mode())
        });
        self.viewport.replace(rebuilt.viewer, true);
        if let (Some(viewer), Some((locals, edits, selected, mode))) = (self.viewport.viewer.as_mut(), old) {
            if self.free_pose && self.original.len() == locals.len() {
                // Rebuilds the gizmo from the captured pose plus the edits so Reset still returns there.
                let _ = viewer.set_joint_locals(&self.original);
                viewer.set_gizmo_enabled(true);
                for edit in edits {
                    let _ = viewer.set_pose_edit(edit.joint, edit);
                }
                if let Some(joint) = selected {
                    viewer.gizmo_mut().select(joint);
                }
                viewer.gizmo_mut().set_mode(mode);
            } else {
                let _ = viewer.set_joint_locals(&locals);
                viewer.set_gizmo_enabled(self.free_pose);
            }
        }
        self.loaded = Some(rebuilt.loaded);
        self.overridden.clear();
        if let Some(face) = self.face.as_mut() {
            face.key = None;
            let frame = face.frame;
            self.seek_face(frame);
        } else if let Some(applied) = self.applied.clone() {
            self.apply_textures(&applied);
        }
    }

    fn on_lane(&mut self, task: LaneTask, result: Result<LaneOutput, String>, jobs: &Jobs) {
        self.lane_busy = None;
        match (task, result) {
            (_, Ok(LaneOutput::Clip(rebuilt))) => {
                self.adopt(rebuilt);
                if let LaneTask::Clip(i) = task {
                    if self.requested_clip == Some(i) {
                        self.activate_clip(i, None);
                        self.status = "Avatar ready.".into();
                    }
                }
            }
            (LaneTask::Face(i), Ok(LaneOutput::Face { rebuilt, face })) => {
                if let Some(rebuilt) = rebuilt {
                    self.adopt(rebuilt);
                }
                let face = Arc::new(face);
                self.faces.insert(i, face.clone());
                if self.face_selected == Some(i) {
                    self.start_face(face);
                }
            }
            (_, Ok(LaneOutput::LoadAll { rebuilt, result })) => {
                self.adopt(rebuilt);
                let clips = self.loaded.as_ref().map_or(0, |l| l.clips.len());
                for face in result.faces {
                    self.faces.insert(face.clip, Arc::new(face));
                }
                self.load_all = None;
                if result.done < result.total {
                    self.load_all_label = format!("Stopped · {} / {} loaded", result.done, result.total);
                } else {
                    self.load_all_label = format!("All {clips} animations and face animations loaded.");
                }
                self.note(self.load_all_label.clone());
                if result.done >= result.total {
                    self.start_tiles(jobs);
                }
            }
            (_, Ok(_)) => {}
            (task, Err(e)) => {
                match task {
                    LaneTask::Clip(_) => self.requested_clip = self.clip,
                    LaneTask::Face(i) if self.face_selected == Some(i) => {
                        self.face_selected = None;
                        self.face_status = "No face animation".into();
                    }
                    LaneTask::Face(_) => {}
                    LaneTask::LoadAll => {
                        let (done, total) = self.load_all.as_ref().map_or((0, 0), |r| (r.done, r.total));
                        self.load_all_label = format!("Loaded {done} / {total}. {e}");
                        self.load_all = None;
                    }
                }
                self.fail(e);
            }
        }
    }

    pub fn handle(&mut self, msg: Msg, jobs: &Jobs) {
        match msg {
            Msg::ExportLog(line) => self.note(line),
            Msg::AvatarLoaded { generation, result } => {
                if generation != self.load_generation {
                    return;
                }
                self.loading = false;
                match result {
                    Ok(rebuilt) => self.on_loaded(rebuilt, jobs),
                    Err(e) => self.fail(e),
                }
            }
            Msg::Lane {
                session,
                task,
                result,
            } => {
                if session == self.session {
                    self.on_lane(task, result, jobs);
                }
                self.pump(jobs);
            }
            Msg::LoadAllProgress {
                session,
                done,
                total,
                label,
            } => {
                if session == self.session {
                    if let Some(run) = self.load_all.as_mut() {
                        run.done = done;
                        run.total = total;
                    }
                    if !label.is_empty() {
                        self.load_all_label = label;
                    }
                }
            }
            Msg::TileProgress { session, done, total } => {
                if session == self.session {
                    if let Some(run) = self.tiles.as_mut() {
                        run.done = done;
                        run.total = total;
                        if !run.cancel.load(Ordering::Relaxed) {
                            self.load_all_label = format!("Rendering tiles {done} / {total}");
                        }
                    }
                }
            }
            Msg::TilesDone { session, result } => {
                if session == self.session && self.tiles.is_some() {
                    self.tiles = None;
                    self.on_tiles(result);
                }
            }
            Msg::Expression {
                session,
                request,
                result,
            } => {
                if session != self.session || request != self.expression_request {
                    return;
                }
                match result {
                    Ok(textures) => {
                        self.apply_textures(&textures);
                        self.applied = Some(textures);
                    }
                    Err(e) => self.fail(e),
                }
            }
            Msg::Face {
                session,
                request,
                result,
            } => {
                if session != self.session || request != self.face_request {
                    return;
                }
                match result {
                    Ok(face) => {
                        let face = Arc::new(face);
                        self.faces.insert(face.clip, face.clone());
                        if self.face_selected == Some(face.clip) {
                            self.start_face(face);
                        }
                    }
                    Err(e) => {
                        self.face_selected = None;
                        self.face_status = "No face animation".into();
                        self.fail(e);
                    }
                }
            }
            Msg::Exported(result) => {
                self.exporting = false;
                match result {
                    Ok((status, lines)) => {
                        self.log.extend(lines);
                        self.status = status;
                    }
                    Err(e) => self.fail(e),
                }
            }
            _ => {}
        }
    }

    // ---- strip tiles ----

    /// Renders the strip tiles from the current avatar on a worker; every message carries the session so a newer
    /// avatar drops them.
    fn start_tiles(&mut self, jobs: &Jobs) {
        let Some(loaded) = self.loaded.clone() else {
            return;
        };
        let total = tiles::total(&loaded);
        let cancel = Arc::new(AtomicBool::new(false));
        self.tiles = Some(TileRun {
            cancel: cancel.clone(),
            done: 0,
            total,
        });
        self.load_all_label = format!("Rendering tiles 0 / {total}");
        let session = self.session;
        let gpu = self.viewport.gpu();
        let textures = self.textures.clone();
        jobs.spawn("tiles", move |reply| {
            let started = std::time::Instant::now();
            let send = |images: Vec<tiles::Tile>| {
                let images = images
                    .into_iter()
                    .map(|(name, img)| (name, rgba_to_color(&img)))
                    .collect();
                reply.send(Msg::Tiles { session, images });
            };
            let outcome = |done| TileOutcome {
                done,
                total,
                seconds: started.elapsed().as_secs_f64(),
            };
            let result = (|| -> anyhow::Result<TileOutcome> {
                let faces = tiles::expression_tiles(&loaded, &gpu, &textures, &cancel, &mut |done| {
                    reply.send(Msg::TileProgress { session, done, total });
                })?;
                let Some(faces) = faces else {
                    return Ok(outcome(0));
                };
                let base = loaded.expressions.len();
                send(faces);
                let clips = tiles::clip_tiles(&loaded, &gpu, &textures, &cancel, &mut |done| {
                    reply.send(Msg::TileProgress {
                        session,
                        done: base + done,
                        total,
                    });
                })?;
                let Some(clips) = clips else {
                    return Ok(outcome(base));
                };
                send(clips);
                Ok(outcome(total))
            })()
            .map_err(|e| format!("{e:#}"));
            reply.send(Msg::TilesDone { session, result });
        });
    }

    fn on_tiles(&mut self, result: Result<TileOutcome, String>) {
        let clips = self.loaded.as_ref().map_or(0, |l| l.clips.len());
        match result {
            Ok(o) if o.done >= o.total => {
                self.note(format!("Rendered {} tiles in {:.1}s", o.total, o.seconds));
                self.load_all_label = format!("All {clips} animations and face animations loaded.");
            }
            Ok(o) => {
                self.load_all_label = format!("Tiles stopped · {} / {} rendered", o.done, o.total);
                self.note(self.load_all_label.clone());
            }
            Err(e) => {
                self.load_all_label = format!("Tiles failed. {e}");
                self.note(self.load_all_label.clone());
            }
        }
    }

    // ---- animation ----

    fn scene_clip(&self) -> Option<usize> {
        self.loaded.as_ref()?.scene_index(self.clip?)
    }

    fn clip_frames(&self) -> u32 {
        self.clip
            .and_then(|c| self.loaded.as_ref()?.clips.get(c))
            .map_or(1, |c| c.frames.max(1))
    }

    fn clip_fps(&self) -> f64 {
        self.clip
            .and_then(|c| self.loaded.as_ref()?.clips.get(c))
            .map_or(30.0, |c| if c.fps > 0.0 { c.fps } else { 30.0 })
    }

    fn pose_at(&mut self, frame: f32) {
        let Some(index) = self.scene_clip() else {
            return;
        };
        if let Some(viewer) = self.viewport.viewer.as_mut() {
            if let Err(e) = viewer.set_clip_frame(index, frame) {
                self.status = e.to_string();
            }
        }
    }

    fn activate_clip(&mut self, clip: usize, jobs: Option<&Jobs>) {
        let Some(loaded) = &self.loaded else {
            return;
        };
        self.requested_clip = Some(clip);
        if !loaded.is_loaded(clip) {
            let name = loaded.clips.get(clip).map(|c| c.name.clone()).unwrap_or_default();
            self.status = format!("Loading {name}...");
            if let Some(jobs) = jobs {
                self.enqueue(LaneTask::Clip(clip), jobs);
            }
            return;
        }
        self.set_free_pose(false);
        self.no_animation = false;
        self.clip = Some(clip);
        self.playing = true;
        self.playhead = 0.0;
        self.frame = 0;
        self.pose_at(0.0);
    }

    /// "No animation": the bind pose, which is also what the rigged export writes.
    fn rest(&mut self) {
        self.no_animation = true;
        self.set_free_pose(false);
        self.clip = None;
        self.requested_clip = None;
        self.playing = false;
        self.frame = 0;
        if let Some(viewer) = self.viewport.viewer.as_mut() {
            viewer.set_rest_pose();
        }
    }

    fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.frame = ((self.playhead * self.clip_fps()).round() as u32).min(self.clip_frames() - 1);
            self.pose_at(self.frame as f32);
        }
    }

    fn seek(&mut self, frame: u32) {
        self.playing = false;
        self.frame = frame.min(self.clip_frames() - 1);
        self.playhead = f64::from(self.frame) / self.clip_fps();
        self.pose_at(self.frame as f32);
        if !self.split {
            self.sync_face();
        }
    }

    /// Combined mode: the face player sits at the body clip's time instead of running its own clock.
    fn sync_face(&mut self) {
        let Some(face) = &self.face else {
            return;
        };
        let anim = &face.face.animation;
        let fps = if anim.fps > 0.0 { anim.fps } else { 30.0 };
        let frame = ((self.playhead * fps).floor() as u32).min(anim.frames.saturating_sub(1));
        if frame != face.frame {
            self.seek_face(frame);
        }
    }

    /// Advances the clip and face players; true while either needs another frame.
    pub fn tick(&mut self, dt: f64) -> bool {
        let mut animating = false;
        if self.playing && !self.free_pose && self.scene_clip().is_some() {
            let fps = self.clip_fps();
            let frames = self.clip_frames();
            let length = f64::from(frames) / fps;
            self.playhead = (self.playhead + dt) % length.max(1e-3);
            let f = (self.playhead * fps).min(f64::from(frames - 1));
            self.frame = (f.round() as u32).min(frames - 1);
            self.pose_at(f as f32);
            animating = true;
        }
        if !self.split {
            if animating {
                self.sync_face();
            }
        } else if let Some(face) = self.face.as_mut() {
            if face.playing {
                let anim = &face.face.animation;
                let fps = if anim.fps > 0.0 { anim.fps } else { 30.0 };
                let length = f64::from(anim.frames.max(1)) / fps;
                face.time = (face.time + dt) % length.max(1e-3);
                let frame = ((face.time * fps).floor() as u32).min(anim.frames.saturating_sub(1));
                if frame != face.frame {
                    self.seek_face(frame);
                }
                animating = true;
            }
        }
        animating || self.loading || self.lane_busy.is_some() || self.exporting
    }

    // ---- expressions and the face player ----

    fn apply_textures(&mut self, textures: &ExpressionTextures) {
        let Some(viewer) = self.viewport.viewer.as_mut() else {
            return;
        };
        for (index, image) in textures {
            if viewer.set_material_texture(*index, image).is_ok() {
                self.overridden.insert(*index);
            }
        }
    }

    fn restore_expression(&mut self) {
        if let Some(applied) = self.applied.clone() {
            self.apply_textures(&applied);
        } else if let Some(viewer) = self.viewport.viewer.as_mut() {
            for index in std::mem::take(&mut self.overridden) {
                let _ = viewer.reset_material_texture(index);
            }
        }
    }

    fn current_expression(&self) -> Expression {
        if let Some(face) = &self.face {
            let anim = &face.face.animation;
            let key = anim.sequence.get(face.frame as usize).copied().unwrap_or(0);
            if let Some(entry) = anim.entries.get(key) {
                return entry.id.clone();
            }
        }
        let [m, e, b] = self.expression;
        session::selection(m, e, b)
    }

    fn choose_expression(&mut self, channel: usize, value: Option<u32>, jobs: &Jobs) {
        let Some(loaded) = self.loaded.clone() else {
            return;
        };
        self.face = None;
        self.face_selected = None;
        self.face_status = "No face animation".into();
        self.expression[channel] = value;
        self.expression_request += 1;
        let (session, request) = (self.session, self.expression_request);
        let expression = self.current_expression();
        let textures = self.textures.clone();
        jobs.spawn("expression", move |reply| {
            let result =
                session::mix_expression(&loaded, &expression, &textures).map_err(|e| format!("{e:#}"));
            reply.send(Msg::Expression {
                session,
                request,
                result,
            });
        });
    }

    fn choose_face(&mut self, clip: Option<usize>, jobs: &Jobs) {
        self.face_request += 1;
        let Some(clip) = clip else {
            self.face = None;
            self.face_selected = None;
            self.face_status = "No face animation".into();
            self.restore_expression();
            return;
        };
        let Some(loaded) = self.loaded.clone() else {
            return;
        };
        self.face = None;
        self.face_selected = Some(clip);
        if let Some(face) = self.faces.get(&clip).cloned() {
            self.start_face(face);
            return;
        }
        self.face_status = "Loading face animation...".into();
        if loaded.is_loaded(clip) {
            let (session, request) = (self.session, self.face_request);
            let textures = self.textures.clone();
            jobs.spawn("face animation", move |reply| {
                let result = session::prepare_face(&loaded, clip, &textures).map_err(|e| format!("{e:#}"));
                reply.send(Msg::Face {
                    session,
                    request,
                    result,
                });
            });
        } else {
            self.enqueue(LaneTask::Face(clip), jobs);
        }
    }

    fn start_face(&mut self, face: Arc<PreparedFace>) {
        self.face = Some(FacePlayer {
            face,
            playing: true,
            time: 0.0,
            frame: 0,
            key: None,
        });
        self.seek_face(0);
        if !self.split {
            self.sync_face();
        }
    }

    fn seek_face(&mut self, frame: u32) {
        let Some(player) = self.face.as_mut() else {
            return;
        };
        let anim = &player.face.animation;
        player.frame = frame.min(anim.frames.saturating_sub(1));
        let fps = if anim.fps > 0.0 { anim.fps } else { 30.0 };
        player.time = f64::from(player.frame) / fps;
        let key = anim.sequence.get(player.frame as usize).copied();
        if key != player.key {
            player.key = key;
            let entry = key.and_then(|k| player.face.entries.get(k)).cloned();
            if let Some(textures) = entry {
                self.apply_textures(&textures);
            }
        }
        if let Some(player) = &self.face {
            self.face_status = format!("{} / Frame {}", player.face.animation.name, player.frame);
        }
    }

    // ---- free pose ----

    /// bones.js `setEnabled`: pauses playback and captures the reset target; the viewer draws and drives the gizmo.
    fn set_free_pose(&mut self, enabled: bool) {
        if enabled && !self.free_pose {
            if self.viewport.viewer.is_none() {
                return;
            }
            self.pause();
            self.playing = false;
            self.original = self
                .viewport
                .viewer
                .as_ref()
                .map(|v| v.joint_locals())
                .unwrap_or_default();
        }
        self.free_pose = enabled && self.viewport.viewer.is_some();
        if let Some(viewer) = self.viewport.viewer.as_mut() {
            viewer.set_gizmo_enabled(self.free_pose);
        }
    }

    // ---- export ----

    fn export(&mut self, jobs: &Jobs, settings: &Settings) {
        if self.exporting || self.loading {
            return;
        }
        let Some(loaded) = self.loaded.clone() else {
            return;
        };
        if self.lane_busy.is_some() {
            self.status = "Wait for the animation to finish loading.".into();
            return;
        }
        self.pause();
        if let Some(face) = self.face.as_mut() {
            face.playing = false;
        }
        let f = &settings.formats;
        let formats = Formats {
            dae: f.dae,
            glb: f.glb,
            obj: f.obj,
            smd: f.smd,
        };
        if !(f.dae || f.glb || f.obj || f.smd) {
            self.status = "Choose at least one supported format.".into();
            return;
        }
        let destination = paths::expand(&settings.output);
        if destination.as_os_str().is_empty() {
            self.status = "Choose an output directory.".into();
            return;
        }
        let expression = self.current_expression();
        let frame = self.frame;
        let pose = if self.free_pose {
            let locals = self
                .viewport
                .viewer
                .as_ref()
                .map(|v| v.joint_locals())
                .unwrap_or_default();
            Some(PoseSource::Free(session::pose_bones(&self.joint_names, &locals)))
        } else if self.no_animation {
            None
        } else {
            match self.scene_clip() {
                Some(clip) => Some(PoseSource::Clip {
                    clip,
                    frame: i64::from(frame),
                }),
                None => {
                    self.status = "Choose an animation before exporting.".into();
                    return;
                }
            }
        };
        self.exporting = true;
        self.status = match &pose {
            Some(_) => format!("Exporting frame {frame}..."),
            None => "Baking and exporting the rigged avatar...".into(),
        };
        jobs.spawn("export", move |reply| {
            let result = match pose {
                Some(pose) => session::export_posed(
                    &loaded,
                    pose,
                    expression,
                    formats,
                    frame.to_string(),
                    &destination,
                )
                .map(|out| {
                    (
                        format!("Exported frame {frame} in {} formats.", out.written.len()),
                        out.log,
                    )
                }),
                None => {
                    let mut stream = |line: String| reply.send(Msg::ExportLog(line));
                    session::export_rigged(&loaded.inputs, formats, &destination, &mut stream).map(
                        |(root, written)| {
                            (
                                format!("Exported the rigged avatar to {}.", root.display()),
                                written.iter().map(|p| format!("wrote {}", p.display())).collect(),
                            )
                        },
                    )
                }
            };
            reply.send(Msg::Exported(result.map_err(|e| format!("{e:#}"))));
        });
    }

    // ---- UI ----

    pub fn windows(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        self.log_dialog.show(ctx, "Activity log", &self.log);
        if self.source_open {
            self.source_open = widgets::dialog(ctx, "source-dialog", "Avatar source", |ui| {
                let col = (ui.available_width() - 18.0) / 2.0;
                let fields: [(&str, &mut String, Pick, &str); 4] = [
                    ("Saved avatar", &mut settings.manifest, Pick::File, ""),
                    ("Asset pack", &mut settings.pack, Pick::File, ""),
                    ("Closet", &mut settings.closet, Pick::Folder, ""),
                    (
                        "Extra animations folder (optional)",
                        &mut settings.anim_dir,
                        Pick::Folder,
                        "*.AvatarAnimation files",
                    ),
                ];
                let mut fields = fields.into_iter();
                while let Some(first) = fields.next() {
                    let second = fields.next();
                    ui.horizontal_top(|ui| {
                        for (i, (label, value, pick, hint)) in
                            std::iter::once(first).chain(second).enumerate()
                        {
                            if i > 0 {
                                ui.add_space(18.0);
                            }
                            ui.vertical(|ui| {
                                ui.set_width(col);
                                ui.add_space(12.0);
                                widgets::path_row(ui, label, value, pick, hint);
                                ui.add_space(12.0);
                            });
                        }
                    });
                }
            });
        }
    }

    pub fn central(
        &mut self,
        ui: &mut egui::Ui,
        jobs: &Jobs,
        settings: &mut Settings,
        catalog: &Catalog,
        thumbs: &HashMap<String, egui::TextureHandle>,
    ) {
        let rect = ui.max_rect();
        let right_w = ((rect.width() - 18.0) / 2.6).max(320.0);
        let stage = Rect::from_min_max(rect.min, egui::pos2(rect.max.x - right_w - 18.0, rect.max.y));
        let side = Rect::from_min_max(egui::pos2(rect.max.x - right_w, rect.min.y), rect.max);
        widgets::in_rect(ui, stage, |ui| self.stage(ui, stage, jobs, settings, catalog));
        widgets::in_rect(ui, side, |ui| {
            self.side(ui, side, jobs, settings, catalog, thumbs)
        });
    }

    fn stage(&mut self, ui: &mut egui::Ui, rect: Rect, jobs: &Jobs, settings: &Settings, catalog: &Catalog) {
        self.viewport.show(ui, rect);
        let painter = ui.painter().clone();
        if self.viewport.viewer.is_none() {
            let button_h = widgets::line(13.0) + 16.0;
            let total = widgets::line(12.0) + 12.0 + button_h;
            let mut y = rect.center().y - total / 2.0;
            painter.text(
                egui::pos2(rect.center().x, y),
                egui::Align2::CENTER_TOP,
                "Load a saved avatar to begin.",
                widgets::font(12.0, W::Regular),
                c::STAGE_TEXT,
            );
            y += widgets::line(12.0) + 12.0;
            let w = painter
                .layout_no_wrap("Load avatar".into(), widgets::font(13.0, W::Regular), c::TEXT)
                .size()
                .x
                + 20.0;
            let button =
                Rect::from_min_size(egui::pos2(rect.center().x - w / 2.0, y), egui::vec2(w, button_h));
            if widgets::button_in(ui, button, "Load avatar", Kind::Primary, !self.loading).clicked() {
                self.load(jobs, settings, catalog);
            }
        }
    }

    fn expression_tiles(
        &self,
        channel: &str,
        catalog: &Catalog,
        thumbs: &HashMap<String, egui::TextureHandle>,
    ) -> Vec<Tile> {
        let thumb = |id: &str| thumbs.get(&catalog::expression_thumb(id)).map(|t| t.id());
        let mut tiles = vec![Tile {
            key: format!("{channel}:none"),
            label: "None".into(),
            thumb: thumb("neutral"),
            badge: None,
        }];
        let entries: Vec<(String, String)> = match &self.loaded {
            Some(l) => l
                .expressions
                .iter()
                .map(|e| (e.id.clone(), e.name.clone()))
                .collect(),
            None => catalog
                .expressions
                .iter()
                .map(|e| (e.id.clone(), e.name.clone()))
                .collect(),
        };
        for (id, name) in entries {
            if !id.starts_with(&format!("{channel}:")) || id == format!("{channel}:0") {
                continue;
            }
            let label = name.rsplit('·').next().unwrap_or(&name).trim().to_string();
            tiles.push(Tile {
                thumb: thumb(&id),
                key: id,
                label,
                badge: None,
            });
        }
        tiles
    }

    fn clip_tiles(
        &self,
        none: &str,
        catalog: &Catalog,
        thumbs: &HashMap<String, egui::TextureHandle>,
    ) -> Vec<Tile> {
        let mut tiles = vec![Tile {
            key: "none".into(),
            label: none.into(),
            thumb: None,
            badge: None,
        }];
        let clips: Vec<(String, String)> = match &self.loaded {
            Some(l) => l
                .clips
                .iter()
                .map(|c| (c.name.clone(), c.thumb.clone()))
                .collect(),
            None => catalog
                .clips
                .iter()
                .map(|c| (c.name.clone(), c.thumb.clone()))
                .collect(),
        };
        for (i, (name, thumb)) in clips.into_iter().enumerate() {
            tiles.push(Tile {
                key: i.to_string(),
                label: name.strip_prefix("Animation ").unwrap_or(&name).to_string(),
                thumb: thumbs.get(&thumb).map(|t| t.id()),
                badge: None,
            });
        }
        tiles
    }

    fn side(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        jobs: &Jobs,
        settings: &mut Settings,
        catalog: &Catalog,
        thumbs: &HashMap<String, egui::TextureHandle>,
    ) {
        self.split = settings.split_animations;
        widgets::window_card(ui, rect, "Pose your avatar", |ui| {
            self.load_all_ui(ui);
            self.transport_ui(ui);
            let active = self.loaded.is_some() && !self.exporting;
            for (c, channel) in CHANNELS.iter().enumerate() {
                ui.add_space(16.0);
                widgets::h3(ui, catalog_title(channel));
                let tiles = self.expression_tiles(channel, catalog, thumbs);
                let selected = match self.expression[c] {
                    Some(n) => format!("{channel}:{n}"),
                    None => format!("{channel}:none"),
                };
                if let Some(i) = widgets::strip(ui, channel, &tiles, Some(&selected), active) {
                    let value = tiles[i].key.rsplit(':').next().and_then(|v| v.parse().ok());
                    self.choose_expression(c, value, jobs);
                }
            }
            ui.add_space(16.0);
            widgets::h3(ui, "Animation");
            if self.loaded.is_none() {
                widgets::muted(ui, "Load an avatar to choose a clip.");
            } else {
                let tiles = self.clip_tiles("No animation", catalog, thumbs);
                let selected = if self.no_animation {
                    "none".to_string()
                } else {
                    self.requested_clip
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "none".into())
                };
                if let Some(i) = widgets::strip(ui, "animation", &tiles, Some(&selected), active) {
                    if i == 0 {
                        self.rest();
                    } else {
                        self.activate_clip(i - 1, Some(jobs));
                    }
                    if !settings.split_animations {
                        self.choose_face(i.checked_sub(1), jobs);
                    }
                }
            }
            ui.add_space(12.0);
            if widgets::checkbox(
                ui,
                &mut settings.split_animations,
                "Split animations",
                14.0,
                active,
            )
            .changed()
                && !settings.split_animations
            {
                self.split = false;
                let clip = if self.no_animation {
                    None
                } else {
                    self.requested_clip
                };
                self.choose_face(clip, jobs);
            }
            if settings.split_animations {
                ui.add_space(16.0);
                widgets::h3(ui, "Face animation");
                if self.loaded.is_some() {
                    let tiles = self.clip_tiles("No face animation", catalog, thumbs);
                    let selected = self
                        .face_selected
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "none".into());
                    if let Some(i) = widgets::strip(ui, "face-animation", &tiles, Some(&selected), active) {
                        self.choose_face(i.checked_sub(1), jobs);
                    }
                } else {
                    ui.add_space(96.0);
                }
                self.face_ui(ui);
            }
            ui.add_space(18.0);
            let w = ui.available_width();
            let (rule, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
            ui.painter().rect_filled(rule, 0.0, c::LINE);
            ui.add_space(12.0);
            self.pose_ui(ui);
            ui.add_space(18.0);
            widgets::text(ui, "Formats", 12.0, W::Regular, c::TEXT);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let f = &mut settings.formats;
                for (value, label) in [
                    (&mut f.glb, "GLB"),
                    (&mut f.obj, "OBJ"),
                    (&mut f.dae, "DAE"),
                    (&mut f.smd, "SMD"),
                ] {
                    widgets::checkbox(ui, value, label, 12.0, true);
                    ui.add_space(15.0);
                }
            });
            ui.add_space(10.0 + 18.0 + 12.0);
            widgets::path_row(
                ui,
                "Output directory",
                &mut settings.output,
                Pick::Folder,
                "Choose an output folder",
            );
        });
    }

    fn load_all_ui(&mut self, ui: &mut egui::Ui) {
        let running = self.load_all.is_some() || self.tiles.is_some();
        if running {
            let cancelling = self
                .load_all
                .as_ref()
                .map(|r| &r.cancel)
                .or(self.tiles.as_ref().map(|r| &r.cancel))
                .is_some_and(|c| c.load(Ordering::Relaxed));
            if widgets::button(ui, "Cancel", Kind::Short, !cancelling && !self.exporting).clicked() {
                if let Some(run) = &self.load_all {
                    run.cancel.store(true, Ordering::Relaxed);
                } else if let Some(run) = &self.tiles {
                    run.cancel.store(true, Ordering::Relaxed);
                    self.load_all_label = "Stopping the tile render...".into();
                }
            }
            ui.add_space(12.0);
        }
        if !self.load_all_label.is_empty() {
            widgets::muted(ui, &self.load_all_label);
            ui.add_space(12.0);
        }
        let counters = self
            .load_all
            .as_ref()
            .map(|r| (r.done, r.total))
            .or(self.tiles.as_ref().map(|r| (r.done, r.total)));
        if let Some((done, total)) = counters {
            let fraction = if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            };
            widgets::progress(ui, fraction);
            ui.add_space(10.0);
        }
    }

    /// `.transport`: a button at the left and a 12px label pushed to the right.
    fn transport_row(ui: &mut egui::Ui, button: &str, enabled: bool, label: &str) -> bool {
        let mut clicked = false;
        ui.horizontal(|ui| {
            clicked = widgets::button(ui, button, Kind::Short, enabled).clicked();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::text(ui, label, 12.0, W::Regular, c::TEXT);
            });
        });
        clicked
    }

    /// Play, the frame slider and the frame counter, laid out in the footer next to the status text.
    fn transport_footer(&mut self, ui: &mut egui::Ui) {
        let usable = self.clip.is_some() && !self.no_animation && !self.free_pose && self.loaded.is_some();
        let label = if self.playing { "Pause" } else { "Play" };
        let (rect, _) = ui.allocate_exact_size(widgets::button_size(ui, "Pause"), egui::Sense::hover());
        if widgets::button_in(ui, rect, label, Kind::Short, usable).clicked() {
            if self.playing {
                self.pause();
            } else {
                self.playing = true;
                self.playhead = f64::from(self.frame) / self.clip_fps();
            }
        }
        ui.add_space(14.0);
        let mut frame = self.frame;
        let max = self.clip_frames().saturating_sub(1);
        let changed = ui
            .allocate_ui_with_layout(
                egui::vec2(260.0, 10.0),
                Layout::left_to_right(Align::Center),
                |ui| widgets::slider(ui, &mut frame, max, usable).changed(),
            )
            .inner;
        if changed {
            self.seek(frame);
        }
        ui.add_space(14.0);
        widgets::text(ui, format!("Frame {}", self.frame), 12.0, W::Regular, c::TEXT);
    }

    /// The selected clip's name under the Load All row.
    fn transport_ui(&mut self, ui: &mut egui::Ui) {
        let clip_name = match (
            self.no_animation,
            self.clip.and_then(|c| self.loaded.as_ref()?.clips.get(c)),
        ) {
            (false, Some(c)) => format!("{} \u{b7} {:.0} fps", c.name, c.fps),
            _ if self.loaded.is_none() => "No animation loaded".into(),
            _ => "No animation".into(),
        };
        widgets::muted(ui, clip_name);
    }

    fn face_ui(&mut self, ui: &mut egui::Ui) {
        let has = self.face.is_some();
        let label = if self.face.as_ref().is_some_and(|f| f.playing) {
            "Pause face"
        } else {
            "Play face"
        };
        ui.add_space(8.0);
        let status = self.face_status.clone();
        if Self::transport_row(ui, label, has, &status) {
            if let Some(face) = self.face.as_mut() {
                face.playing = !face.playing;
            }
        }
        ui.add_space(8.0 + 12.0);
        let (mut frame, max) = self
            .face
            .as_ref()
            .map_or((0, 0), |f| (f.frame, f.face.animation.frames.saturating_sub(1)));
        if widgets::slider(ui, &mut frame, max, has).changed() {
            if let Some(face) = self.face.as_mut() {
                face.playing = false;
            }
            self.seek_face(frame);
        }
        ui.add_space(12.0);
    }

    fn pose_ui(&mut self, ui: &mut egui::Ui) {
        let mut enabled = self.free_pose;
        let usable = self.viewport.viewer.is_some() && self.lane_busy.is_none();
        if widgets::checkbox(ui, &mut enabled, "Free pose", 14.0, usable).changed() {
            self.set_free_pose(enabled);
        }
        ui.add_space(10.0);
        if !self.free_pose {
            return;
        }
        ui.add_space(2.0);
        widgets::muted(
            ui,
            "Rotate the head, torso, arms or legs. Select Whole avatar to move the character.",
        );
        ui.add_space(12.0);
        let Some(viewer) = self.viewport.viewer.as_mut() else {
            return;
        };
        let listed = viewer.gizmo().listed_joints().to_vec();
        let current = viewer
            .gizmo()
            .selected()
            .and_then(|s| listed.iter().find(|(j, _)| *j == s))
            .map_or("", |(_, label)| label);
        widgets::field_label(ui, "Bone");
        ui.add_space(5.0);
        let options: Vec<String> = listed.iter().map(|(_, l)| (*l).to_string()).collect();
        let w = ui.available_width();
        if let Some(k) = widgets::select(ui, "bone-select", current, &options, w) {
            viewer.gizmo_mut().select(listed[k].0);
        }
        let Some(joint) = viewer.gizmo().selected() else {
            return;
        };
        let can_translate = viewer.gizmo().can_translate();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, "Rotate", Kind::Short, true).clicked() {
                viewer.gizmo_mut().set_mode(GizmoMode::Rotate);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button(ui, "Move", Kind::Short, can_translate).clicked() {
                    viewer.gizmo_mut().set_mode(GizmoMode::Translate);
                }
            });
        });
        ui.add_space(8.0);
        let translate = viewer.gizmo().mode() == GizmoMode::Translate;
        let Some(pose) = viewer.joint_pose(joint) else {
            return;
        };
        let mut values = if translate {
            pose.translation.to_array()
        } else {
            pose.rotation_degrees.to_array()
        };
        let mut changed = false;
        let col = (ui.available_width() - 16.0) / 3.0;
        let enabled = !translate || can_translate;
        ui.horizontal(|ui| {
            for (i, (axis, value)) in ["X", "Y", "Z"].iter().zip(values.iter_mut()).enumerate() {
                if i > 0 {
                    ui.add_space(8.0);
                }
                ui.vertical(|ui| {
                    ui.set_width(col);
                    widgets::field_label(ui, axis);
                    ui.add_space(5.0);
                    ui.scope(|ui| {
                        widgets::input_style(ui);
                        let drag = if translate {
                            egui::DragValue::new(value).speed(0.005).fixed_decimals(4)
                        } else {
                            egui::DragValue::new(value).speed(0.5).fixed_decimals(2)
                        };
                        changed |= ui
                            .add_enabled_ui(enabled, |ui| ui.add_sized([col, widgets::INPUT_H], drag))
                            .inner
                            .changed();
                    });
                });
            }
        });
        if changed && values.iter().all(|v| v.is_finite()) {
            let mut edit = pose;
            if translate {
                edit.translation = Vec3::from_array(values);
            } else {
                edit.rotation_degrees = Vec3::from_array(values);
            }
            if let Err(e) = viewer.set_pose_edit(joint, edit) {
                self.status = e.to_string();
            }
        }
        ui.add_space(12.0);
        widgets::muted(
            ui,
            if translate {
                "Local position in metres"
            } else {
                "Local rotation in degrees"
            },
        );
        ui.add_space(12.0);
        let mut reset: Option<Option<usize>> = None;
        ui.horizontal(|ui| {
            if widgets::button(ui, "Reset bone", Kind::Short, true).clicked() {
                reset = Some(Some(joint));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button(ui, "Reset pose", Kind::Short, true).clicked() {
                    reset = Some(None);
                }
            });
        });
        let result = match reset {
            Some(Some(joint)) => viewer.reset_joint(joint),
            Some(None) => viewer.reset_pose(),
            None => Ok(()),
        };
        if let Err(e) = result {
            self.status = e.to_string();
        }
        ui.add_space(8.0);
    }

    pub fn footer(&mut self, ui: &mut egui::Ui, jobs: &Jobs, settings: &Settings, catalog: &Catalog) {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let can_export =
                self.loaded.is_some() && !self.exporting && !self.loading && self.lane_busy.is_none();
            if widgets::button(ui, "Export", Kind::Primary, can_export).clicked() {
                self.export(jobs, settings);
            }
            ui.add_space(10.0);
            if widgets::button(ui, "Reload avatar", Kind::Short, !self.loading && !self.exporting).clicked() {
                self.load(jobs, settings, catalog);
            }
            ui.add_space(10.0);
            if widgets::button(ui, "Activity log", Kind::Short, true).clicked() {
                self.log_dialog.open();
            }
            ui.add_space(10.0);
            if widgets::button(ui, "Avatar source", Kind::Short, true).clicked() {
                self.source_open = true;
            }
            ui.add_space(12.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&self.status)
                            .font(widgets::font(11.0, W::Regular))
                            .color(c::MUTED),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
                ui.add_space(24.0);
                self.transport_footer(ui);
            });
        });
    }
}

fn catalog_title(channel: &str) -> &'static str {
    match channel {
        "mouth" => "Mouth",
        "eyes" => "Eyes",
        _ => "Brows",
    }
}
