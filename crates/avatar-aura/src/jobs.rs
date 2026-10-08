// Background work: each job runs on its own thread and reports through one channel that the UI drains per frame.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use avatar_export::scene::Scene;
use avatar_view::Viewer;
use eframe::egui;

use crate::import::Outcome;
use crate::session::{ExpressionTextures, LoadAllResult, Loaded, PreparedFace};

/// A rebuilt viewer and the session state it shows.
pub struct Rebuilt {
    pub loaded: Loaded,
    pub viewer: Viewer,
}

/// A mannequin preview ready to show.
pub struct ImportPreview {
    pub index: usize,
    pub viewer: Viewer,
    pub scene: Arc<Scene>,
    pub dir: std::path::PathBuf,
}

/// The serialized scene lane: clip loads, face animations that need a clip, and Load All.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LaneTask {
    Clip(usize),
    Face(usize),
    LoadAll,
}

pub enum LaneOutput {
    Clip(Rebuilt),
    Face {
        rebuilt: Option<Rebuilt>,
        face: PreparedFace,
    },
    LoadAll {
        rebuilt: Rebuilt,
        result: LoadAllResultParts,
    },
}

/// Load All counters and prepared faces, without the session that moved into `Rebuilt`.
pub struct LoadAllResultParts {
    pub faces: Vec<PreparedFace>,
    pub done: usize,
    pub total: usize,
}

impl LoadAllResultParts {
    pub fn split(result: LoadAllResult) -> (Loaded, Self) {
        (
            result.loaded,
            Self {
                faces: result.faces,
                done: result.done,
                total: result.total,
            },
        )
    }
}

/// How a tile run ended; `done` falls short of `total` when cancelled.
pub struct TileOutcome {
    pub done: usize,
    pub total: usize,
    pub seconds: f64,
}

pub enum Msg {
    Thumbs(Vec<(String, egui::ColorImage)>),
    ImportAnalyzed {
        session: u64,
        outcome: Outcome,
        icons: Vec<Option<egui::ColorImage>>,
    },
    ImportWritten(Outcome),
    TitleIcon(Outcome),
    Titles(Vec<(String, String)>),
    ImportPreview {
        session: u64,
        generation: u64,
        log: Vec<String>,
        result: Result<ImportPreview, String>,
    },
    ExportLog(String),
    AvatarLoaded {
        generation: u64,
        result: Result<Rebuilt, String>,
    },
    Lane {
        session: u64,
        task: LaneTask,
        result: Result<LaneOutput, String>,
    },
    LoadAllProgress {
        session: u64,
        done: usize,
        total: usize,
        label: String,
    },
    /// Strip tiles rendered from the session's avatar, replacing the bundled ones under the same keys.
    Tiles {
        session: u64,
        images: Vec<(String, egui::ColorImage)>,
    },
    TileProgress {
        session: u64,
        done: usize,
        total: usize,
    },
    TilesDone {
        session: u64,
        result: Result<TileOutcome, String>,
    },
    Expression {
        session: u64,
        request: u64,
        result: Result<ExpressionTextures, String>,
    },
    Face {
        session: u64,
        request: u64,
        result: Result<PreparedFace, String>,
    },
    Exported(Result<(String, Vec<String>), String>),
    Failed(String),
}

/// Sender handed to a job; every send wakes the UI.
#[derive(Clone)]
pub struct Reply {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Reply {
    pub fn send(&self, msg: Msg) {
        // A closed channel means the window is gone; the result has nowhere to go.
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

pub struct Jobs {
    reply: Reply,
    rx: Receiver<Msg>,
}

impl Jobs {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            reply: Reply { tx, ctx: ctx.clone() },
            rx,
        }
    }

    /// Runs `f` on a new thread; a panic inside it is reported as `Msg::Failed`.
    pub fn spawn(&self, name: &str, f: impl FnOnce(&Reply) + Send + 'static) {
        let reply = self.reply.clone();
        let label = name.to_string();
        let spawned = std::thread::Builder::new()
            .name(format!("aura-{label}"))
            .spawn(move || {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| f(&reply))) {
                    let detail = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("internal error");
                    reply.send(Msg::Failed(format!("{label} stopped: {detail}")));
                }
            });
        if let Err(e) = spawned {
            self.reply.send(Msg::Failed(format!("cannot start {name}: {e}")));
        }
    }

    pub fn drain(&self) -> Vec<Msg> {
        self.rx.try_iter().collect()
    }
}

/// Decodes PNG or other image bytes for an egui texture.
pub fn color_image(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(rgba_to_color(&img))
}

pub fn rgba_to_color(img: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}
