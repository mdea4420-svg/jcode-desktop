//! Named OpenAI-compatible third-party API profiles (`[providers.<name>]` in
//! `~/.jcode/config.toml`), with private key storage and `/models` discovery.
//!
//! Secrets are written through `jcode_base::provider_catalog::save_named_api_key`
//! into Jcode's private per-provider `.env` file and never returned to UI state
//! or written in plaintext to `config.toml`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jcode_base::config::{
    Config, NamedProviderAuth, NamedProviderConfig, NamedProviderModelConfig, NamedProviderType,
};
use jcode_base::provider_catalog::{
    api_base_uses_localhost, is_safe_env_file_name, is_safe_env_key_name,
    load_api_key_from_env_or_config, normalize_api_base, resolve_login_provider,
    save_named_api_key,
};

const RESERVED_NAMES: &[&str] = &[
    "auto",
    "claude-subprocess",
    "compat",
    "custom",
    "azure-openai",
    "aoai",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomProviderSummary {
    pub name: String,
    pub base_url: String,
    pub default_model: Option<String>,
    pub models: Vec<String>,
    pub model_catalog: bool,
    pub has_api_key: bool,
    pub is_default: bool,
}

impl CustomProviderSummary {
    pub fn active_model(&self) -> Option<&str> {
        self.default_model
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| self.models.first().map(String::as_str))
    }

    pub fn route_spec(&self) -> Option<String> {
        self.active_model()
            .map(|model| format!("{}:{model}", self.name))
    }
}

#[derive(Clone, Debug, Default)]
pub struct CustomProviderDraft {
    /// Set when editing an existing profile so renames clean up the old section
    /// and blank API-key edits preserve the stored credential.
    pub original_name: Option<String>,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub default_model: String,
    pub models: Vec<String>,
    pub set_default: bool,
    pub model_catalog: bool,
}

pub fn config_path() -> Result<PathBuf, String> {
    Config::path().ok_or_else(|| "Could not determine ~/.jcode/config.toml path.".to_string())
}

/// Offline preview data for screenshots and UI unit tests.
pub fn offline_summaries() -> Vec<CustomProviderSummary> {
    vec![CustomProviderSummary {
        name: "local-gateway".into(),
        base_url: "http://127.0.0.1:3425/v1".into(),
        default_model: Some("codex/gpt-6.2-sol".into()),
        models: vec![
            "codex/gpt-6.2-sol".into(),
            "cpr-fast/gpt-6.2-sol".into(),
            "workbuddy/deepseek-v4.1-flash".into(),
        ],
        model_catalog: true,
        has_api_key: true,
        is_default: false,
    }]
}

pub fn list() -> Vec<CustomProviderSummary> {
    let Ok(path) = config_path() else {
        return Vec::new();
    };
    list_at(&path, &|env_key, env_file| {
        load_api_key_from_env_or_config(env_key, env_file).is_some()
    })
}

pub fn list_at(
    path: &Path,
    has_stored_key: &dyn Fn(&str, &str) -> bool,
) -> Vec<CustomProviderSummary> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(config) = parse_config_or_default(&content) else {
        return Vec::new();
    };
    let default_provider = config.provider.default_provider.as_deref();
    let mut out: Vec<CustomProviderSummary> = config
        .providers
        .iter()
        .map(|(name, profile)| summary_from_config(name, profile, default_provider, has_stored_key))
        .collect();
    out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    out
}

fn summary_from_config(
    name: &str,
    profile: &NamedProviderConfig,
    default_provider: Option<&str>,
    has_stored_key: &dyn Fn(&str, &str) -> bool,
) -> CustomProviderSummary {
    let inline_key = profile
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    let env_key = profile.api_key_env.as_deref().unwrap_or("");
    let env_file = profile
        .env_file
        .as_deref()
        .unwrap_or("openai-compatible.env");
    let has_api_key = inline_key || (!env_key.is_empty() && has_stored_key(env_key, env_file));
    let mut models: Vec<String> = profile.models.iter().map(|m| m.id.clone()).collect();
    if let Some(default) = profile
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        && !models.iter().any(|existing| existing == default)
    {
        models.insert(0, default.to_string());
    }
    CustomProviderSummary {
        name: name.to_string(),
        base_url: profile.base_url.clone(),
        default_model: profile.default_model.clone(),
        models,
        model_catalog: profile.model_catalog,
        has_api_key,
        is_default: default_provider == Some(name),
    }
}

