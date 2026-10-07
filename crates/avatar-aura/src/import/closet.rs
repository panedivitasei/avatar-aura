// Port of avatar_aura/closet.py: `<guid>.bin` items, `icons/<guid>.png`, closet_index.tsv and closet_awards.tsv.
// Appends keep existing rows byte for byte and back the index up before changing it.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use super::validate::ItemResult;

pub fn index_line(res: &ItemResult) -> String {
    format!(
        "{}\t{:08X}\t{}\t{}",
        res.guid, res.categories, res.bodies, res.name
    )
}

fn filetime_now() -> u64 {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    ((unix + 11_644_473_600.0) * 10_000_000.0) as u64
}

pub fn award_line(res: &ItemResult) -> String {
    format!(
        "{}\t{:08X}\t{}\t{}\t{}",
        res.guid,
        res.title_id,
        res.title_name,
        res.description,
        filetime_now()
    )
}

fn first_fields(data: &[u8]) -> HashSet<Vec<u8>> {
    data.split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
        .map(|l| l.split(|b| *b == b'\t').next().unwrap_or(&[]).to_vec())
        .collect()
}

fn append_rows(mut data: Vec<u8>, rows: &[String]) -> Vec<u8> {
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.extend_from_slice(b"\r\n");
    }
    for row in rows {
        data.extend_from_slice(row.as_bytes());
        data.extend_from_slice(b"\r\n");
    }
    data
}

/// Appends a closet_awards.tsv row per new award item; returns the action line and the files written.
pub fn update_awards(results: &[ItemResult], closet: &Path) -> io::Result<(Option<String>, Vec<PathBuf>)> {
    let awards: Vec<&ItemResult> = results.iter().filter(|r| r.is_award).collect();
    if awards.is_empty() {
        return Ok((None, Vec::new()));
    }
    let path = closet.join("closet_awards.tsv");
    let data = std::fs::read(&path).unwrap_or_default();
    let listed = first_fields(&data);
    let mut seen = HashSet::new();
    let rows: Vec<String> = awards
        .iter()
        .filter(|r| !listed.contains(r.guid.as_bytes()) && seen.insert(r.guid.clone()))
        .map(|r| award_line(r))
        .collect();
    if rows.is_empty() {
        return Ok((
            Some("award details already in closet_awards.tsv".into()),
            Vec::new(),
        ));
    }
    std::fs::write(&path, append_rows(data, &rows))?;
    Ok((
        Some(format!(
            "recorded {} award detail line(s) in closet_awards.tsv",
            rows.len()
        )),
        vec![path],
    ))
}

fn tsv_rows(path: &Path) -> Vec<Vec<String>> {
    std::fs::read(path)
        .map(|data| {
            data.split(|b| *b == b'\n')
                .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
                .map(|l| {
                    let text = String::from_utf8_lossy(l);
                    text.trim_start_matches('\u{feff}')
                        .split('\t')
                        .map(str::to_string)
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// (title id, game name) for every game with awards in the closet, sorted by name.
pub fn list_award_titles(closet: &Path) -> Vec<(String, String)> {
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for f in tsv_rows(&closet.join("closet_titles.tsv")) {
        if f.len() >= 2 && !f[0].is_empty() {
            names.entry(f[0].to_uppercase()).or_insert_with(|| f[1].clone());
        }
    }
    let mut titles: BTreeMap<String, String> = BTreeMap::new();
    for f in tsv_rows(&closet.join("closet_awards.tsv")) {
        if f.len() >= 2 && !f[1].is_empty() {
            let tid = f[1].to_uppercase();
            let name = f
                .get(2)
                .filter(|n| !n.is_empty())
                .cloned()
                .or_else(|| names.get(&tid).cloned())
                .unwrap_or_default();
            titles.entry(tid).or_insert(name);
        }
    }
    let mut out: Vec<(String, String)> = titles.into_iter().collect();
    out.sort_by(|a, b| (a.1.to_lowercase(), &a.0).cmp(&(b.1.to_lowercase(), &b.0)));
    out
}

pub fn write_item_files(res: &ItemResult, closet: &Path) -> io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(closet)?;
    let bin = closet.join(format!("{}.bin", res.guid));
    std::fs::write(&bin, &res.bin_bytes)?;
    let mut written = vec![bin];
    if let Some(icon) = &res.icon_bytes {
        let dir = closet.join("icons");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.png", res.guid));
        std::fs::write(&path, icon)?;
        written.push(path);
    }
    Ok(written)
}

/// Appends new items to closet_index.tsv (backup first) or creates the index.
pub fn update_index(results: &[ItemResult], closet: &Path) -> io::Result<(String, Vec<PathBuf>)> {
    std::fs::create_dir_all(closet)?;
    let path = closet.join("closet_index.tsv");
    if path.exists() {
        let data = std::fs::read(&path)?;
        let indexed = first_fields(&data);
        let rows: Vec<String> = results
            .iter()
            .filter(|r| !indexed.contains(r.guid.as_bytes()))
            .map(index_line)
            .collect();
        if rows.is_empty() {
            return Ok(("already in closet_index.tsv, index unchanged".into(), Vec::new()));
        }
        let mut backup = path.clone().into_os_string();
        backup.push(".bak");
        std::fs::write(PathBuf::from(backup), &data)?;
        std::fs::write(&path, append_rows(data, &rows))?;
        return Ok((
            format!(
                "appended {} line(s) to existing closet_index.tsv (backup: .bak)",
                rows.len()
            ),
            vec![path],
        ));
    }
    let mut seen = HashSet::new();
    let rows: Vec<String> = results
        .iter()
        .filter(|r| seen.insert(r.guid.clone()))
        .map(index_line)
        .collect();
    std::fs::write(&path, append_rows(Vec::new(), &rows))?;
    Ok((
        format!("created new closet_index.tsv ({} line(s))", rows.len()),
        vec![path],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(guid: &str) -> ItemResult {
        ItemResult {
            guid: guid.into(),
            categories: 8,
            name: "Shirt".into(),
            bin_bytes: vec![1, 2, 3],
            ..ItemResult::default()
        }
    }

    #[test]
    fn index_appends_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("aura-closet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = item("00000008-0000-0000-0000-00000000000a");
        let (action, _) = update_index(std::slice::from_ref(&a), &dir).unwrap();
        assert!(action.starts_with("created"));
        let (action, _) =
            update_index(&[a.clone(), item("00000008-0000-0000-0000-00000000000b")], &dir).unwrap();
        assert!(action.starts_with("appended 1"));
        let text = std::fs::read_to_string(dir.join("closet_index.tsv")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\t00000008\t3\tShirt\r\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
