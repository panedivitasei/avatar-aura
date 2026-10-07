//! Ports Args, PrintUsage and the option parsing of RunFromArgs from avatar_export.cpp (`--avatar` / `--avatar-info`).

use std::path::PathBuf;

use avatar_formats::AssetId;

use crate::util::atoi;

#[derive(Clone, Debug)]
pub struct Args {
    pub manifest: PathBuf,
    pub out_dir: PathBuf,
    /// AvatarAssetPack.toc.
    pub toc: PathBuf,
    /// AvatarAssetPackLegacyV1.toc, used to rescue old-body items.
    pub legacy_toc: PathBuf,
    pub closet: PathBuf,
    pub skeleton_version: i32,
    pub no_scale: bool,
    pub want_prop: bool,
    /// `--avatar-info`: summary JSON on stdout, no bake.
    pub info_only: bool,
    /// Head composite resolution.
    pub bake_size: i32,
    /// Whole-head composites per feature frame.
    pub face_frames: bool,
    pub face_frame_size: i32,
    /// Parsed for compatibility; the bake does not read it.
    pub eye_whites: bool,
    pub anims: Vec<PathBuf>,
    pub anim_dir: PathBuf,
    pub pack_anims: bool,
    /// `--pack-anim <substring>[@<frame>]`, repeatable.
    pub pack_anim_filters: Vec<String>,
    pub list_anims: bool,
    pub preview_dir: PathBuf,
    pub preview_size: i32,
    /// `mid`, `peak`, a fraction in 0..1 or a frame number.
    pub preview_frame: String,
    /// `--gen-icons` worn-item mode: when set, the run renders `icon_target` into this PNG and exports nothing.
    pub icon_out: PathBuf,
    pub icon_target: AssetId,
    pub icon_categories: u32,
    pub icon_size: i32,
    pub icon_yaw: f32,
    pub icon_pitch: f32,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            manifest: PathBuf::new(),
            out_dir: PathBuf::new(),
            toc: PathBuf::new(),
            legacy_toc: PathBuf::new(),
            closet: PathBuf::new(),
            skeleton_version: 2,
            no_scale: false,
            want_prop: true,
            info_only: false,
            bake_size: 1024,
            face_frames: false,
            face_frame_size: 512,
            eye_whites: false,
            anims: Vec::new(),
            anim_dir: PathBuf::new(),
            pack_anims: false,
            pack_anim_filters: Vec::new(),
            list_anims: false,
            preview_dir: PathBuf::new(),
            preview_size: 384,
            preview_frame: "mid".into(),
            icon_out: PathBuf::new(),
            icon_target: AssetId::default(),
            icon_categories: 0,
            icon_size: 128,
            icon_yaw: 25.0,
            icon_pitch: 10.0,
        }
    }
}

pub const USAGE: &str = "avatarextract --avatar <avatar_manifest.bin> <out_dir> [options]
  --toc <AvatarAssetPack.toc>          (default: next to the manifest's
                                        userdata: ..\\avatarpack\\AvatarAssetPack.toc)
  --legacy-toc <AvatarAssetPackLegacyV1.toc>
  --closet <dir>                       (default: <pack_dir>\\closet)
  --skeleton-version 1|2               (default 2 = Kinect-era titles)
  --no-scale                           ignore height/weight factors
  --no-prop                            skip the carryable
  --bake-size N                        head composite size (default 1024)
  --face-frames [N]                    whole-head composite per feature frame
  --eye-whites                         paint a sclera mask into eye art lacking one
  --anim <file.AvatarAnimation>        export this animation (repeatable)
  --anim-dir <dir>                     export every *.AvatarAnimation in dir
  --pack-anims                         export every animation asset in the pack
  --pack-anim <name>[@<frame>]         export pack animations whose name contains
                                       <name> (case-insensitive, repeatable); @<frame>
                                       overrides --preview-frame for that clip
  --list-anims                         list pack animations and exit
  --preview-dir <dir>                  render preview_T-Pose.png + preview_<clip>.png
  --preview-size N                     preview image size (default 384)
  --preview-frame mid|peak|<0..1>|<N>  which clip frame to pose (default mid)
  --avatar-info                        print an avatar summary JSON, no export