/// Normalize user-entered endpoint URLs, adding scheme and `/v1` when omitted.
pub fn normalize_endpoint_url(raw: &str, append_v1_if_bare_origin: bool) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter the API Base URL first (for example https://api.example.com/v1).".into());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else if looks_local_host(trimmed) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    let normalized = normalize_api_base(&with_scheme).ok_or_else(|| {
        format!(
            "Invalid Base URL '{trimmed}'. Use https://... for remote servers, or http://127.0.0.1 / LAN IP for local servers."
        )
    })?;
    if !append_v1_if_bare_origin {
        return Ok(normalized);
    }
    Ok(append_v1_if_root(&normalized))
}

fn append_v1_if_root(normalized: &str) -> String {
    let Some(after_scheme) = normalized.split_once("://").map(|(_, rest)| rest) else {
        return normalized.to_string();
    };
    if !after_scheme.contains('/') {
        format!("{normalized}/v1")
    } else {
        normalized.to_string()
    }
}

fn looks_local_host(host_and_rest: &str) -> bool {
    let authority = host_and_rest.split('/').next().unwrap_or(host_and_rest);
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or(authority)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    let lower = host.to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".local")
        || lower.starts_with("127.")
        || lower.starts_with("10.")
        || lower.starts_with("192.168.")
        || lower.starts_with("100.")
        || lower == "::1"
}

/// Suggest a non-reserved profile name from a Base URL.
pub fn suggest_name(base_url: &str) -> String {
    let Ok(normalized) = normalize_endpoint_url(base_url, false) else {
        return "custom-api".into();
    };
    let after_scheme = normalized
        .split_once("://")
        .map_or(normalized.as_str(), |(_, rest)| rest);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (h, p) = rest.split_once(']').unwrap_or((rest, ""));
        (h, p.strip_prefix(':').unwrap_or(""))
    } else {
        authority.split_once(':').unwrap_or((authority, ""))
    };
    let lower = host.to_ascii_lowercase();
    let raw = if lower == "localhost" || lower == "127.0.0.1" || lower == "::1" {
        if port.is_empty() {
            "local-api".to_string()
        } else {
            format!("local-{port}")
        }
    } else {
        let parts: Vec<&str> = lower
            .split('.')
            .filter(|part| {
                !matches!(
                    *part,
                    "api" | "llm" | "www" | "v1" | "com" | "cn" | "ai" | "io" | "net" | "org" | "dev"
                )
            })
            .collect();
        parts.first().copied().unwrap_or("custom-api").to_string()
    };
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let candidate = if cleaned.is_empty()
        || !cleaned.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
    {
        "custom-api".to_string()
    } else {
        cleaned
    };
    if ensure_profile_name_not_reserved(&candidate).is_err() {
        format!("{candidate}-api")
    } else {
        candidate
    }
}

pub fn parse_model_list(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for token in text.split([',', '\n', '\r']) {
        let model = token.trim();
        if !model.is_empty() && seen.insert(model.to_string()) {
            out.push(model.to_string());
        }
    }
    out
}

