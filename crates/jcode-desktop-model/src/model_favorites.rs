//! Favorite model routes pinned at the top of the model pickers.
//!
//! Keys are route specs (`magpie:codex/gpt-6.2-sol`), the strings the pickers
//! submit as `/model <spec>`. Specs for routes that disappear from the
//! catalog stay in the file and are simply skipped while unavailable.

use std::io;
use std::path::{Path, PathBuf};

/// `~/.jcode/desktop-model-favorites.json`.
pub fn path() -> Option<PathBuf> {
    Some(
        crate::platform_dirs::jcode_home()?
            .join("desktop-model-favorites.json"),
    )
}

/// The picker's stored lists: favorite route specs and hidden
/// provider/channel group keys (`provider|api_method|channel`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lists {
    pub favorites: Vec<String>,
    pub hidden_groups: Vec<String>,
}

/// Stored lists in stored order. Missing or corrupt data reads as empty
/// rather than failing the picker; files written before `hidden_groups`
/// existed still load.
pub fn load(path: &Path) -> Lists {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Lists::default();
    };
    #[derive(serde::Deserialize)]
    struct Stored {
        #[serde(default)]
        favorites: Vec<String>,
        #[serde(default)]
        hidden_groups: Vec<String>,
    }
    serde_json::from_str::<Stored>(&content)
        .map(|stored| Lists {
            favorites: stored.favorites,
            hidden_groups: stored.hidden_groups,
        })
        .unwrap_or_default()
}

/// Write both lists through a temporary file so a crash mid-write cannot
/// leave a truncated store behind.
pub fn save(path: &Path, lists: &Lists) -> io::Result<()> {
    #[derive(serde::Serialize)]
    struct Stored<'a> {
        favorites: &'a [String],
        hidden_groups: &'a [String],
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&Stored {
        favorites: &lists.favorites,
        hidden_groups: &lists.hidden_groups,
    })?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Toggle `spec` and return its new favorited state. Existing entries keep
/// their order; new favorites append.
pub fn toggle(list: &mut Vec<String>, spec: &str) -> bool {
    if let Some(index) = list.iter().position(|entry| entry == spec) {
        list.remove(index);
        false
    } else {
        list.push(spec.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_corrupt_files_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("favorites.json");
        assert_eq!(load(&path), Lists::default());
        std::fs::write(&path, "not json {{{").unwrap();
        assert_eq!(load(&path), Lists::default());
        std::fs::write(&path, r#"{"favorites": "nope"}"#).unwrap();
        assert_eq!(load(&path), Lists::default());
    }

    #[test]
    fn files_without_hidden_groups_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("favorites.json");
        std::fs::write(&path, r#"{"favorites": ["magpie:codex/gpt-6.2-sol"]}"#).unwrap();
        let lists = load(&path);
        assert_eq!(lists.favorites, ["magpie:codex/gpt-6.2-sol"]);
        assert!(lists.hidden_groups.is_empty());
    }

    #[test]
    fn save_load_roundtrip_and_toggle_keeps_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/favorites.json");
        let mut lists = Lists {
            favorites: vec![
                "magpie:codex/gpt-6.2-sol".to_string(),
                "openai-api:atlas-1".to_string(),
            ],
            hidden_groups: vec!["magpie|openai-compatible:magpie|group".to_string()],
        };
        save(&path, &lists).unwrap();
        assert_eq!(load(&path), lists);

        assert!(!toggle(&mut lists.favorites, "magpie:codex/gpt-6.2-sol"));
        assert_eq!(lists.favorites, ["openai-api:atlas-1"]);
        assert!(toggle(&mut lists.favorites, "claude-api:fable-5"));
        assert_eq!(
            lists.favorites,
            ["openai-api:atlas-1", "claude-api:fable-5"]
        );
        assert!(toggle(&mut lists.favorites, "magpie:codex/gpt-6.2-sol"));
        assert_eq!(
            lists.favorites,
            [
                "openai-api:atlas-1",
                "claude-api:fable-5",
                "magpie:codex/gpt-6.2-sol"
            ]
        );
        assert!(toggle(&mut lists.hidden_groups, "other|method|"));
        save(&path, &lists).unwrap();
        assert_eq!(load(&path), lists);
    }
}
