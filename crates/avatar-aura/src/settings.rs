// Persisted UI state: last paths, export formats and window size, as TOML in the user config folder.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FormatFlags {
    pub glb: bool,
    pub obj: bool,
    pub dae: bool,
    pub smd: bool,
}

impl Default for FormatFlags {
    fn default() -> Self {
        Self {
            glb: true,
            obj: false,
            dae: false,
            smd: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub manifest: String,
    pub pack: String,
    pub closet: String,
    pub anim_dir: String,
    pub output: String,
    pub import_closet: String,
    pub icon_path: String,
    pub formats: FormatFlags,
    pub window: Option<[f32; 2]>,
    /// `import` or `export`.
    pub tab: String,
}

impl Settings {
    pub fn path() -> PathBuf {
        paths::config_dir().join("settings.toml")
    }

    /// Stored settings with empty paths filled from the userdata defaults.
    pub fn load() -> Self {
        let mut s: Settings = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| toml::from_str(t.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        s.fill_defaults();
        s
    }

    pub fn fill_defaults(&mut self) {
        if self.manifest.is_empty() {
            self.manifest = paths::default_manifest();
        }
        if self.pack.is_empty() {
            self.pack = paths::default_pack();
        }
        if self.closet.is_empty() {
            self.closet = paths::default_closet();
        }
        if self.import_closet.is_empty() {
            self.import_closet = paths::default_closet();
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let s = Settings {
            output: "C:/out".into(),
            window: Some([1280.0, 850.0]),
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
        assert!(back.formats.glb && !back.formats.obj);
    }
}
