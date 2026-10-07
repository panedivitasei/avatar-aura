// Port of avatar_aura/import_service.py and import_preview.py: validation results, closet installs and the
// mannequin preview bake.

pub mod closet;
pub mod manifest;
pub mod validate;

use std::path::{Path, PathBuf};

use validate::{bodies_label, category_names, ItemResult, PackageError};

/// What the item list shows per validated item.
#[derive(Clone, Debug)]
pub struct ItemSummary {
    pub guid: String,
    pub name: String,
    pub categories: String,
    pub bodies: &'static str,
    pub award: bool,
    pub icon: Option<Vec<u8>>,
}

pub fn describe(results: &[ItemResult]) -> Vec<ItemSummary> {
    results
        .iter()
        .map(|r| ItemSummary {
            guid: r.guid.clone(),
            name: r.name.clone(),
            categories: category_names(r.categories),
            bodies: bodies_label(r.bodies),
            award: r.is_award,
            icon: r.icon_bytes.clone(),
        })
        .collect()
}

/// Outcome of a validate, import or title-icon run.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub ok: bool,
    pub log: Vec<String>,
    pub items: Vec<ItemSummary>,
    pub written: Vec<PathBuf>,
    pub error: String,
    pub results: Vec<ItemResult>,
}

pub fn read_results(paths: &[PathBuf]) -> Result<Vec<ItemResult>, PackageError> {
    if paths.is_empty() {
        return Err(PackageError("Pick an input file first.".into()));
    }
    let mut results = Vec::new();
    for path in paths {
        if path.as_os_str().is_empty() || !path.is_file() {
            return Err(PackageError("Pick an input file first.".into()));
        }
        results.extend(validate::process_package(path)?);
    }
    Ok(results)
}

fn result_log(results: &[ItemResult]) -> Vec<String> {
    results
        .iter()
        .flat_map(|r| {
            r.log
                .iter()
                .cloned()
                .chain(r.warnings.iter().map(|w| format!("note: {w}")))
        })
        .collect()
}

/// Validates, and with `install` writes items, index, award rows and title icons into the closet.
pub fn perform(paths: &[PathBuf], closet: &Path, icon: &Path, install: bool) -> Outcome {
    let mut log = Vec::new();
    let run = |log: &mut Vec<String>| -> Result<Outcome, PackageError> {
        if install && closet.as_os_str().is_empty() {
            return Err(PackageError("Pick a closet folder first.".into()));
        }
        let results = read_results(paths)?;
        log.extend(result_log(&results));
        let items = describe(&results);
        if !install {
            log.push("preview only, nothing was written.".into());
            return Ok(Outcome {
                ok: true,
                items,
                results,
                ..Outcome::default()
            });
        }
        let io = |e: std::io::Error| PackageError(e.to_string());
        let icon_bytes = validate::require_game_icon(&results, icon)?;
        let mut written = Vec::new();
        for r in &results {
            written.extend(closet::write_item_files(r, closet).map_err(io)?);
        }
        let (index_action, index_files) = closet::update_index(&results, closet).map_err(io)?;
        let (award_action, award_files) = closet::update_awards(&results, closet).map_err(io)?;
        written.extend(index_files);
        written.extend(award_files);
        if icon_bytes.is_some() {
            let mut titles: Vec<String> = results
                .iter()
                .filter(|r| r.is_award)
                .map(|r| format!("{:08X}", r.title_id))
                .collect();
            titles.sort();
            titles.dedup();
            for title in titles {
                written.push(validate::install_title_icon(closet, &title, icon)?);
            }
        }
        log.extend(written.iter().map(|f| format!("wrote {}", f.display())));
        log.push(format!("index: {index_action}"));
        if let Some(action) = award_action {
            log.push(format!("awards: {action}"));
        }
        if !icon.as_os_str().is_empty() && icon_bytes.is_none() {
            log.push("note: icon ignored, this container holds no avatar awards".into());
        }
        Ok(Outcome {
            ok: true,
            items,
            written,
            results,
            ..Outcome::default()
        })
    };
    match run(&mut log) {
        Ok(mut outcome) => {
            outcome.log = log;
            outcome
        }
        Err(e) => {
            log.push(format!("REFUSED: {e}"));
            Outcome {
                ok: false,
                log,
                error: e.to_string(),
                ..Outcome::default()
            }
        }
    }
}

pub fn title_icon(closet: &Path, title: &str, icon: &Path) -> Outcome {
    let run = || -> Result<PathBuf, PackageError> {
        if closet.as_os_str().is_empty() || title.is_empty() {
            return Err(PackageError(
                "Pick the closet folder and an installed game first.".into(),
            ));
        }
        if icon.as_os_str().is_empty() {
            return Err(PackageError("Set the game icon path first.".into()));
        }
        validate::install_title_icon(closet, title, icon)
    };
    match run() {
        Ok(path) => Outcome {
            ok: true,
            log: vec![format!("wrote {}", path.display())],
            written: vec![path],
            ..Outcome::default()
        },
        Err(e) => Outcome {
            ok: false,
            log: vec![format!("REFUSED: {e}")],
            error: e.to_string(),
            ..Outcome::default()
        },
    }
}

/// Bakes one validated item worn on the bundled mannequin into `dir` and returns its avatar.json.
pub fn bake_mannequin_preview(item: &ItemResult, pack: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let closet_dir = dir.join("closet");
    std::fs::create_dir_all(&closet_dir)?;
    closet::write_item_files(item, &closet_dir)?;
    closet::update_index(std::slice::from_ref(item), &closet_dir)?;
    let mannequin = if item.bodies == 2 {
        "mannequin_female.amd"
    } else {
        "mannequin_male.amd"
    };
    let base_path = crate::paths::asset(&["mannequins", mannequin]);
    let base = match std::fs::read(&base_path) {
        Ok(b) => b,
        Err(_) => crate::paths::bundled_bytes(&format!("mannequins/{mannequin}"))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| anyhow::anyhow!("{} is missing", base_path.display()))?,
    };
    let manifest = dir.join("mannequin.amd");
    std::fs::write(
        &manifest,
        manifest::wear_on_manifest(&base, &item.guid, item.categories)?,
    )?;
    let mut request = crate::bake::Request {
        inputs: crate::bake::Inputs {
            manifest,
            pack: pack.to_path_buf(),
            closet: closet_dir.clone(),
        },
        out_dir: dir.to_path_buf(),
        bake_size: Some(512),
        no_scale: true,
        pack_anim_filters: vec!["Animation Generic Stand 0@0.5".into()],
        ..crate::bake::Request::default()
    };
    if item.categories & validate::ANIMATION_CATEGORY != 0 {
        request.anims.push(closet_dir.join(format!("{}.bin", item.guid)));
    }
    crate::bake::run(&request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_a_closet_item_on_the_mannequin() {
        let item = Path::new(r"C:\Users\edward\Documents\ReXGlue\ae-sub\assets\closet")
            .join("00000008-0004-c172-caeb-f9c44d5308c9.bin");
        let pack = crate::paths::expand(&crate::paths::default_pack());
        if !item.is_file() || !pack.is_file() {
            return;
        }
        let outcome = perform(&[item], Path::new(""), Path::new(""), false);
        assert!(outcome.ok, "{:?}", outcome.log);
        let dir = std::env::temp_dir().join(format!("aura-preview-{}", std::process::id()));
        let json = bake_mannequin_preview(&outcome.results[0], &pack, &dir).unwrap();
        let scene = crate::bake::read_scene(&json).unwrap();
        assert!(scene.components.iter().any(|c| c.guid == outcome.results[0].guid));
        assert_eq!(scene.animations.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
