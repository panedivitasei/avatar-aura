// Fills crates/avatar-aura/assets with the game-derived files the repo does not carry, from an AvatarEditorRecomp tree.

mod json;
mod png;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context};
use avatar_bake::args::Args;
use avatar_export::faces::{catalog, sha256_hex};
use avatar_export::scene::{FaceFile, Scene};
use avatar_export::Avatar;
use json::{dump, obj, Json};

const USAGE: &str = "\
usage: cargo run -p build-assets -- <AvatarEditorRecomp checkout or install> [options]

Writes into the assets folder:
  clips/       Cheer, Salute and Idle (the recomp's Look) .AvatarAnimation, plus provenance.json
  mannequins/  mannequin_male.amd and mannequin_female.amd, plus provenance.json
  faces/       face layer and head composite PNGs from a --face-frames 512 bake, plus face_index.json
  catalog.json expression and clip names; clips are the three above, then every pack animation

Options:
  --out <dir>          assets folder to fill (default: crates/avatar-aura/assets)
  --pack <toc>         AvatarAssetPack.toc (default: <recomp>/assets/AvatarAssetPack.toc)
  --closet <dir>       closet folder (default: <recomp>/assets/closet)
  --mannequins <dir>   folder holding the two .amd files (default: the DryCleaner build output,
                       out/apps/DryCleaner, apps/DryCleaner, DryCleaner or tools/dry_cleaner)
  --manifest <file>    saved avatar to bake the faces and catalog from, used as is

Without --manifest the bake runs on mannequin_male.amd with the face features and colours of the
avatar the shipped faces were made from: mouth Grin, eyes Excited, brows Large, no face paint, and
that avatar's nine colours. Those two inputs alone reproduce the shipped art pixel for pixel; the
stock mannequin (Open, Almond, Curvy, Jaw Stubble, its own colours) bakes different faces.
";

/// Mouth, eyes, brows: the texture slots 0-2 of the reference avatar.
const REFERENCE_FEATURES: [[u8; 16]; 3] = [
    guid(0x0000_8000, 0x02eb),
    guid(0x0000_2000, 0x02a0),
    guid(0x0000_4000, 0x0263),
];

/// Skin, hair, mouth, iris, eyebrow, eye shadow, facial hair, skin features 1 and 2.
const REFERENCE_COLORS: [u32; 9] = [
    0xFFD7AA71, 0xFF6E5326, 0xFFB56157, 0xFF6381A7, 0xFF493421, 0xFF000000, 0xFF493421, 0xFFCF5969,
    0xFFCF5969,
];

const fn guid(a: u32, b: u16) -> [u8; 16] {
    let a = a.to_be_bytes();
    let b = b.to_be_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], 0x00, 0x03, 0xC1, 0xC8, 0xF1, 0x09, 0xA1, 0x9C, 0xB2, 0xE0,
    ]
}

/// Bundled clip name and the recomp file it is copied from.
const CLIPS: [(&str, &str); 3] = [
    ("Cheer.AvatarAnimation", "Cheer.AvatarAnimation"),
    ("Idle.AvatarAnimation", "Look.AvatarAnimation"),
    ("Salute.AvatarAnimation", "Salute.AvatarAnimation"),
];

const MANNEQUINS: [&str; 2] = ["mannequin_female.amd", "mannequin_male.amd"];

const MANNEQUIN_DIRS: [&str; 4] = [
    "out/apps/DryCleaner",
    "apps/DryCleaner",
    "DryCleaner",
    "tools/dry_cleaner",
];

const THUMBNAIL_RENDERER: &str =
    "avatarextract --preview-frame 0.35 for clips; three.js head portrait for face composites";

#[derive(Default)]
struct Opts {
    recomp: PathBuf,
    out: PathBuf,
    pack: Option<PathBuf>,
    closet: Option<PathBuf>,
    mannequins: Option<PathBuf>,
    manifest: Option<PathBuf>,
}

enum Parsed {
    Run(Opts),
    Help,
}

