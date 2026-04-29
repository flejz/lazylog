//! Named filter presets persisted as JSON files.
//!
//! Presets live in `~/.cache/lazylog/presets/<name>.json`. Each file holds
//! a single `FilterPreset { name, filter }` document so users can save the
//! current `FilterState` and reload it later.

use serde::{Deserialize, Serialize};

use crate::filter::FilterState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterPreset {
    pub name: String,
    pub filter: FilterState,
}

fn presets_dir() -> Option<std::path::PathBuf> {
    Some(dirs::cache_dir()?.join("lazylog").join("presets"))
}

/// Persist the given filter under `name`. Creates the cache dir if needed.
pub fn save_preset(name: &str, filter: &FilterState) -> anyhow::Result<()> {
    let dir = presets_dir().ok_or_else(|| anyhow::anyhow!("no cache dir"))?;
    std::fs::create_dir_all(&dir)?;
    let preset = FilterPreset { name: name.to_string(), filter: filter.clone() };
    let json = serde_json::to_string_pretty(&preset)?;
    std::fs::write(dir.join(format!("{}.json", sanitize_name(name))), json)?;
    Ok(())
}

/// Read every saved preset. Returns an empty vec if the dir does not exist.
pub fn list_presets() -> Vec<FilterPreset> {
    let Some(dir) = presets_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut presets: Vec<FilterPreset> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                return None;
            }
            let json = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str(&json).ok()
        })
        .collect();
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    presets
}

/// Replace path-unsafe chars so the preset name can be a filename.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}