/// Fetch model IDs from `{base_url}/models` (with fallback to `{base_url}/v1/models`
/// when `base_url` had no path). Returns `(working_base_url, models)`.
pub fn fetch_models(
    base_url: &str,
    typed_api_key: Option<&str>,
    existing_profile: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let exact = normalize_endpoint_url(base_url, false)?;
    let key = resolve_fetch_key(typed_api_key, existing_profile);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| format!("Could not build HTTP client: {error}"))?;

    let mut candidates = vec![exact.clone()];
    let with_v1 = append_v1_if_root(&exact);
    if with_v1 != exact {
        candidates.push(with_v1);
    }

    let mut last_error = String::new();
    for candidate in candidates {
        let url = format!("{candidate}/models");
        let mut request = client.get(&url).header("Accept", "application/json");
        if let Some(secret) = key.as_deref().filter(|s| !s.is_empty()) {
            request = request.bearer_auth(secret);
        }
        match request.send() {
            Ok(response) => {
                let status = response.status();
                let body = response.text().unwrap_or_default();
                if !status.is_success() {
                    last_error = format!("GET {url} returned HTTP {}", status.as_u16());
                    continue;
                }
                match extract_model_ids(&body) {
                    Ok(models) if !models.is_empty() => return Ok((candidate, models)),
                    Ok(_) => {
                        last_error = format!("{url} returned an empty model list.");
                    }
                    Err(error) => {
                        last_error = format!("{url}: {error}");
                    }
                }
            }
            Err(error) => {
                last_error = format!("Could not reach {url}: {error}");
            }
        }
    }
    Err(last_error)
}

fn resolve_fetch_key(typed_api_key: Option<&str>, existing_profile: Option<&str>) -> Option<String> {
    if let Some(key) = typed_api_key.map(str::trim).filter(|k| !k.is_empty()) {
        return Some(key.to_string());
    }
    let profile_name = existing_profile?;
    let path = config_path().ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    let config = parse_config_or_default(&content).ok()?;
    let profile = config.providers.get(profile_name)?;
    if let Some(inline) = profile.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        return Some(inline.to_string());
    }
    let env_key = profile.api_key_env.as_deref()?;
    let env_file = profile
        .env_file
        .as_deref()
        .unwrap_or("openai-compatible.env");
    load_api_key_from_env_or_config(env_key, env_file)
}

fn extract_model_ids(body: &str) -> Result<Vec<String>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "response was not valid JSON".to_string())?;
    let array = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.get("models").and_then(serde_json::Value::as_array))
        .or_else(|| value.as_array())
        .ok_or_else(|| "JSON did not contain a `data` or `models` array".to_string())?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in array {
        let id = item
            .as_str()
            .or_else(|| item.get("id").and_then(serde_json::Value::as_str))
            .or_else(|| item.get("name").and_then(serde_json::Value::as_str))
            .map(str::trim)
            .filter(|id| !id.is_empty());
        if let Some(id) = id {
            let clean = id.strip_prefix("models/").unwrap_or(id);
            if seen.insert(clean.to_string()) {
                out.push(clean.to_string());
            }
        }
    }
    Ok(out)
}

pub fn save(draft: CustomProviderDraft) -> Result<CustomProviderSummary, String> {
    let path = config_path()?;
    let summary = save_at(
        &path,
        draft,
        &|env_file, env_key, secret| {
            save_named_api_key(env_file, env_key, secret).map_err(|e| e.to_string())
        },
        &|env_key, env_file| load_api_key_from_env_or_config(env_key, env_file).is_some(),
    )?;
    reload_runtime_server();
    Ok(summary)
}