fn parse(argv: &[String]) -> anyhow::Result<Parsed> {
    let mut o = Opts {
        out: join(Path::new(env!("CARGO_MANIFEST_DIR")), "../avatar-aura/assets"),
        ..Opts::default()
    };
    let mut recomp = None;
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        let mut value = || {
            it.next()
                .map(PathBuf::from)
                .ok_or_else(|| anyhow!("{a} needs a value"))
        };
        match a.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--out" => o.out = value()?,
            "--pack" => o.pack = Some(value()?),
            "--closet" => o.closet = Some(value()?),
            "--mannequins" => o.mannequins = Some(value()?),
            "--manifest" => o.manifest = Some(value()?),
            s if s.starts_with("--") => bail!("unknown option {s}"),
            s if recomp.is_none() => recomp = Some(PathBuf::from(s)),
            s => bail!("unexpected argument {s}"),
        }
    }
    o.recomp = recomp.ok_or_else(|| anyhow!("no AvatarEditorRecomp path given"))?;
    Ok(Parsed::Run(o))
}

/// Every input, located up front so one run reports all that is missing.
struct Inputs {
    assets: PathBuf,
    mannequins: PathBuf,
    pack: PathBuf,
    closet: PathBuf,
    manifest: Option<PathBuf>,
}

fn locate(o: &Opts) -> anyhow::Result<Inputs> {
    if !o.recomp.is_dir() {
        bail!("{} is not a folder", o.recomp.display());
    }
    let mut missing = Vec::new();
    // A checkout keeps the game files in assets/; an install may carry them at its root.
    let assets = [o.recomp.join("assets"), o.recomp.clone()]
        .into_iter()
        .find(|d| d.join(CLIPS[1].1).is_file())
        .unwrap_or_else(|| o.recomp.join("assets"));
    for (_, src) in CLIPS {
        let p = assets.join(src);
        if !p.is_file() {
            missing.push(format!("clip {}", p.display()));
        }
    }
    let has_mannequins = |d: &Path| MANNEQUINS.iter().all(|m| d.join(m).is_file());
    let mannequins = match &o.mannequins {
        Some(d) => d.clone(),
        None => MANNEQUIN_DIRS
            .iter()
            .map(|d| join(&o.recomp, d))
            .find(|d| has_mannequins(d))
            .unwrap_or_else(|| join(&o.recomp, MANNEQUIN_DIRS[0])),
    };
    if !has_mannequins(&mannequins) {
        missing.push(format!(
            "mannequin_male.amd and mannequin_female.amd in {} (build the DryCleaner app or pass --mannequins)",
            mannequins.display()
        ));
    }
    let pack = o
        .pack
        .clone()
        .unwrap_or_else(|| assets.join("AvatarAssetPack.toc"));
    if !pack.is_file() {
        missing.push(format!("asset pack {} (--pack)", pack.display()));
    }
    let closet = o.closet.clone().unwrap_or_else(|| assets.join("closet"));
    if !closet.is_dir() {
        missing.push(format!("closet folder {} (--closet)", closet.display()));
    }
    if let Some(m) = &o.manifest {
        if !m.is_file() {
            missing.push(format!("manifest {}", m.display()));
        }
    }
    if !missing.is_empty() {
        bail!("missing inputs:\n  {}", missing.join("\n  "));
    }
    Ok(Inputs {
        assets,
        mannequins,
        pack,
        closet,
        manifest: o.manifest.clone(),
    })
}

fn join(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, c| match c {
        ".." => p.parent().map(Path::to_path_buf).unwrap_or(p),
        _ => p.join(c),
    })
}

fn read(path: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// `<recomp folder name>/<relative path>` when `path` lies under the recomp, else the path as given.
fn source_label(recomp: &Path, path: &Path) -> String {
    let name = std::fs::canonicalize(recomp)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    match (name, path.strip_prefix(recomp)) {
        (Some(name), Ok(rel)) => format!("{name}/{}", rel.to_string_lossy().replace('\\', "/")),
        _ => path.display().to_string(),
    }
}

/// Copies `(dest name, source path, provenance source)` entries and writes their provenance.json.
fn copy_set(dir: &Path, files: &[(&str, PathBuf, String)], wrote: &mut Vec<String>) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut prov = Vec::new();
    for (name, src, label) in files {
        let bytes = read(src)?;
        write(&dir.join(name), &bytes)?;
        wrote.push(format!(
            "{}  ({} bytes, from {})",
            dir.join(name).display(),
            bytes.len(),
            src.display()
        ));
        prov.push((
            name.to_string(),
            obj([
                ("source", label.as_str().into()),
                ("sha256", sha256_hex(&bytes).into()),
            ]),
        ));
    }
    write(
        &dir.join("provenance.json"),
        dump(&Json::Obj(prov), true).as_bytes(),
    )?;
    wrote.push(dir.join("provenance.json").display().to_string());
    Ok(())
}

