// Programmatic front of the avatar-bake `--avatar` mode: builds `Args` the way `cli::run_from_args` does and
// runs the bake in process, plus a clip-only path that skips the texture bake.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use avatar_bake::args::Args;
use avatar_bake::manifest::Metadata;
use avatar_bake::resolve::{load_closet, load_pack, resolve, Assets};
use avatar_bake::skin::build_joint_xforms;
use avatar_bake::{animations, bake};
use avatar_export::scene;

/// Saved avatar, asset pack and closet: the inputs every bake shares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs {
    pub manifest: PathBuf,
    pub pack: PathBuf,
    pub closet: PathBuf,
}

impl Inputs {
    pub fn check(&self) -> anyhow::Result<()> {
        if self.manifest.as_os_str().is_empty() || !self.manifest.is_file() {
            bail!("pick the saved avatar (avatar_manifest.bin) first");
        }
        if self.pack.as_os_str().is_empty() || !self.pack.is_file() {
            bail!("AvatarAssetPack.toc not found, pick it");
        }
        Ok(())
    }

    /// `run_from_args` defaults: the legacy pack beside the main one, the closet when it is a folder.
    fn args(&self) -> Args {
        let mut a = Args {
            manifest: self.manifest.clone(),
            toc: self.pack.clone(),
            ..Args::default()
        };
        let legacy = self
            .pack
            .parent()
            .unwrap_or(Path::new(""))
            .join("AvatarAssetPackLegacyV1.toc");
        if legacy.is_file() {
            a.legacy_toc = legacy;
        }
        if self.closet.is_dir() {
            a.closet = self.closet.clone();
        }
        a
    }
}

/// One `--avatar` run; unset fields keep the command-line defaults.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub inputs: Inputs,
    pub out_dir: PathBuf,
    pub bake_size: Option<i32>,
    pub face_frames: bool,
    pub no_scale: bool,
    pub pack_anims: bool,
    pub pack_anim_filters: Vec<String>,
    pub anims: Vec<PathBuf>,
    pub anim_dir: PathBuf,
    pub preview_dir: PathBuf,
}

impl Request {
    pub fn args(&self) -> Args {
        let mut a = self.inputs.args();
        a.out_dir = self.out_dir.clone();
        if let Some(size) = self.bake_size {
            a.bake_size = size.max(64);
        }
        a.face_frames = self.face_frames;
        a.face_frame_size = 512;
        a.no_scale = self.no_scale;
        a.pack_anims = self.pack_anims;
        a.pack_anim_filters = self.pack_anim_filters.clone();
        a.anims = self.anims.clone();
        a.anim_dir = self.anim_dir.clone();
        a.preview_dir = self.preview_dir.clone();
        a
    }
}

fn exit_reason(code: i32) -> &'static str {
    match code {
        1 => "the manifest is not a 1000-byte saved avatar",
        2 => "the asset pack could not be loaded",
        3 => "the skeleton could not be resolved",
        4 => "avatar.json could not be written",
        5 => "the icon render failed",
        _ => "unknown failure",
    }
}

fn guarded<T>(what: &str, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let detail = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("internal error");
            Err(anyhow!("{what} stopped: {detail}"))
        }
    }
}

/// Runs the bake and returns the path of the `avatar.json` it wrote.
pub fn run(request: &Request) -> anyhow::Result<PathBuf> {
    request.inputs.check()?;
    std::fs::create_dir_all(&request.out_dir)
        .with_context(|| format!("creating {}", request.out_dir.display()))?;
    let args = request.args();
    let code = guarded("bake", || bake::run_export(&args))?;
    if code != 0 {
        bail!("avatar bake failed (exit {code}): {}", exit_reason(code));
    }
    let json = request.out_dir.join("avatar.json");
    if !json.is_file() {
        bail!("bake produced no avatar.json");
    }
    Ok(json)
}

pub fn read_scene(json: &Path) -> anyhow::Result<scene::Scene> {
    let text = std::fs::read_to_string(json).with_context(|| format!("reading {}", json.display()))?;
    scene::Scene::from_json(&text).with_context(|| format!("parsing {}", json.display()))
}

/// Which clips a clip-only bake collects.
#[derive(Clone, Debug, Default)]
pub struct ClipQuery {
    pub pack_anims: bool,
    pub pack_anim_filters: Vec<String>,
    pub anims: Vec<PathBuf>,
    pub anim_dir: PathBuf,
}

/// The animation stage of `run_export` alone: resolve the skeleton, collect clips, convert their tracks.
pub fn bake_clips(inputs: &Inputs, query: &ClipQuery) -> anyhow::Result<Vec<scene::Animation>> {
    inputs.check()?;
    let mut args = inputs.args();
    args.pack_anims = query.pack_anims;
    args.pack_anim_filters = query.pack_anim_filters.clone();
    args.anims = query.anims.clone();
    args.anim_dir = query.anim_dir.clone();
    guarded("clip bake", move || {
        let bytes =
            std::fs::read(&args.manifest).with_context(|| format!("reading {}", args.manifest.display()))?;
        let metadata = Metadata::parse(&bytes).map_err(|_| anyhow!("{}", exit_reason(1)))?;
        let pack = load_pack(&args.toc).ok_or_else(|| anyhow!("{}", exit_reason(2)))?;
        let legacy_pack = if args.legacy_toc.as_os_str().is_empty() {
            None
        } else {
            load_pack(&args.legacy_toc)
        };
        let closet = if args.closet.as_os_str().is_empty() {
            avatar_formats::Closet::default()
        } else {
            load_closet(&args.closet)
        };
        let assets = Assets {
            pack,
            legacy_pack,
            closet,
        };
        let res = resolve(&metadata, &args, &assets).ok_or_else(|| anyhow!("{}", exit_reason(3)))?;
        let xforms = build_joint_xforms(&res.skeleton, !args.no_scale);
        Ok(animations::collect(&args, &assets, &res)
            .iter()
            .map(|a| animations::to_scene(a, &xforms))
            .collect())
    })
}
