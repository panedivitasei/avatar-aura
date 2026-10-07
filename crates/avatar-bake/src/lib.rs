//! Avatar bake pipeline, ported from avatar_export.cpp: resolves a saved avatar manifest against the asset pack
//! and closet, bakes materials, face layers, animations and previews, and writes avatar.json.
#![forbid(unsafe_code)]

pub mod animations;
pub mod args;
pub mod bake;
pub mod cli;
pub mod glmath;
pub mod icons;
pub mod json;
pub mod manifest;
pub mod msvc_sort;
pub mod preview;
pub mod resolve;
pub mod skin;
pub mod textures;
pub mod util;