";

pub fn print_usage() {
    print!("{USAGE}");
}

/// What the option scan asks the caller to do.
#[derive(Debug)]
pub enum Parsed {
    /// Run with these arguments and positionals.
    Run(Box<Args>, Vec<String>),
    /// `--help` / `-h`: print usage and exit 0.
    Help,
}

/// The option loop of RunFromArgs; unknown words become positionals.
pub fn parse(argv: &[String]) -> Parsed {
    let mut args = Args::default();
    let mut positionals = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        let has_next = i + 1 < argv.len();
        let next_path = |i: &mut usize, dst: &mut PathBuf| {
            if *i + 1 < argv.len() {
                *i += 1;
                *dst = PathBuf::from(&argv[*i]);
            }
        };
        match a {
            "--toc" => next_path(&mut i, &mut args.toc),
            "--legacy-toc" => next_path(&mut i, &mut args.legacy_toc),
            "--closet" => next_path(&mut i, &mut args.closet),
            "--skeleton-version" if has_next => {
                i += 1;
                args.skeleton_version = atoi(&argv[i]);
            }
            "--no-scale" => args.no_scale = true,
            "--no-prop" => args.want_prop = false,
            "--avatar-info" => args.info_only = true,
            "--bake-size" if has_next => {
                i += 1;
                args.bake_size = atoi(&argv[i]).max(64);
            }
            "--face-frames" => {
                args.face_frames = true;
                if has_next && argv[i + 1].as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    i += 1;
                    args.face_frame_size = atoi(&argv[i]).max(64);
                }
            }
            "--eye-whites" => args.eye_whites = true,
            "--anim" if has_next => {
                i += 1;
                args.anims.push(PathBuf::from(&argv[i]));
            }
            "--anim-dir" => next_path(&mut i, &mut args.anim_dir),
            "--pack-anims" => args.pack_anims = true,
            "--pack-anim" if has_next => {
                i += 1;
                args.pack_anim_filters.push(argv[i].clone());
            }
            "--preview-dir" => next_path(&mut i, &mut args.preview_dir),
            "--preview-size" if has_next => {
                i += 1;
                args.preview_size = atoi(&argv[i]).max(64);
            }
            "--preview-frame" if has_next => {
                i += 1;
                args.preview_frame = argv[i].clone();
            }
            "--list-anims" => args.list_anims = true,
            "--help" | "-h" => return Parsed::Help,
            _ => positionals.push(argv[i].clone()),
        }
        i += 1;
    }
    Parsed::Run(Box::new(args), positionals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_golden_command() {
        let argv = strings(&[
            "m.bin",
            "out",
            "--toc",
            "p.toc",
            "--pack-anims",
            "--face-frames",
            "512",
            "--preview-frame",
            "0.35",
            "--preview-size",
            "192",
            "--bake-size",
            "8",
        ]);
        let Parsed::Run(a, pos) = parse(&argv) else {
            panic!("help")
        };
        assert_eq!(pos, ["m.bin", "out"]);
        assert_eq!(a.toc, PathBuf::from("p.toc"));
        assert!(a.pack_anims && a.face_frames);
        assert_eq!(a.face_frame_size, 512);
        assert_eq!(a.preview_frame, "0.35");
        assert_eq!(a.preview_size, 192);
        assert_eq!(a.bake_size, 64);
    }

    #[test]
    fn trailing_value_option_is_a_positional() {
        let Parsed::Run(_, pos) = parse(&strings(&["--bake-size"])) else {
            panic!("help")
        };
        assert_eq!(pos, ["--bake-size"]);
        assert!(matches!(parse(&strings(&["x", "-h"])), Parsed::Help));
    }
}
