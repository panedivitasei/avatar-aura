// Error type shared by every decoder in the crate.

use crate::strb::BlockId;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bit stream overrun: need {needed} bits at {offset}, have {size}")]
    Overrun {
        offset: usize,
        needed: usize,
        size: usize,
    },
    #[error("read of {requested} bits does not fit a {width}-bit value")]
    ReadWidth { requested: usize, width: usize },
    #[error("malformed data: {0}")]
    Malformed(&'static str),
    #[error("lzx: {0}")]
    Lzx(&'static str),
    #[error("not an STRB container")]
    NotStrb,
    #[error("STRB block {0:?} not found")]
    BlockNotFound(BlockId),
    #[error("unknown asset pack version")]
    UnknownPackVersion,
    #[error("invalid asset id text")]
    AssetIdText,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Mirrors the C++ `assert_true`: a false condition is malformed input.
pub(crate) fn ensure(cond: bool, what: &'static str) -> Result<()> {
    if cond {
        Ok(())
    } else {
        Err(Error::Malformed(what))
    }
}