/// The male mannequin with the reference avatar's face features and colours.
fn reference_manifest(male: &[u8]) -> anyhow::Result<Vec<u8>> {
    if male.len() != 1000 {
        bail!(
            "mannequin_male.amd is {} bytes, not a 1000-byte saved avatar",
            male.len()
        );
    }
    let mut m = male.to_vec();
    for (slot, id) in REFERENCE_FEATURES.iter().enumerate() {
        let o = 0x3C + slot * 32;
        m[o..o + 16].copy_from_slice(id);
    }
    m[0x3C + 3 * 32..0x3C + 3 * 32 + 16].fill(0);
    for (i, c) in REFERENCE_COLORS.iter().enumerate() {
        m[0xFC + i * 4..0x100 + i * 4].copy_from_slice(&c.to_be_bytes());
    }
    Ok(m)
}

fn bake(inputs: &Inputs, manifest: &Path, anim_dir: &Path, work: &Path) -> anyhow::Result<PathBuf> {
    let mut args = Args {
        manifest: manifest.to_path_buf(),
        out_dir: work.to_path_buf(),
        toc: inputs.pack.clone(),
        closet: inputs.closet.clone(),
        face_frames: true,
        face_frame_size: 512,
        anim_dir: anim_dir.to_path_buf(),
        pack_anims: true,
        ..Args::default()
    };
    let legacy = inputs.pack.with_file_name("AvatarAssetPackLegacyV1.toc");
    if legacy.is_file() {
        args.legacy_toc = legacy;
    }
    let code = avatar_bake::bake::run_export(&args)?;
    if code != 0 {
        bail!("avatar bake failed with exit code {code}");
    }
    let json = work.join("avatar.json");
    if !json.is_file() {
        bail!("the bake wrote no avatar.json");
    }
    Ok(json)
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// SanitizeName: ASCII letters, digits, `_` and `-` survive, spaces become `_`.
fn safe_name(value: &str) -> String {
    let out: String = value
        .chars()
        .filter_map(|c| match c {
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => Some(c),
            ' ' => Some('_'),
            _ => None,
        })
        .collect();
    if out.is_empty() {
        "item".into()
    } else {
        out
    }
}

/// The avatar.json `face` block in the bake's key order, file names cut to base names.
fn face_index(scene: &Scene) -> Json {
    let face = scene.face.clone().unwrap_or_default();
    let files = |list: &[FaceFile]| {
        Json::Arr(
            list.iter()
                .map(|f| {
                    obj([
                        ("channel", f.channel.as_str().into()),
                        ("frame", Json::Int(i64::from(f.frame))),
                        ("file", file_name(&f.file).into()),
                    ])
                })
                .collect(),
        )
    };
    let slots = ["mouth", "eyes", "brows", "face_paint", "eye_shadow", "face"]
        .iter()
        .filter_map(|name| {
            face.slots.get(*name).map(|s| {
                (
                    name.to_string(),
                    obj([
                        ("guid", s.guid.as_str().into()),
                        ("name", s.name.as_str().into()),
                        ("layers", Json::Int(i64::from(s.layers))),
                        ("width", Json::Int(i64::from(s.width))),
                        ("height", Json::Int(i64::from(s.height))),
                    ]),
                )
            })
        })
        .collect();
    let names = ["eyes", "mouth", "brows"]
        .iter()
        .map(|name| {
            let list = face.layer_names.get(*name).cloned().unwrap_or_default();
            (
                name.to_string(),
                Json::Arr(list.into_iter().map(Json::Str).collect()),
            )
        })
        .collect();
    obj([
        (
            "head_materials",
            Json::Arr(face.head_materials.iter().map(|m| m.as_str().into()).collect()),
        ),
        ("slots", Json::Obj(slots)),
        ("layer_files", files(&face.layer_files)),
        ("composite_files", files(&face.composite_files)),
        ("layer_names", Json::Obj(names)),
    ])
}

/// Re-encodes every face PNG of the bake with the stb encoder and replaces the faces folder's PNGs.
fn write_faces(bake_face: &Path, dir: &Path, scene: &Scene, wrote: &mut Vec<String>) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.ends_with(".png") || name == "face_index.json" {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(bake_face)
        .with_context(|| format!("the bake wrote no face folder at {}", bake_face.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        bail!("the bake wrote no face PNGs");
    }
    for src in &pngs {
        let img = image::open(src)
            .with_context(|| format!("decoding {}", src.display()))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let bytes = png::encode_rgba(img.as_raw(), w as usize, h as usize);
        write(&dir.join(src.file_name().unwrap_or_default()), &bytes)?;
    }
    wrote.push(format!("{}  ({} PNGs)", dir.display(), pngs.len()));
    let index = dir.join("face_index.json");
    write(&index, dump(&face_index(scene), false).as_bytes())?;
    wrote.push(index.display().to_string());
    Ok(())
}

fn write_catalog(
    path: &Path,
    avatar_json: &Path,
    manifest: &[u8],
    pack: &Path,
    wrote: &mut Vec<String>,
) -> anyhow::Result<()> {
    let avatar = Avatar::load(avatar_json).map_err(|e| anyhow!("loading {}: {e}", avatar_json.display()))?;
    let expressions: Vec<Json> = catalog(&avatar)
        .map_err(|e| anyhow!("building the expression list: {e}"))?
        .into_iter()
        .map(|e| {
            let thumb = format!("{}.png", safe_name(&e.id));
            obj([
                ("id", e.id.into()),
                ("name", e.name.into()),
                ("thumb", thumb.into()),
            ])
        })
        .collect();
    let counts = (expressions.len(), avatar.animations.len());
    let clips = avatar
        .animations
        .iter()
        .enumerate()
        .map(|(i, c)| {
            obj([
                ("id", Json::Int(i as i64)),
                ("name", c.name.as_str().into()),
                ("fps", Json::Float(c.fps)),
                ("frames", Json::Int(i64::from(c.frame_count))),
                ("thumb", format!("clip_{}.png", safe_name(&c.name)).into()),
            ])
        })
        .collect();
    let doc = obj([
        ("expressions", Json::Arr(expressions)),
        ("clips", Json::Arr(clips)),
        ("manifest_sha256", sha256_hex(manifest).into()),
        ("thumbnail_renderer", THUMBNAIL_RENDERER.into()),
        ("pack_sha256", sha256_hex(&read(pack)?).into()),
    ]);
    write(path, dump(&doc, true).as_bytes())?;
    wrote.push(format!(
        "{}  ({} expressions, {} clips)",
        path.display(),
        counts.0,
        counts.1
    ));
    Ok(())
}

fn run(o: &Opts) -> anyhow::Result<Vec<String>> {
    let inputs = locate(o)?;
    let mut wrote = Vec::new();
    let clips_dir = o.out.join("clips");
    let clips: Vec<_> = CLIPS
        .iter()
        .map(|(name, src)| (*name, inputs.assets.join(src), src.to_string()))
        .collect();
    copy_set(&clips_dir, &clips, &mut wrote)?;
    let mannequins: Vec<_> = MANNEQUINS
        .iter()
        .map(|name| {
            let src = inputs.mannequins.join(name);
            let label = source_label(&o.recomp, &src);
            (*name, src, label)
        })
        .collect();
    copy_set(&o.out.join("mannequins"), &mannequins, &mut wrote)?;

    let manifest = match &inputs.manifest {
        Some(m) => read(m)?,
        None => reference_manifest(&read(&inputs.mannequins.join("mannequin_male.amd"))?)?,
    };
    let work = std::env::temp_dir().join(format!("avatar-aura-build-assets-{}", std::process::id()));
    std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    let result = (|| {
        let manifest_path = work.join("avatar_manifest.bin");
        write(&manifest_path, &manifest)?;
        let bake_dir = work.join("bake");
        let avatar_json = bake(&inputs, &manifest_path, &clips_dir, &bake_dir)?;
        let scene =
            Scene::load(&avatar_json).map_err(|e| anyhow!("reading {}: {e}", avatar_json.display()))?;
        write_faces(&bake_dir.join("face"), &o.out.join("faces"), &scene, &mut wrote)?;
        write_catalog(
            &o.out.join("catalog.json"),
            &avatar_json,
            &manifest,
            &inputs.pack,
            &mut wrote,
        )
    })();
    // A leftover work folder only costs temp space.
    let _ = std::fs::remove_dir_all(&work);
    result?;
    Ok(wrote)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse(&argv) {
        Ok(Parsed::Run(o)) => o,
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e:#}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&opts) {
        Ok(wrote) => {
            println!("\nwrote:");
            for line in wrote {
                println!("  {line}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