pub fn save_at(
    path: &Path,
    draft: CustomProviderDraft,
    store_key: &dyn Fn(&str, &str, &str) -> Result<(), String>,
    has_stored_key: &dyn Fn(&str, &str) -> bool,
) -> Result<CustomProviderSummary, String> {
    let api_base = normalize_endpoint_url(&draft.base_url, true)?;
    let raw_name = if draft.name.trim().is_empty() {
        suggest_name(&api_base)
    } else {
        draft.name.trim().to_string()
    };
    let name = validate_profile_name(&raw_name)?;
    ensure_profile_name_not_reserved(&name)?;

    let mut model_ids = Vec::new();
    let mut seen = BTreeSet::new();
    for id in std::iter::once(draft.default_model.as_str())
        .chain(draft.models.iter().map(String::as_str))
    {
        let trimmed = id.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            model_ids.push(trimmed.to_string());
        }
    }
    let Some(default_model) = model_ids.first().cloned() else {
        return Err(
            "Enter at least one model ID, or click Fetch models to load them from the server."
                .into(),
        );
    };

    let original_content = std::fs::read_to_string(path).unwrap_or_default();
    let existing_config = parse_config_or_default(&original_content)
        .map_err(|error| format!("Existing config.toml is invalid: {error}"))?;

    let editing_key = draft
        .original_name
        .as_deref()
        .filter(|old| !old.is_empty())
        .unwrap_or(&name);
    let previous = existing_config.providers.get(editing_key);

    if draft.original_name.as_deref() != Some(name.as_str())
        && existing_config.providers.contains_key(&name)
        && draft.original_name.is_some()
    {
        return Err(format!(
            "A custom provider named '{name}' already exists. Pick another name."
        ));
    }

    let typed_key = draft
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());

    let (auth, auth_header, api_key_env, env_file, inline_api_key, requires_api_key) =
        if let Some(secret) = typed_key {
            let env_key = derived_api_key_env(&name);
            let file_name = format!("provider-{name}.env");
            store_key(&file_name, &env_key, secret)?;
            (
                NamedProviderAuth::Bearer,
                None,
                Some(env_key),
                Some(file_name),
                None,
                Some(true),
            )
        } else if let Some(prev) = previous {
            (
                prev.auth.clone(),
                prev.auth_header.clone(),
                prev.api_key_env.clone(),
                prev.env_file.clone(),
                prev.api_key.clone(),
                prev.requires_api_key,
            )
        } else if api_base_uses_localhost(&api_base) || looks_local_host(&api_base) {
            (NamedProviderAuth::None, None, None, None, None, Some(false))
        } else {
            return Err(
                "Paste an API key for this remote endpoint (local 127.0.0.1 / LAN servers can omit it)."
                    .into(),
            );
        };

    // Preserve rich per-model metadata (context_window, input, reasoning) for
    // models that already existed on this profile (e.g. Magpie-written entries).
    let models: Vec<NamedProviderModelConfig> = model_ids
        .into_iter()
        .map(|id| {
            if let Some(existing_model) = previous
                .and_then(|prev| prev.models.iter().find(|m| m.id == id))
                .cloned()
            {
                existing_model
            } else {
                NamedProviderModelConfig {
                    id,
                    reasoning: None,
                    reasoning_effort: None,
                    context_window: None,
                    input: Vec::new(),
                }
            }
        })
        .collect();

    let profile = NamedProviderConfig {
        provider_type: NamedProviderType::OpenAiCompatible,
        base_url: api_base,
        api: previous.and_then(|p| p.api.clone()),
        auth,
        auth_header,
        headers: previous.map(|p| p.headers.clone()).unwrap_or_default(),
        api_key_env,
        api_key: inline_api_key,
        env_file,
        default_model: Some(default_model.clone()),
        requires_api_key,
        provider_routing: previous.is_some_and(|p| p.provider_routing),
        model_catalog: draft.model_catalog,
        allow_provider_pinning: previous.is_some_and(|p| p.allow_provider_pinning),
        models,
        extra_body: previous.and_then(|p| p.extra_body.clone()),
        supports_reasoning_effort: previous.and_then(|p| p.supports_reasoning_effort),
        disable_reasoning_heuristics: previous.is_some_and(|p| p.disable_reasoning_heuristics),
    };

    let mut updated = remove_named_provider_sections(&original_content, &name);
    if let Some(old_name) = draft.original_name.as_deref()
        && old_name != name
    {
        updated = remove_named_provider_sections(&updated, old_name);
    }
    let was_default = existing_config.provider.default_provider.as_deref() == Some(editing_key);
    if draft.set_default || was_default {
        updated = upsert_provider_defaults(updated, &name, &default_model);
    }
    updated = append_profile_section(updated, &name, &profile)?;

    let parsed = toml::from_str::<Config>(&updated)
        .map_err(|error| format!("Generated config.toml failed validation: {error}"))?;
    write_atomic(path, updated.as_bytes())?;

    let default_after = parsed.provider.default_provider.as_deref();
    Ok(summary_from_config(
        &name,
        &profile,
        default_after,
        has_stored_key,
    ))
}

