// Port of avatar_aura/errors.py and the ValueError/ExportError raises across the export modules.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("avatar.json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    /// Malformed scene data: short arrays, bad indices, parents after children.
    #[error("bad scene: {0}")]
    Scene(String),
    /// Rejected caller input; the text matches the Python ValueError messages.
    #[error("{0}")]
    Invalid(String),
    /// Export pipeline failure; the text matches the Python ExportError messages.
    #[error("{0}")]
    Export(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}

pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

pub(crate) fn bad_scene(msg: impl Into<String>) -> Error {
    Error::Scene(msg.into())
}
