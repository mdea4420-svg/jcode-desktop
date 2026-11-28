//! `@file` and `@folder` workspace path completion for `PromptInput`.

use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(15);
const MAX_SCANNED_ENTRIES: usize = 3000;
const MAX_RESULTS: usize = 12;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    "trash",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MentionEntry {
    pub rel_path: String,
    pub is_dir: bool,
}

struct CachedEntries {
    root: String,
    scanned_at: Instant,
    entries: Vec<MentionEntry>,
}

#[derive(Default)]
pub(crate) struct AtMentionState {
    pub selection: usize,
    pub dismissed_token: Option<(Range<usize>, String)>,
    pub scroll: gpui::ScrollHandle,
    cache: RefCell<Option<CachedEntries>>,
}

impl AtMentionState {
    pub(crate) fn entries_for(&self, working_dir: Option<&str>) -> Vec<MentionEntry> {
        let Some(dir) = working_dir.map(str::trim).filter(|d| !d.is_empty()) else {
            return Vec::new();
        };
        let now = Instant::now();
        if let Some(cached) = self.cache.borrow().as_ref()
            && cached.root == dir
            && now.saturating_duration_since(cached.scanned_at) < CACHE_TTL
        {
            return cached.entries.clone();
        }
        let entries = scan_workspace(Path::new(dir));
        *self.cache.borrow_mut() = Some(CachedEntries {
            root: dir.to_owned(),
            scanned_at: now,
            entries: entries.clone(),
        });
        entries
    }

    pub(crate) fn invalidate(&self) {
        *self.cache.borrow_mut() = None;
    }
}

fn scan_workspace(root: &Path) -> Vec<MentionEntry> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((root.to_path_buf(), 0usize));
    while let Some((dir, depth)) = queue.pop_front() {
        if out.len() >= MAX_SCANNED_ENTRIES || depth > 5 {
            break;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = read.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            if out.len() >= MAX_SCANNED_ENTRIES {
                break;
            }
            let name = child.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            if SKIP_DIRS.contains(&name_str) || (name_str.starts_with('.') && name_str != ".github" && name_str != ".jcode" && name_str != ".devin") {
                continue;
            }
            let path = child.path();
            let Ok(ft) = child.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel_path = rel.to_string_lossy().replace('\\', "/");
            let is_dir = ft.is_dir();
            out.push(MentionEntry {
                rel_path,
                is_dir,
            });
            if is_dir {
                queue.push_back((path, depth + 1));
            }
        }
    }
    out
}

/// Extract `@query` ending at `cursor` when `@` is at byte 0 or preceded by whitespace.
pub(crate) fn active_at_token(content: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let cursor = cursor.min(content.len());
    if !content.is_char_boundary(cursor) {
        return None;
    }
    // Do not trigger inside slash commands at the start of the line when typing command names.
    let prefix = &content[..cursor];
    let token_start = prefix
        .rfind(char::is_whitespace)
        .map_or(0, |idx| idx + prefix[idx..].chars().next().map_or(1, char::len_utf8));
    let token = &content[token_start..cursor];
    let query = token.strip_prefix('@')?;
    if query.contains(char::is_whitespace) {
        return None;
    }
    Some((token_start..cursor, query))
}

fn fuzzy_score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        let depth = candidate.matches('/').count() as i32;
        return Some(100 - depth * 10);
    }
    let cand_lower = candidate.to_lowercase();
    let file_name = cand_lower.rsplit('/').next().unwrap_or(&cand_lower);
    let depth_penalty = candidate.matches('/').count() as i32 * 3;
    if file_name == query {
        return Some(1000 - depth_penalty);
    }
    if file_name.starts_with(query) {
        return Some(800 - (file_name.len() as i32) - depth_penalty);
    }
    if cand_lower.starts_with(query) {
        return Some(700 - depth_penalty);
    }
    if file_name.contains(query) {
        return Some(600 - (file_name.len() as i32) - depth_penalty);
    }
    if cand_lower.contains(query) {
        return Some(500 - depth_penalty);
    }
    // Subsequence match
    let mut chars = cand_lower.chars();
    for q in query.chars() {
        chars.find(|&c| c == q)?;
    }
    Some(250 - (cand_lower.len() as i32) - depth_penalty)
}

pub(crate) fn match_entries(entries: &[MentionEntry], query: &str) -> Vec<MentionEntry> {
    let query = query.trim_end_matches('/').to_lowercase();
    let mut scored: Vec<(i32, &MentionEntry)> = entries
        .iter()
        .filter_map(|entry| fuzzy_score(&entry.rel_path, &query).map(|score| (score, entry)))
        .collect();
    scored.sort_by(|(sa, ea), (sb, eb)| {
        sb.cmp(sa)
            .then_with(|| ea.rel_path.len().cmp(&eb.rel_path.len()))
            .then_with(|| ea.rel_path.cmp(&eb.rel_path))
    });
    scored
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, entry)| entry.clone())
        .collect()
}

pub(crate) fn replacement_text(entry: &MentionEntry) -> String {
    let raw = if entry.is_dir {
        format!("{}/", entry.rel_path)
    } else {
        entry.rel_path.clone()
    };
    if raw.chars().any(char::is_whitespace) {
        format!("\"{raw}\" ")
    } else if entry.is_dir {
        format!("@{raw}")
    } else {
        format!("{raw} ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_at_token_detects_standalone_and_mid_sentence_mentions() {
        assert_eq!(active_at_token("@src/main", 9), Some((0..9, "src/main")));
        assert_eq!(
            active_at_token("请检查 @panel_ext", "请检查 @panel_ext".len()),
            Some(("请检查 ".len().."请检查 @panel_ext".len(), "panel_ext"))
        );
        assert_eq!(active_at_token("user@example.com", 16), None);
        assert_eq!(active_at_token("@done ", 6), None);
    }

    #[test]
    fn fuzzy_matcher_ranks_filename_prefix_above_deep_subsequence() {
        let entries = vec![
            MentionEntry {
                rel_path: "crates/deep/module/panel_extensions.rs".into(),
                is_dir: false,
            },
            MentionEntry {
                rel_path: "src/panel.rs".into(),
                is_dir: false,
            },
            MentionEntry {
                rel_path: "src".into(),
                is_dir: true,
            },
        ];
        let hits = match_entries(&entries, "panel_ext");
        assert_eq!(hits[0].rel_path, "crates/deep/module/panel_extensions.rs");
        assert_eq!(replacement_text(&hits[0]), "crates/deep/module/panel_extensions.rs ");
        assert_eq!(replacement_text(&entries[2]), "@src/");
    }
}