pub fn make_default(name: &str) -> Result<Option<String>, String> {
    let path = config_path()?;
    let spec = make_default_at(&path, name)?;
    reload_runtime_server();
    Ok(spec)
}

pub fn make_default_at(path: &Path, name: &str) -> Result<Option<String>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read config.toml: {error}"))?;
    let config = parse_config_or_default(&content)
        .map_err(|error| format!("Invalid config.toml: {error}"))?;
    let profile = config
        .providers
        .get(name)
        .ok_or_else(|| format!("Custom provider '{name}' was not found."))?;
    let model = profile
        .default_model
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .or_else(|| profile.models.first().map(|m| m.id.as_str()))
        .unwrap_or("")
        .to_string();
    let updated = upsert_provider_defaults(content, name, &model);
    toml::from_str::<Config>(&updated)
        .map_err(|error| format!("Updated config.toml failed validation: {error}"))?;
    write_atomic(path, updated.as_bytes())?;
    Ok((!model.is_empty()).then(|| format!("{name}:{model}")))
}

pub fn remove(name: &str) -> Result<(), String> {
    let path = config_path()?;
    remove_at(&path, name)?;
    reload_runtime_server();
    Ok(())
}

pub fn remove_at(path: &Path, name: &str) -> Result<(), String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let config = parse_config_or_default(&content)
        .map_err(|error| format!("Invalid config.toml: {error}"))?;
    let mut updated = remove_named_provider_sections(&content, name);
    if config.provider.default_provider.as_deref() == Some(name) {
        updated = clear_provider_defaults(updated);
    }
    toml::from_str::<Config>(&updated)
        .map_err(|error| format!("Updated config.toml failed validation: {error}"))?;
    write_atomic(path, updated.as_bytes())
}

