//! Minimal comment-preserving edits to `~/.jcode/config.toml`, following the
//! writer Desktop already uses for its own settings: replace only the value
//! span of one key, or insert the key, leaving every other line untouched.
use std::collections::BTreeMap;

/// Set `key = value` in `[section]`, where `value` is a TOML literal.
pub fn set_value(text: &str, section: &str, key: &str, value: &str) -> anyhow::Result<String> {
    let header = format!("[{section}]");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let start = lines.iter().position(|line| {
        line.split('#')
            .next()
            .is_some_and(|value| value.trim() == header)
    });
    if let Some(start) = start {
        let end = lines[start + 1..]
            .iter()
            .position(|line| line.trim_start().starts_with('['))
            .map_or(lines.len(), |offset| start + 1 + offset);
        if let Some(index) = (start + 1..end).find(|&index| {
            lines[index]
                .split_once('=')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
        }) {
            let mut tail = lines[index..end].join("\n");
            let values: BTreeMap<String, toml::Spanned<toml::Value>> = toml::from_str(&tail)?;
            let span = values
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("could not locate `{key}` in [{section}]"))?
                .span();
            tail.replace_range(span, value);
            lines.splice(index..end, tail.lines().map(str::to_owned));
        } else {
            lines.insert(start + 1, format!("{key} = {value}"));
        }
    } else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(format!("{key} = {value}"));
    }
    let mut result = lines.join("\n");
    result.push('\n');
    // Never write a file jcode could no longer parse.
    result.parse::<toml::Table>()?;
    Ok(result)
}

/// A string or array-of-strings hook value as a list.
pub fn string_list(value: Option<&toml::Value>) -> Vec<String> {
    match value {
        Some(toml::Value::String(text)) if !text.trim().is_empty() => vec![text.clone()],
        Some(toml::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn list_literal(items: &[String]) -> String {
    toml::Value::Array(items.iter().cloned().map(toml::Value::String).collect()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_one_key_and_keeps_comments() {
        let text = "# mine\n[hooks]\npre_tool_timeout_ms = 5000\nturn_end = \"notify\" # keep\n\n[display]\ntheme = \"x\"\n";
        let edited = set_value(text, "hooks", "turn_end", &list_literal(&["notify".into(), "b".into()])).unwrap();
        assert!(edited.contains("# mine"));
        assert!(edited.contains("turn_end = [\"notify\", \"b\"] # keep"));
        assert!(edited.contains("theme = \"x\""));
        let inserted = set_value(&edited, "hooks", "pre_tool", "[\"gate\"]").unwrap();
        assert!(inserted.contains("[hooks]\npre_tool = [\"gate\"]\n"));
        let appended = set_value("[a]\nb = 1", "hooks", "turn_start", "[]").unwrap();
        assert!(appended.ends_with("[hooks]\nturn_start = []\n"));
    }
}
