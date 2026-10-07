// Port of the shared helpers of avatarextract/main.cpp; each module ports one command-line mode.
//! Library side of the avatarextract tool: every mode is a `pub` function taking plain arguments and
//! a log sink that receives the text the C++ tool printed to stdout.

#![forbid(unsafe_code)]

/// `printf` stand-in for a log sink; write errors on the log are ignored like the C++ ignores them.
macro_rules! say {
    ($log:expr, $($arg:tt)*) => {{
        let _ = write!($log, $($arg)*);
    }};
}

pub mod blob;
pub mod closet_import;
pub mod objwrite;
pub mod outfits;
pub mod pack;
pub mod package;
pub mod refit;

/// `%.4f` with the space flag: non-negative values get a leading blank.
pub(crate) fn space_f4(v: f32) -> String {
    if v.is_sign_negative() {
        format!("{v:.4}")
    } else {
        format!(" {v:.4}")
    }
}
