// Default locations from avatar_aura/paths.py: userdata, asset roots, closet, config and cache folders.

use std::path::{Path, PathBuf};

use include_dir::{include_dir, Dir};

static BUNDLED: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets");

/// Trims blanks and surrounding quotes, and expands a leading `~`.
pub fn expand(value: &str) -> PathBuf {
    let trimmed = value.trim().trim_matches('"');
    if let Some(rest) = trimmed.strip_prefix('~') {
        if let Some(home) = home_dir() {
            return home.join(rest.trim_start_matches(['/', std::path::MAIN_SEPARATOR]));
        }
    }
    PathBuf::from(trimmed)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// `%USERPROFILE%\Documents\ReXGlue`, the shared workspace of the editor tools.
fn workspace() -> PathBuf {
    home_dir().unwrap_or_default().join("Documents").join("ReXGlue")
}

pub fn userdata() -> PathBuf {
    if let Some(configured) = std::env::var_os("AVATAR_AURA_USERDATA") {
        return expand(&configured.to_string_lossy());
    }
    let shared = jmstudios_dir().join("userdata");
    [
        exe_dir().join("userdata"),
        shared.clone(),
        workspace().join("userdata"),
    ]
    .into_iter()
    .find(|c| c.is_dir())
    .unwrap_or(shared)
}

fn asset_roots() -> Vec<PathBuf> {
    vec![
        userdata().join("avatarpack"),
        assets_dir(),
        exe_dir(),
        workspace().join("ae-sub").join("assets"),
    ]
}

/// `Documents/JMstudios`, the user-data tree every JMstudios recomp reads.
pub fn jmstudios_dir() -> PathBuf {
    home_dir().unwrap_or_default().join("Documents").join("JMstudios")
}

/// `Documents/JMstudios/userdata/avatar`, where the Avatar Editor recomp keeps the saved avatar.
pub fn shared_avatar_dir() -> PathBuf {
    jmstudios_dir().join("userdata").join("avatar")
}

pub fn default_manifest() -> String {
    [
        shared_avatar_dir().join("avatar_manifest.bin"),
        userdata().join("avatars").join("avatar_manifest.bin"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .map(|p| p.display().to_string())
    .unwrap_or_default()
}

pub fn default_pack() -> String {
    asset_roots()
        .into_iter()
        .map(|r| r.join("AvatarAssetPack.toc"))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

pub fn default_closet() -> String {
    asset_roots()
        .into_iter()
        .map(|r| r.join("closet"))
        .find(|p| p.is_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn project() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("org", "ReXGlue", "Avatar Aura")
}

pub fn config_dir() -> PathBuf {
    project()
        .map(|p| p.config_dir().to_path_buf())
        .unwrap_or_else(|| exe_dir().join("config"))
}

pub fn cache_dir() -> PathBuf {
    project()
        .map(|p| p.cache_dir().to_path_buf())
        .unwrap_or_else(|| exe_dir().join("cache"))
}

/// The bundled `assets` folder: beside the executable when shipped that way, else the embedded copy unpacked into the cache.
pub fn assets_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let beside = exe_dir().join("assets");
        if beside.join("catalog.json").is_file() {
            return beside;
        }
        let target = cache_dir().join(format!("assets-{}", env!("CARGO_PKG_VERSION")));
        // A failed unpack leaves the folder partial; asset lookups then report the missing file.
        let _ = unpack(&BUNDLED, &target);
        target
    })
    .clone()
}

fn unpack(dir: &Dir<'_>, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for file in dir.files() {
        let path = target.join(file.path());
        let fresh = std::fs::metadata(&path).is_ok_and(|m| m.len() == file.contents().len() as u64);
        if !fresh {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, file.contents())?;
        }
    }
    for sub in dir.dirs() {
        unpack(sub, target)?;
    }
    Ok(())
}

/// Embedded bytes of a bundled file, for data the UI needs before any disk access.
pub fn bundled_bytes(relative: &str) -> Option<&'static [u8]> {
    BUNDLED.get_file(relative).map(|f| f.contents())
}

pub fn asset(parts: &[&str]) -> PathBuf {
    parts.iter().fold(assets_dir(), |p, part| p.join(part))
}

/// SanitizeName's byte classification: letters, digits, `_` and `-` survive, blanks become `_`.
pub fn safe_name(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_names_match_python() {
        assert_eq!(
            safe_name("Animation Generic Stand 1"),
            "Animation_Generic_Stand_1"
        );
        assert_eq!(safe_name("mouth:3"), "mouth3");
        assert_eq!(safe_name("::"), "item");
    }

    #[test]
    fn bundled_catalog_is_embedded() {
        assert!(bundled_bytes("catalog.json").is_some());
        assert!(bundled_bytes("mannequins/mannequin_male.amd").is_some());
    }
}
