// Port of avatar_aura/face_animation.py: a clip's timed face tracks as a sequence of mixed expression entries.

use std::collections::{BTreeSet, HashMap};

use crate::avatar::Avatar;
use crate::error::{invalid, Result};
use crate::faces::{mixed_catalog_entry, Expression, MixedEntry, Selection};

/// Track name and the composite channel it selects from.
pub const CHANNELS: [(&str, &str); 5] = [
    ("mouth", "mouth"),
    ("eye_left", "eyes"),
    ("eye_right", "eyes"),
    ("brow_left", "brows"),
    ("brow_right", "brows"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct FaceAnimation {
    pub id: usize,
    pub name: String,
    pub fps: f64,
    pub frames: u32,
    /// Entry index per clip frame.
    pub sequence: Vec<usize>,
    pub entries: Vec<MixedEntry>,
}

/// Mixes one entry per distinct face state of clip `index`; frames without a composite fall back to 0.
pub fn prepare(avatar: &Avatar, index: usize) -> Result<FaceAnimation> {
    let clip = avatar
        .animations
        .get(index)
        .ok_or_else(|| invalid("Choose a face animation."))?;
    let available: BTreeSet<(&str, u32)> = avatar
        .face
        .composite_files
        .iter()
        .map(|e| (e.channel.as_str(), e.frame))
        .collect();
    let empty: Vec<u32> = Vec::new();
    let track = |name: &str| -> &Vec<u32> {
        match &clip.face {
            Some(f) => match name {
                "mouth" => &f.mouth,
                "eye_left" => &f.eye_left,
                "eye_right" => &f.eye_right,
                "brow_left" => &f.brow_left,
                "brow_right" => &f.brow_right,
                _ => &empty,
            },
            None => &empty,
        }
    };
    let mut sequence = Vec::with_capacity(clip.frame_count as usize);
    let mut entries = Vec::new();
    let mut lookup: HashMap<[u32; 5], usize> = HashMap::new();
    for frame in 0..clip.frame_count as usize {
        let mut key = [0u32; 5];
        let mut selection = Selection::new();
        for (slot, (channel, source)) in CHANNELS.iter().enumerate() {
            let value = track(channel).get(frame).copied().unwrap_or(0);
            let value = if available.contains(&(*source, value)) {
                value
            } else {
                0
            };
            key[slot] = value;
            selection.insert((*channel).to_string(), Some(value));
        }
        let entry = match lookup.get(&key) {
            Some(&i) => i,
            None => {
                entries.push(mixed_catalog_entry(avatar, &Expression::Mix(selection))?);
                lookup.insert(key, entries.len() - 1);
                entries.len() - 1
            }
        };
        sequence.push(entry);
    }
    Ok(FaceAnimation {
        id: index,
        name: clip.name.clone(),
        fps: clip.fps,
        frames: clip.frame_count,
        sequence,
        entries,
    })
}
