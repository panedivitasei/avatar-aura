// `--smoke`: headless start-up check. Loads the bundled assets, bakes or reuses the default avatar when one is
// configured, renders one frame offscreen and reports; no window is created.

use anyhow::{anyhow, bail, Context};

use crate::catalog::Catalog;
use crate::paths;
use crate::session::{self, TextureCache};
use crate::settings::Settings;

const SIZE: u32 = 256;

pub fn run() -> anyhow::Result<()> {
    let catalog = Catalog::load()?;
    for mannequin in ["mannequin_male.amd", "mannequin_female.amd"] {
        let path = paths::asset(&["mannequins", mannequin]);
        if !path.is_file() {
            bail!("{} is missing", path.display());
        }
    }
    println!(
        "assets: {} ({} expressions, {} clips)",
        paths::assets_dir().display(),
        catalog.expressions.len(),
        catalog.clips.len()
    );
    let (gpu, adapter) = crate::gpu::headless()?;
    println!("adapter: {adapter}");

    let mut settings = Settings::default();
    settings.fill_defaults();
    let inputs = crate::bake::Inputs {
        manifest: paths::expand(&settings.manifest),
        pack: paths::expand(&settings.pack),
        closet: paths::expand(&settings.closet),
    };
    let mut viewer = avatar_view::Viewer::new(&gpu.device, &gpu.queue, crate::viewport::FORMAT, true);
    let background = viewer
        .render_to_rgba(&gpu.device, &gpu.queue, SIZE, SIZE)
        .map_err(|e| anyhow!("empty render: {e}"))?;
    if inputs.check().is_err() {
        println!("default avatar: not configured, rendered the empty viewport");
        return Ok(());
    }
    let loaded = session::load(&inputs, std::path::Path::new(""), &catalog, &mut |line| {
        println!("{line}");
    })?;
    viewer = gpu.build_viewer(&loaded.scene, &loaded.dir)?;
    let neutral = session::mix_expression(
        &loaded,
        &session::selection(None, None, None),
        &TextureCache::default(),
    )?;
    for (index, image) in &neutral {
        viewer.set_material_texture(*index, image)?;
    }
    let frame = viewer
        .render_to_rgba(&gpu.device, &gpu.queue, SIZE, SIZE)
        .map_err(|e| anyhow!("render: {e}"))?;
    let shot = paths::cache_dir().join("smoke.png");
    frame
        .save(&shot)
        .with_context(|| format!("writing {}", shot.display()))?;
    println!("frame: {}", shot.display());
    let changed = frame
        .pixels()
        .zip(background.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(x, y)| x.abs_diff(*y) > 6))
        .count();
    let coverage = changed as f64 / f64::from(SIZE * SIZE);
    println!(
        "rendered {SIZE}x{SIZE}: {:.1}% avatar pixels, {} joints, {} expressions",
        coverage * 100.0,
        viewer.joint_names().len(),
        loaded.expressions.len()
    );
    if coverage < 0.02 {
        bail!("the avatar did not appear in the offscreen frame");
    }
    Ok(())
}