/// Reload the shared background daemon so newly saved `[providers.<name>]`
/// routes appear in live sessions immediately without dropping active work.
pub fn reload_runtime_server() {
    let jcode = crate::platform::companion_executable("jcode");
    let legacy = jcode_sdk::api::legacy_socket_path();
    if jcode_sdk::socket_accepts(&legacy) {
        let mut command = std::process::Command::new(&jcode);
        crate::platform::no_console_window(&mut command);
        let _ = command
            .args(["server", "reload", "--force", "--quiet"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    crate::accounts::request_refresh();
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("custom-api.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    if cfg!(windows) && path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn parse_config_or_default(content: &str) -> Result<Config, toml::de::Error> {
    if content.trim().is_empty() {
        Ok(Config::default())
    } else {
        toml::from_str::<Config>(content)
    }
}

fn validate_profile_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Profile name cannot be empty.".into());
    }
    if name.len() > 64 {
        return Err("Profile name must be 64 characters or fewer.".into());
    }
    let mut chars = name.chars();
    if !chars.next().is_some_and(|ch| ch.is_ascii_alphanumeric()) {
        return Err("Profile name must start with an English letter or digit.".into());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("Profile name may only contain letters, digits, '-' and '_'.".into());
    }
    Ok(name.to_string())
}

fn ensure_profile_name_not_reserved(name: &str) -> Result<(), String> {
    if resolve_login_provider(name).is_some()
        || RESERVED_NAMES
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return Err(format!(
            "'{name}' is a built-in provider ID. Pick another name such as '{name}-api'."
        ));
    }
    Ok(())
}

fn derived_api_key_env(name: &str) -> String {
    let suffix: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let candidate = format!("JCODE_PROVIDER_{suffix}_API_KEY");
    debug_assert!(is_safe_env_key_name(&candidate));
    debug_assert!(is_safe_env_file_name(&format!("provider-{name}.env")));
    candidate
}

fn append_profile_section(
    mut content: String,
    name: &str,
    profile: &NamedProviderConfig,
) -> Result<String, String> {
    let mut table = toml::map::Map::new();
    let value = toml::Value::try_from(profile).map_err(|e| e.to_string())?;
    table.insert(name.to_string(), value);
    let mut providers = toml::map::Map::new();
    providers.insert("providers".into(), toml::Value::Table(table));
    let rendered =
        toml::to_string_pretty(&toml::Value::Table(providers)).map_err(|e| e.to_string())?;

    if !content.trim().is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.trim().is_empty() && !content.ends_with("\n\n") {
        content.push('\n');
    }
    content.push_str(rendered.trim_end());
    content.push('\n');
    Ok(content)
}

fn toml_quote(text: &str) -> String {
    toml::Value::String(text.to_string()).to_string()
}

fn upsert_provider_defaults(content: String, profile: &str, model: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();
    let Some(idx) = lines
        .iter()
        .position(|line| line.split('#').next().is_some_and(|h| h.trim() == "[provider]"))
    else {
        let mut prefix = format!(
            "[provider]\ndefault_provider = {}\ndefault_model = {}\n\n",
            toml_quote(profile),
            toml_quote(model)
        );
        if !content.trim().is_empty() {
            prefix.push_str(content.trim_start_matches('\n'));
            if !prefix.ends_with('\n') {
                prefix.push('\n');
            }
        }
        return prefix;
    };

    let section_end = |lines: &[String], from: usize| {
        lines
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, line)| is_toml_header(line))
            .map(|(index, _)| index)
            .unwrap_or(lines.len())
    };

    let end = section_end(&lines, idx + 1);
    upsert_key_in_range(
        &mut lines,
        idx + 1,
        end,
        "default_provider",
        &toml_quote(profile),
    );
    let end = section_end(&lines, idx + 1);
    upsert_key_in_range(
        &mut lines,
        idx + 1,
        end,
        "default_model",
        &toml_quote(model),
    );
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn clear_provider_defaults(content: String) -> String {
    let lines: Vec<String> = content.lines().map(str::to_owned).collect();
    let Some(idx) = lines
        .iter()
        .position(|line| line.split('#').next().is_some_and(|h| h.trim() == "[provider]"))
    else {
        return content;
    };
    let end = lines
        .iter()
        .enumerate()
        .skip(idx + 1)
        .find(|(_, line)| is_toml_header(line))
        .map(|(index, _)| index)
        .unwrap_or(lines.len());
    let mut kept = Vec::with_capacity(lines.len());
    for (i, line) in lines.into_iter().enumerate() {
        if i > idx
            && i < end
            && (line_has_toml_key(&line, "default_provider")
                || line_has_toml_key(&line, "default_model"))
        {
            continue;
        }
        kept.push(line);
    }
    let mut out = kept.join("\n");
    out.push('\n');
    out
}

fn upsert_key_in_range(lines: &mut Vec<String>, start: usize, end: usize, key: &str, value: &str) {
    for line in lines.iter_mut().take(end).skip(start) {
        if line_has_toml_key(line, key) {
            *line = format!("{key} = {value}");
            return;
        }
    }
    lines.insert(start, format!("{key} = {value}"));
}

fn remove_named_provider_sections(content: &str, name: &str) -> String {
    let mut kept = Vec::new();
    let mut skip = false;
    for line in content.lines() {
        if is_toml_header(line) {
            skip = is_named_provider_header(line, name);
        }
        if !skip {
            kept.push(line);
        }
    }
    if kept.is_empty() {
        String::new()
    } else {
        let mut out = kept.join("\n");
        out.push('\n');
        out
    }
}

fn is_named_provider_header(line: &str, name: &str) -> bool {
    let trimmed = line.split('#').next().unwrap_or(line).trim();
    let inner = if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
        &trimmed[2..trimmed.len() - 2]
    } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        return false;
    };
    let inner = inner.trim();
    let quoted = toml_quote(name);
    for prefix in [
        format!("providers.{name}"),
        format!("providers.{quoted}"),
        format!("providers.'{name}'"),
    ] {
        if inner == prefix || inner.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }
    false
}

