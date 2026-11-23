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

/// Favorite route specs in stored order. Missing or corrupt data reads as
/// empty rather than failing the picker.
pub fn load(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    #[derive(serde::Deserialize)]
    struct Stored {
        #[serde(default)]
        favorites: Vec<String>,
    }
    serde_json::from_str::<Stored>(&content)
        .map(|stored| stored.favorites)
        .unwrap_or_default()
}

/// Write `specs` through a temporary file so a crash mid-write cannot leave
/// a truncated store behind.
pub fn save(path: &Path, specs: &[String]) -> io::Result<()> {
    #[derive(serde::Serialize)]
    struct Stored<'a> {
        favorites: &'a [String],
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&Stored { favorites: specs })?;
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
        assert!(load(&path).is_empty());
        std::fs::write(&path, "not json {{{").unwrap();
        assert!(load(&path).is_empty());
        std::fs::write(&path, r#"{"favorites": "nope"}"#).unwrap();
        assert!(load(&path).is_empty());
    }

    #[test]
    fn save_load_roundtrip_and_toggle_keeps_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/favorites.json");
        let specs = vec![
            "magpie:codex/gpt-6.2-sol".to_string(),
            "openai-api:atlas-1".to_string(),
        ];
        save(&path, &specs).unwrap();
        assert_eq!(load(&path), specs);

        let mut list = load(&path);
        assert!(!toggle(&mut list, "magpie:codex/gpt-6.2-sol"));
        assert_eq!(list, ["openai-api:atlas-1"]);
        assert!(toggle(&mut list, "claude-api:fable-5"));
        assert_eq!(list, ["openai-api:atlas-1", "claude-api:fable-5"]);
        assert!(toggle(&mut list, "magpie:codex/gpt-6.2-sol"));
        assert_eq!(
            list,
            [
                "openai-api:atlas-1",
                "claude-api:fable-5",
                "magpie:codex/gpt-6.2-sol"
            ]
        );
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
    }
}