fn is_toml_header(line: &str) -> bool {
    let trimmed = line.split('#').next().unwrap_or(line).trim();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

fn line_has_toml_key(line: &str, key: &str) -> bool {
    line.split('#')
        .next()
        .and_then(|head| head.split_once('='))
        .is_some_and(|(candidate, _)| candidate.trim() == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn save_edit_default_and_remove_preserve_comments_and_stored_keys() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "# keep user comment\n[display]\ntheme = \"ocean\"\n").unwrap();

        let keys: RefCell<HashMap<(String, String), String>> = RefCell::new(HashMap::new());
        let store = |file: &str, env_key: &str, secret: &str| {
            keys.borrow_mut()
                .insert((env_key.to_string(), file.to_string()), secret.to_string());
            Ok(())
        };
        let check = |env_key: &str, file: &str| {
            keys.borrow()
                .contains_key(&(env_key.to_string(), file.to_string()))
        };

        let saved = save_at(
            &path,
            CustomProviderDraft {
                original_name: None,
                name: String::new(),
                base_url: "api.siliconflow.cn".into(),
                api_key: Some("sk-secret-value-123".into()),
                default_model: "deepseek-ai/DeepSeek-V3".into(),
                models: vec![
                    "deepseek-ai/DeepSeek-V3".into(),
                    "Qwen/Qwen2.5-Coder-32B".into(),
                ],
                set_default: true,
                model_catalog: true,
            },
            &store,
            &check,
        )
        .unwrap();

        assert_eq!(saved.name, "siliconflow");
        assert_eq!(saved.base_url, "https://api.siliconflow.cn/v1");
        assert!(saved.is_default);
        assert!(saved.has_api_key);
        assert_eq!(saved.models.len(), 2);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# keep user comment"));
        assert!(!raw.contains("sk-secret-value-123"), "secret never in config.toml");

        // Editing without typing a new key keeps the stored credential.
        let edited = save_at(
            &path,
            CustomProviderDraft {
                original_name: Some("siliconflow".into()),
                name: "siliconflow".into(),
                base_url: "https://api.siliconflow.cn/v1".into(),
                api_key: None,
                default_model: "Qwen/Qwen2.5-Coder-32B".into(),
                models: vec!["Qwen/Qwen2.5-Coder-32B".into()],
                set_default: true,
                model_catalog: false,
            },
            &store,
            &check,
        )
        .unwrap();
        assert!(edited.has_api_key);
        assert_eq!(
            edited.route_spec().as_deref(),
            Some("siliconflow:Qwen/Qwen2.5-Coder-32B")
        );

        // Built-in provider names are rejected with a clear hint.
        let err = save_at(
            &path,
            CustomProviderDraft {
                name: "openai".into(),
                base_url: "https://api.example.com/v1".into(),
                api_key: Some("sk-1".into()),
                default_model: "m".into(),
                ..Default::default()
            },
            &store,
            &check,
        )
        .unwrap_err();
        assert!(err.contains("built-in provider ID"));

        remove_at(&path, "siliconflow").unwrap();
        assert!(list_at(&path, &check).is_empty());
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# keep user comment"));
        assert!(!after.contains("default_provider"));
    }

    #[test]
    fn fetch_models_falls_back_to_v1_models_and_sends_bearer_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for expected in ["GET /models ", "GET /v1/models "] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.starts_with(expected), "{req}");
                assert!(req.contains("Authorization: Bearer test-token\r\n") || req.contains("authorization: Bearer test-token\r\n"));
                let response = if expected == "GET /models " {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    let body = r#"{"data":[{"id":"codex/gpt-6.2-sol"},{"id":"cpr-fast/gpt-6.2-sol"}]}"#;
                    &format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let (working_base, models) =
            fetch_models(&format!("127.0.0.1:{}", addr.port()), Some("test-token"), None).unwrap();
        server.join().unwrap();
        assert_eq!(working_base, format!("http://127.0.0.1:{}/v1", addr.port()));
        assert_eq!(models, ["codex/gpt-6.2-sol", "cpr-fast/gpt-6.2-sol"]);
    }
}
