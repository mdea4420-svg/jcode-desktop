//! Named third-party API profiles (`[providers.<name>]` in
//! `~/.jcode/config.toml`), with private key storage and `/models` discovery.
//!
//! Secrets are written through `jcode_base::provider_catalog::save_named_api_key`
//! into Jcode's private per-provider `.env` file and never returned to UI state
//! or written in plaintext to `config.toml`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use jcode_base::config::{
    Config, NamedProviderAuth, NamedProviderConfig, NamedProviderModelConfig, NamedProviderType,
};
use jcode_base::provider_catalog::{
    OPENAI_COMPAT_LOCAL_ENABLED_ENV, OPENAI_COMPAT_PROFILE, api_base_uses_localhost,
    is_safe_env_file_name, is_safe_env_key_name, load_api_key_from_env_or_config,
    load_env_value_from_env_or_config, normalize_api_base, openai_compatible_profiles,
    resolve_login_provider, resolve_openai_compatible_profile, save_named_api_key,
};

const RESERVED_NAMES: &[&str] = &[
    "auto",
    "claude-subprocess",
    "compat",
    "custom",
    "azure-openai",
    "aoai",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CustomApiFormat {
    #[default]
    OpenAiCompatible,
    AnthropicCompatible,
    OpenRouter,
}

impl CustomApiFormat {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI Chat Completions",
            Self::AnthropicCompatible => "Anthropic Messages",
            Self::OpenRouter => "OpenRouter",
        }
    }

    fn default_auth(self) -> (NamedProviderAuth, Option<String>) {
        match self {
            Self::AnthropicCompatible => (NamedProviderAuth::Header, Some("x-api-key".into())),
            _ => (NamedProviderAuth::Bearer, None),
        }
    }

    fn default_key_header(self) -> &'static str {
        match self {
            Self::AnthropicCompatible => "x-api-key",
            _ => "api-key",
        }
    }
}

impl From<&NamedProviderType> for CustomApiFormat {
    fn from(value: &NamedProviderType) -> Self {
        match value {
            NamedProviderType::OpenAiCompatible => Self::OpenAiCompatible,
            NamedProviderType::AnthropicCompatible => Self::AnthropicCompatible,
            NamedProviderType::OpenRouter => Self::OpenRouter,
        }
    }
}

impl From<CustomApiFormat> for NamedProviderType {
    fn from(value: CustomApiFormat) -> Self {
        match value {
            CustomApiFormat::OpenAiCompatible => Self::OpenAiCompatible,
            CustomApiFormat::AnthropicCompatible => Self::AnthropicCompatible,
            CustomApiFormat::OpenRouter => Self::OpenRouter,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomProviderSummary {
    pub name: String,
    pub base_url: String,
    pub default_model: Option<String>,
    pub models: Vec<String>,
    pub model_catalog: bool,
    pub has_api_key: bool,
    pub is_default: bool,
    pub format: CustomApiFormat,
    pub enabled: bool,
}

impl Default for CustomProviderSummary {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_url: String::new(),
            default_model: None,
            models: Vec::new(),
            model_catalog: false,
            has_api_key: false,
            is_default: false,
            format: CustomApiFormat::default(),
            enabled: true,
        }
    }
}

impl CustomProviderSummary {
    pub fn host_summary(&self) -> String {
        reqwest::Url::parse(&self.base_url)
            .ok()
            .map(|url| {
                let host = url.host_str().unwrap_or_default();
                url.port().map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"))
            })
            .unwrap_or_default()
    }

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
    pub model_settings: BTreeMap<String, ModelSettingsEdit>,
    pub format: CustomApiFormat,
    pub cached_catalog: Option<Vec<CatalogModelEntry>>,
    pub supports_reasoning_effort: Option<bool>,
    pub set_default: bool,
    pub model_catalog: bool,
}

pub const MODEL_REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelSettings {
    pub context_window: Option<usize>,
    pub reasoning: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub image_input: Option<bool>,
    pub pdf_input: Option<bool>,
    pub max_output_tokens: Option<usize>,
    pub alias: Option<String>,
    pub reasoning_levels: Option<Vec<String>>,
    pub subagent_schedulable: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelSettingsEdit {
    // The outer option indicates that the field was edited. The inner None
    // clears a previously configured override so the runtime can infer it.
    pub context_window: Option<Option<usize>>,
    pub reasoning: Option<Option<bool>>,
    pub reasoning_effort: Option<Option<String>>,
    pub image_input: Option<Option<bool>>,
    pub pdf_input: Option<Option<bool>>,
    pub max_output_tokens: Option<Option<usize>>,
    pub alias: Option<Option<String>>,
    pub reasoning_levels: Option<Option<Vec<String>>>,
    pub subagent_schedulable: Option<Option<bool>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogModelEntry {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<usize>,
    pub max_output_tokens: Option<usize>,
    pub reasoning_levels: Vec<String>,
    pub default_reasoning_effort: Option<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct DesktopProviderState {
    disabled_providers: BTreeMap<String, NamedProviderConfig>,
    cached_catalog: BTreeMap<String, Vec<CatalogModelEntry>>,
    model_meta: BTreeMap<String, BTreeMap<String, DesktopModelMeta>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct DesktopModelMeta {
    max_output_tokens: Option<usize>,
    alias: Option<String>,
    reasoning_levels: Option<Vec<String>>,
    subagent_schedulable: Option<bool>,
}

const DESKTOP_PROVIDERS_FILE: &str = "desktop-custom-providers.json";
static PROVIDER_WRITES: Mutex<()> = Mutex::new(());

fn lock_provider_writes() -> Result<MutexGuard<'static, ()>, String> {
    PROVIDER_WRITES.lock().map_err(|_| "Custom provider write lock was poisoned.".into())
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

fn read_config_at(path: &Path) -> Result<(String, Config), String> {
    let content = read_optional(path)?.unwrap_or_default();
    let config = parse_config_or_default(&content)
        .map_err(|error| format!("Invalid config.toml: {error}"))?;
    Ok((content, config))
}

fn read_desktop_state_at(path: &Path) -> Result<(Option<String>, DesktopProviderState), String> {
    let raw = read_optional(&path.with_file_name(DESKTOP_PROVIDERS_FILE))?;
    let state = match &raw {
        Some(text) => serde_json::from_str(text)
            .map_err(|error| format!("Invalid {DESKTOP_PROVIDERS_FILE}: {error}"))?,
        None => DesktopProviderState::default(),
    };
    Ok((raw, state))
}

fn write_desktop_state_at(path: &Path, state: &DesktopProviderState) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    write_atomic(&path.with_file_name(DESKTOP_PROVIDERS_FILE), &bytes)
}

// For an operation that writes both files, put the recoverable snapshot in
// place first. Readers prefer the active profile if both copies exist.
fn write_provider_state_at(
    path: &Path,
    config_update: Option<&str>,
    original: Option<&str>,
    state: &DesktopProviderState,
    state_changed: bool,
) -> Result<(), String> {
    if state_changed {
        write_desktop_state_at(path, state)?;
    }
    if let Some(content) = config_update
        && let Err(error) = write_atomic(path, content.as_bytes())
    {
        if state_changed {
            let sidecar = path.with_file_name(DESKTOP_PROVIDERS_FILE);
            let restored = match original {
                Some(raw) => write_atomic(&sidecar, raw.as_bytes()),
                None => std::fs::remove_file(&sidecar).map_err(|error| error.to_string()),
            };
            if let Err(restore_error) = restored {
                return Err(format!("{error}; could not restore {DESKTOP_PROVIDERS_FILE}: {restore_error}"));
            }
        }
        return Err(error);
    }
    Ok(())
}

pub fn cached_catalog(name: &str) -> Vec<CatalogModelEntry> {
    config_path().ok().map_or_else(Vec::new, |path| cached_catalog_at(&path, name))
}

pub fn cached_catalog_at(path: &Path, name: &str) -> Vec<CatalogModelEntry> {
    read_desktop_state_at(path)
        .ok()
        .and_then(|(_, mut state)| state.cached_catalog.remove(name))
        .unwrap_or_default()
}

pub fn save_cached_catalog(name: &str, entries: Vec<CatalogModelEntry>) -> Result<(), String> {
    save_cached_catalog_at(&config_path()?, name, entries)
}

pub fn save_cached_catalog_at(
    path: &Path,
    name: &str,
    entries: Vec<CatalogModelEntry>,
) -> Result<(), String> {
    let _lock = lock_provider_writes()?;
    let name = validate_profile_name(name)?;
    let (_, mut state) = read_desktop_state_at(path)?;
    state.cached_catalog.insert(name, entries);
    write_desktop_state_at(path, &state)
}

pub fn provider_model_settings(name: &str) -> HashMap<String, ModelSettings> {
    config_path().ok().map_or_else(HashMap::new, |path| provider_model_settings_at(&path, name))
}

pub fn provider_model_settings_at(path: &Path, name: &str) -> HashMap<String, ModelSettings> {
    let Ok((_, config)) = read_config_at(path) else { return HashMap::new() };
    let state = read_desktop_state_at(path).map(|(_, state)| state).unwrap_or_default();
    config.providers.get(name).or_else(|| state.disabled_providers.get(name))
        .map_or_else(HashMap::new, |profile| model_settings_for_profile(profile, state.model_meta.get(name)))
}

pub fn offline_model_settings(name: &str) -> HashMap<String, ModelSettings> {
    model_settings_from_config(
        r#"
[providers.local-gateway]
base_url = "http://127.0.0.1:3425/v1"
models = [{ id = "codex/gpt-6.2-sol", context_window = 272000, reasoning = true, reasoning_effort = "high", input = ["text", "image"] }]
"#,
        name,
    )
}

fn model_settings_from_config(content: &str, name: &str) -> HashMap<String, ModelSettings> {
    parse_config_or_default(content).ok()
        .and_then(|mut config| config.providers.remove(name))
        .map_or_else(HashMap::new, |profile| model_settings_for_profile(&profile, None))
}

fn model_settings_for_profile(
    profile: &NamedProviderConfig,
    desktop: Option<&BTreeMap<String, DesktopModelMeta>>,
) -> HashMap<String, ModelSettings> {
    let mut settings = HashMap::new();
    for row in &profile.models {
        settings.entry(row.id.clone()).or_insert_with(|| {
            model_settings(row, desktop.and_then(|meta| meta.get(&row.id)))
        });
    }
    settings
}

fn model_settings(row: &NamedProviderModelConfig, desktop: Option<&DesktopModelMeta>) -> ModelSettings {
    let accepts = |modality: &str| (!row.input.is_empty())
        .then(|| row.input.iter().any(|kind| kind.eq_ignore_ascii_case(modality)));
    ModelSettings {
        context_window: row.context_window,
        reasoning: row.reasoning,
        reasoning_effort: row.reasoning_effort.clone(),
        image_input: accepts("image"),
        pdf_input: accepts("pdf"),
        max_output_tokens: desktop.and_then(|meta| meta.max_output_tokens),
        alias: desktop.and_then(|meta| meta.alias.clone()),
        reasoning_levels: desktop.and_then(|meta| meta.reasoning_levels.clone()),
        subagent_schedulable: desktop.and_then(|meta| meta.subagent_schedulable),
    }
}

impl ModelSettingsEdit {
    fn validate(&self, id: &str) -> Result<(), String> {
        if self.context_window.flatten()
            .is_some_and(|tokens| tokens == 0 || i64::try_from(tokens).is_err())
        {
            return Err(format!("Model '{id}': context_window must be a positive whole number of tokens."));
        }
        if self.max_output_tokens.flatten() == Some(0) {
            return Err(format!("Model '{id}': max_output_tokens must be a positive whole number of tokens."));
        }
        if let Some(Some(effort)) = &self.reasoning_effort
            && !MODEL_REASONING_EFFORTS.contains(&effort.as_str())
        {
            return Err(format!("Model '{id}': unsupported reasoning_effort '{effort}'."));
        }
        if let Some(Some(levels)) = &self.reasoning_levels
            && let Some(level) = levels.iter().find(|value| !MODEL_REASONING_EFFORTS.contains(&value.as_str()))
        {
            return Err(format!("Model '{id}': unsupported reasoning_levels value '{level}'."));
        }
        Ok(())
    }

    fn apply(self, model: &mut NamedProviderModelConfig, desktop: &mut DesktopModelMeta) {
        if let Some(value) = self.context_window { model.context_window = value; }
        if let Some(value) = self.reasoning { model.reasoning = value; }
        if let Some(value) = self.reasoning_effort { model.reasoning_effort = value; }
        if let Some(value) = self.max_output_tokens { desktop.max_output_tokens = value; }
        if let Some(value) = self.alias { desktop.alias = value; }
        if let Some(value) = self.reasoning_levels { desktop.reasoning_levels = value; }
        if let Some(value) = self.subagent_schedulable { desktop.subagent_schedulable = value; }
        let modalities = [("image", self.image_input), ("pdf", self.pdf_input)];
        let explicitly_configured = modalities.iter().any(|(_, value)| matches!(value, Some(Some(_))));
        if explicitly_configured && !model.input.iter().any(|kind| kind.eq_ignore_ascii_case("text")) {
            model.input.push("text".into());
        }
        for (modality, value) in modalities {
            match value {
                Some(Some(true)) => {
                    if !model.input.iter().any(|kind| kind.eq_ignore_ascii_case(modality)) {
                        model.input.push(modality.into());
                    }
                }
                Some(_) => model.input.retain(|kind| !kind.eq_ignore_ascii_case(modality)),
                None => {}
            }
        }
        if !explicitly_configured && modalities.iter().any(|(_, value)| *value == Some(None))
            && model.input.len() == 1 && model.input[0].eq_ignore_ascii_case("text")
        {
            model.input.clear();
        }
    }
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
        format: CustomApiFormat::default(),
        enabled: true,
    }]
}

/// The runtime model fields combined with desktop-only metadata.
pub type ConfiguredModelMeta = ModelSettings;

/// `(profile, model id)` -> configured capabilities for active routes.
pub fn configured_model_metadata() -> HashMap<(String, String), ConfiguredModelMeta> {
    config_path().ok().map_or_else(HashMap::new, |path| configured_model_metadata_at(&path))
}

pub fn configured_model_metadata_at(path: &Path) -> HashMap<(String, String), ConfiguredModelMeta> {
    let Ok((_, config)) = read_config_at(path) else { return HashMap::new() };
    let state = read_desktop_state_at(path).map(|(_, state)| state).unwrap_or_default();
    let mut out = HashMap::new();
    for (name, profile) in &config.providers {
        let desktop = state.model_meta.get(name);
        for model in &profile.models {
            if !model.id.is_empty() {
                out.insert((name.clone(), model.id.clone()), model_settings(model, desktop.and_then(|meta| meta.get(&model.id))));
            }
        }
    }
    out
}

/// `openai-compatible:<id>` route methods for built-in presets that hold no
/// usable credential. The daemon marks every preset available while a named
/// profile is active (its configured check short-circuits to the named
/// profile), so the picker drops the presets nothing configured.
pub fn unconfigured_preset_route_methods() -> HashSet<String> {
    let named = named_profile_names();
    unconfigured_preset_route_methods_with(&named, |env_var, env_file| {
        load_api_key_from_env_or_config(env_var, env_file)
            .or_else(|| load_env_value_from_env_or_config(env_var, env_file))
    })
}

/// `lookup` answers `(env_var, env_file)` credential checks so tests can drive
/// every branch without touching the real environment.
fn unconfigured_preset_route_methods_with(
    named: &HashSet<String>,
    lookup: impl Fn(&str, &str) -> Option<String>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for profile in openai_compatible_profiles() {
        // A `[providers.<name>]` profile with a preset's id keeps its routes;
        // it is the user's own configuration, not the built-in preset.
        if named.contains(profile.id) {
            continue;
        }
        let resolved = resolve_openai_compatible_profile(*profile);
        // Mirrors `openai_compatible_profile_is_configured` without the
        // named-profile shortcut that marks every preset configured.
        let configured = lookup(&resolved.api_key_env, &resolved.env_file).is_some()
            || (!resolved.requires_api_key
                && ((resolved.id == OPENAI_COMPAT_PROFILE.id
                    && api_base_uses_localhost(&resolved.api_base))
                    || lookup(OPENAI_COMPAT_LOCAL_ENABLED_ENV, &resolved.env_file)
                        .map(|value| env_flag_enabled(&value))
                        .unwrap_or(false)));
        if !configured {
            out.insert(format!("openai-compatible:{}", resolved.id));
        }
    }
    out
}

/// `[providers.<name>]` profile names, used to keep user profiles that happen
/// to share a built-in preset's id.
pub fn named_profile_names() -> HashSet<String> {
    let Ok(path) = config_path() else {
        return HashSet::new();
    };
    named_profile_names_at(&path)
}

fn named_profile_names_at(path: &Path) -> HashSet<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    let Ok(config) = parse_config_or_default(&content) else {
        return HashSet::new();
    };
    config.providers.keys().cloned().collect()
}

fn env_flag_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Offline metadata for the screenshot model fixture's Magpie routes.
pub fn offline_model_metadata() -> HashMap<(String, String), ConfiguredModelMeta> {
    let meta = |context_window, image_input, reasoning| ConfiguredModelMeta {
        context_window,
        image_input,
        reasoning,
        ..Default::default()
    };
    [
        (
            ("magpie".to_string(), "codex/gpt-6.2-sol".to_string()),
            meta(Some(272_000), Some(true), Some(true)),
        ),
        (
            ("magpie".to_string(), "cpr-fast/gpt-6.2-sol".to_string()),
            meta(Some(272_000), Some(false), None),
        ),
        (
            ("magpie".to_string(), "truesota/deepseek-flash".to_string()),
            meta(Some(1_000_000), Some(true), None),
        ),
        (
            (
                "magpie".to_string(),
                "workbuddy/deepseek-v4.1-flash".to_string(),
            ),
            meta(Some(1_000_000), None, Some(true)),
        ),
    ]
    .into_iter()
    .collect()
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
    let Ok((_, config)) = read_config_at(path) else { return Vec::new() };
    let state = read_desktop_state_at(path).map(|(_, state)| state).unwrap_or_default();
    let default_provider = config.provider.default_provider.as_deref();
    let mut out: Vec<_> = config.providers.iter()
        .map(|(name, profile)| summary_from_config(name, profile, default_provider, true, has_stored_key))
        .collect();
    out.extend(state.disabled_providers.iter()
        .filter(|(name, _)| !config.providers.contains_key(*name))
        .map(|(name, profile)| summary_from_config(name, profile, None, false, has_stored_key)));
    out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    out
}

fn summary_from_config(
    name: &str,
    profile: &NamedProviderConfig,
    default_provider: Option<&str>,
    enabled: bool,
    has_stored_key: &dyn Fn(&str, &str) -> bool,
) -> CustomProviderSummary {
    let inline_key = profile
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    let env_key = profile.api_key_env.as_deref().unwrap_or("");
    let env_file = env_file_for(profile);
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
        is_default: enabled && default_provider == Some(name),
        format: (&profile.provider_type).into(),
        enabled,
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
/// Compatibility entry point retained for the current custom API editor.
pub fn fetch_models(
    base_url: &str,
    typed_api_key: Option<&str>,
    existing_profile: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let profile = existing_fetch_profile(existing_profile);
    let format = profile.as_ref().map(|profile| CustomApiFormat::from(&profile.provider_type))
        .unwrap_or_default();
    let auth = CatalogAuth::new(format, typed_api_key, profile.as_ref(), &load_api_key_from_env_or_config);
    fetch_model_catalog(base_url, &auth, false, |page, ids| parse_model_catalog(page, ids, |id, _| id.to_owned()))
}

pub fn fetch_model_catalog_rich(
    base_url: &str,
    typed_api_key: Option<&str>,
    existing_profile: Option<&str>,
    format: CustomApiFormat,
) -> Result<(String, Vec<CatalogModelEntry>), String> {
    let auth = catalog_auth(typed_api_key, existing_profile, format);
    fetch_rich_catalog(base_url, &auth, false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionTestReport {
    pub working_base_url: String,
    pub model_count: usize,
    pub latency_ms: u64,
}

pub fn test_connection(
    base_url: &str,
    typed_api_key: Option<&str>,
    existing_profile: Option<&str>,
    format: CustomApiFormat,
) -> Result<ConnectionTestReport, String> {
    let start = Instant::now();
    let auth = catalog_auth(typed_api_key, existing_profile, format);
    let (working_base_url, entries) = fetch_rich_catalog(base_url, &auth, true)?;
    Ok(ConnectionTestReport {
        working_base_url,
        model_count: entries.len(),
        latency_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

fn fetch_rich_catalog(
    base_url: &str,
    auth: &CatalogAuth,
    allow_empty: bool,
) -> Result<(String, Vec<CatalogModelEntry>), String> {
    fetch_model_catalog(base_url, auth, allow_empty, |page, ids| parse_model_catalog(page, ids, catalog_model_entry))
}

fn existing_fetch_profile(name: Option<&str>) -> Option<NamedProviderConfig> {
    let name = name?;
    fetch_profile_at(&config_path().ok()?, name)
}

fn fetch_profile_at(path: &Path, name: &str) -> Option<NamedProviderConfig> {
    let (_, mut config) = read_config_at(path).ok()?;
    config.providers.remove(name).or_else(|| {
        read_desktop_state_at(path).ok()?.1.disabled_providers.remove(name)
    })
}

fn catalog_auth(
    typed_api_key: Option<&str>,
    existing_profile: Option<&str>,
    format: CustomApiFormat,
) -> CatalogAuth {
    let profile = existing_fetch_profile(existing_profile);
    CatalogAuth::new(format, typed_api_key, profile.as_ref(), &load_api_key_from_env_or_config)
}

fn env_file_for(profile: &NamedProviderConfig) -> &str {
    profile.env_file.as_deref().unwrap_or_else(|| {
        if profile.provider_type == NamedProviderType::AnthropicCompatible {
            "anthropic.env"
        } else {
            "openai-compatible.env"
        }
    })
}

struct CatalogAuth {
    format: CustomApiFormat,
    auth: NamedProviderAuth,
    header: Option<String>,
    headers: BTreeMap<String, String>,
    key: Option<String>,
}

impl CatalogAuth {
    fn new(
        format: CustomApiFormat,
        typed_key: Option<&str>,
        profile: Option<&NamedProviderConfig>,
        load_key: &dyn Fn(&str, &str) -> Option<String>,
    ) -> Self {
        let typed_key = typed_key.map(str::trim).filter(|key| !key.is_empty());
        let matching = profile.filter(|profile| CustomApiFormat::from(&profile.provider_type) == format);
        let (auth, header) = matching
            .filter(|profile| profile.auth != NamedProviderAuth::None || typed_key.is_none())
            .map_or_else(|| format.default_auth(), |profile| (profile.auth.clone(), profile.auth_header.clone()));
        let key = if auth == NamedProviderAuth::None {
            None
        } else {
            typed_key.map(str::to_owned).or_else(|| {
                let profile = profile?;
                profile.api_key_env.as_deref().and_then(|env| load_key(env, env_file_for(profile)))
                    .or_else(|| profile.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty()).map(str::to_owned))
            })
        };
        Self {
            format,
            auth,
            header,
            headers: matching.map(|profile| profile.headers.clone()).unwrap_or_default(),
            key,
        }
    }

    fn authenticate(&self, mut request: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if self.format == CustomApiFormat::AnthropicCompatible
            && !self.headers.keys().any(|name| name.eq_ignore_ascii_case("anthropic-version"))
        {
            request = request.header("anthropic-version", "2023-06-01");
        }
        match (&self.auth, &self.key) {
            (NamedProviderAuth::Bearer, Some(secret)) => request.bearer_auth(secret),
            (NamedProviderAuth::Header, Some(secret)) => {
                request.header(self.header.as_deref().unwrap_or(self.format.default_key_header()), secret)
            }
            _ => request,
        }
    }
}

fn fetch_model_catalog<T>(
    base_url: &str,
    auth: &CatalogAuth,
    allow_empty: bool,
    parse: impl Fn(&serde_json::Value, &mut HashSet<String>) -> Result<Vec<T>, String>,
) -> Result<(String, Vec<T>), String> {
    let exact = normalize_endpoint_url(base_url, false)?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
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
        match fetch_catalog_pages(&client, &url, auth, &parse) {
            Ok(models) if allow_empty || !models.is_empty() => return Ok((candidate, models)),
            Ok(_) => last_error = format!("{url} returned an empty model list."),
            Err(error) => last_error = format!("{url}: {error}"),
        }
    }
    Err(last_error)
}

fn fetch_catalog_pages<T>(
    client: &reqwest::blocking::Client,
    url: &str,
    auth: &CatalogAuth,
    parse: &impl Fn(&serde_json::Value, &mut HashSet<String>) -> Result<Vec<T>, String>,
) -> Result<Vec<T>, String> {
    let mut all = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let mut request = client.get(url).header("Accept", "application/json");
        if let Some(cursor) = cursor.as_deref() {
            request = request.query(&[("after_id", cursor)]);
        }
        let response = auth.authenticate(request).send().map_err(|error| error.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("GET returned HTTP {}", status.as_u16()));
        }
        let page: serde_json::Value = response.json().map_err(|_| "response was not valid JSON".to_owned())?;
        all.extend(parse(&page, &mut seen_ids)?);
        if auth.format != CustomApiFormat::AnthropicCompatible || page.get("has_more").and_then(|value| value.as_bool()) != Some(true) {
            return Ok(all);
        }
        let next = page.get("last_id").and_then(|value| value.as_str()).filter(|value| !value.is_empty());
        cursor = next.map(str::to_owned).filter(|next| seen_cursors.insert(next.clone()));
        if cursor.is_none() {
            return Err("model catalog pagination did not advance".into());
        }
    }
    Err("model catalog exceeded 100 pages".into())
}

fn catalog_model_entry(id: &str, row: &serde_json::Value) -> CatalogModelEntry {
    let positive = |field: &serde_json::Value| field.as_u64().and_then(|value| usize::try_from(value).ok()).filter(|value| *value > 0);
    let number = |keys: &[&str]| keys.iter().find_map(|key| row.get(key).and_then(positive));
    let top_provider = row.get("top_provider");
    let capability = catalog_reasoning(row);
    let (reasoning_levels, default_reasoning_effort) = match capability {
        MagpieEffort::Known { levels, default, .. } => (levels.into_iter().map(str::to_owned).collect(), default),
        _ => (Vec::new(), None),
    };
    CatalogModelEntry {
        id: id.to_owned(),
        display_name: ["display_name", "name"].iter().find_map(|key| row.get(key).and_then(|value| value.as_str())
            .map(str::trim).filter(|value| !value.is_empty() && *value != id).map(str::to_owned)),
        context_window: number(&["context_window", "context_length", "max_input_tokens"])
            .or_else(|| top_provider?.get("context_length").and_then(positive)),
        max_output_tokens: number(&["max_output_tokens", "max_completion_tokens", "max_tokens"])
            .or_else(|| top_provider?.get("max_completion_tokens").and_then(positive)),
        reasoning_levels,
        default_reasoning_effort,
    }
}

pub fn offline_catalog_models(profile: &str) -> Vec<CatalogModelEntry> {
    let prefix = if profile.trim().is_empty() { "preview" } else { profile };
    [
        ("codex/gpt-6.2-sol", "GPT-6.2 Sol", 272_000, 64_000),
        ("cpr-fast/gpt-6.2-sol", "GPT-6.2 Sol Fast", 272_000, 64_000),
        ("workbuddy/deepseek-v4.1-flash", "DeepSeek V4.1 Flash", 1_000_000, 32_768),
        ("anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6", 200_000, 64_000),
        ("anthropic/claude-opus-4-6", "Claude Opus 4.6", 200_000, 128_000),
        ("google/gemini-pro-preview", "Gemini Pro Preview", 1_000_000, 65_536),
        ("google/gemini-flash-preview", "Gemini Flash Preview", 1_000_000, 32_768),
        ("zai/glm-5.3-flash", "GLM 5.3 Flash", 200_000, 32_768),
        ("qwen/qwen-coder", "Qwen Coder", 131_072, 16_384),
        ("mistral/codestral", "Codestral", 256_000, 32_768),
        ("mini-reasoner", "Mini Reasoner", 96_000, 12_288),
        ("large-reasoner", "Large Reasoner", 512_000, 64_000),
    ].into_iter().map(|(id, name, context, output)| CatalogModelEntry {
        id: if id.contains('/') { id.to_owned() } else { format!("{prefix}/{id}") },
        display_name: Some(name.into()),
        context_window: Some(context),
        max_output_tokens: Some(output),
        reasoning_levels: ["low", "medium", "high"].into_iter().map(str::to_owned).collect(),
        default_reasoning_effort: Some("medium".into()),
    }).collect()
}

fn parse_model_catalog<T>(
    value: &serde_json::Value,
    seen: &mut HashSet<String>,
    parse: impl Fn(&str, &serde_json::Value) -> T,
) -> Result<Vec<T>, String> {
    let array = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.get("models").and_then(serde_json::Value::as_array))
        .or_else(|| value.as_array())
        .ok_or_else(|| "JSON did not contain a `data` or `models` array".to_string())?;
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
            if !clean.is_empty() && seen.insert(clean.to_string()) {
                out.push(parse(clean, item));
            }
        }
    }
    Ok(out)
}

/// Reasoning capabilities available for a custom provider model.
/// The name is retained for the existing effort picker API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MagpieEffort {
    Unknown,
    Unsupported,
    RuntimeLimited,
    Known {
        levels: Vec<&'static str>,
        fixed: bool,
        default: Option<String>,
    },
}

pub fn magpie_effort(model: &str) -> MagpieEffort {
    custom_provider_effort("magpie", model)
}

pub fn custom_provider_effort(provider: &str, model: &str) -> MagpieEffort {
    config_path().ok().map_or(MagpieEffort::Unknown, |path| {
        custom_provider_effort_at(&path, provider, model, &load_api_key_from_env_or_config)
    })
}

pub fn custom_provider_effort_at(
    path: &Path,
    provider: &str,
    model: &str,
    load_key: &dyn Fn(&str, &str) -> Option<String>,
) -> MagpieEffort {
    use MagpieEffort::*;
    let Ok((_, config)) = read_config_at(path) else { return Unknown };
    let Some(profile) = config.providers.get(provider) else { return Unknown };
    let format = CustomApiFormat::from(&profile.provider_type);
    let configured = profile.models.iter().find(|row| row.id == model)
        .or_else(|| profile.models.iter().find(|row| row.id.eq_ignore_ascii_case(model)));
    let model_support = configured.and_then(|row| row.reasoning);
    if model_support == Some(false) || (model_support.is_none() && profile.supports_reasoning_effort == Some(false)) {
        return Unsupported;
    }
    let model_default = configured.and_then(|row| row.reasoning_effort.as_deref());
    let state = read_desktop_state_at(path).map(|(_, state)| state).unwrap_or_default();
    if let Some(levels) = state.model_meta.get(provider).and_then(|meta| meta.get(model))
        .and_then(|meta| meta.reasoning_levels.as_deref())
    {
        if levels.is_empty() {
            return Unsupported;
        }
        return effort_from_levels(levels, model_default);
    }
    let cached = state.cached_catalog.get(provider).and_then(|entries| entries.iter().find(|entry| entry.id == model));
    let capability = if let Some(entry) = cached.filter(|entry| !entry.reasoning_levels.is_empty() || entry.default_reasoning_effort.is_some()) {
        effort_from_levels(&entry.reasoning_levels, entry.default_reasoning_effort.as_deref())
    } else {
        let auth = CatalogAuth::new(format, None, Some(profile), load_key);
        fetch_model_catalog(&profile.base_url, &auth, false, |page, ids| {
            parse_model_catalog(page, ids, |id, row| (id.to_owned(), catalog_reasoning(row)))
        }).ok().and_then(|(_, rows)| rows.into_iter().find(|(id, _)| id == model).map(|(_, effort)| effort)).unwrap_or(Unknown)
    };
    if let Known { levels, fixed, default } = capability {
        // This guard preserves the original Magpie behavior for endpoints that
        // advertise effort while the OpenAI-compatible runtime lacks opt-in.
        if format == CustomApiFormat::OpenAiCompatible
            && model_support != Some(true) && profile.supports_reasoning_effort != Some(true)
        {
            return RuntimeLimited;
        }
        let default = model_default.filter(|value| levels.contains(value)).map(str::to_owned).or(default);
        return Known { levels, fixed, default };
    }
    capability
}

fn effort_from_levels(levels: &[String], default: Option<&str>) -> MagpieEffort {
    if levels.is_empty() && default.is_none() {
        return MagpieEffort::Unknown;
    }
    let mut supported = Vec::new();
    for value in levels {
        if let Some(level) = MODEL_REASONING_EFFORTS.iter().copied().find(|level| level == value)
            && !supported.contains(&level)
        {
            supported.push(level);
        }
    }
    let fixed = supported.len() <= 1;
    let default = default.filter(|value| supported.contains(value) || (fixed && supported.is_empty()))
        .or_else(|| fixed.then(|| supported.first().copied()).flatten()).map(str::to_owned);
    MagpieEffort::Known { levels: supported, fixed, default }
}

fn catalog_reasoning(row: &serde_json::Value) -> MagpieEffort {
    let effort = parse_magpie_effort(row);
    if effort != MagpieEffort::Unknown {
        return effort;
    }
    let Some(capability) = row.get("capabilities").and_then(|value| value.get("effort")) else {
        return effort;
    };
    if capability.get("supported").and_then(|value| value.as_bool()) == Some(false) {
        return MagpieEffort::Unsupported;
    }
    let levels: Vec<_> = MODEL_REASONING_EFFORTS.iter().copied().filter(|level| {
        capability.get(level).and_then(|value| value.get("supported")).and_then(|value| value.as_bool()) == Some(true)
    }).map(str::to_owned).collect();
    effort_from_levels(&levels, None)
}

fn parse_magpie_effort(row: &serde_json::Value) -> MagpieEffort {
    if row.get("reasoning").and_then(|value| value.as_bool()) == Some(false) {
        return MagpieEffort::Unsupported;
    }
    let Some(values) = ["reasoning_levels", "supported_reasoning_levels", "supported_reasoning_efforts"]
        .iter()
        .find_map(|field| row.get(field).and_then(|value| value.as_array()))
    else {
        return MagpieEffort::Unknown;
    };
    let advertised: Vec<_> = values.iter().filter_map(|value| {
        value.as_str().or_else(|| value.get("effort").and_then(|value| value.as_str()))
            .map(str::trim).filter(|value| !value.is_empty())
    }).collect();
    if advertised.is_empty() {
        return MagpieEffort::Unknown;
    }
    let fixed = advertised.iter().all(|value| *value == advertised[0]);
    let mut levels = Vec::new();
    for value in &advertised {
        // This runtime omits `none` from chat completions and rejects
        // extension values such as `ultra`. Never turn those into aliases.
        if let Some(level) = MODEL_REASONING_EFFORTS.iter().copied().find(|level| level == value)
            && !levels.contains(&level)
        {
            levels.push(level);
        }
    }
    let default = ["default_reasoning_effort", "default_reasoning_level"]
        .iter().find_map(|field| row.get(field).and_then(|value| value.as_str()))
        .filter(|value| advertised.contains(value))
        .or_else(|| fixed.then_some(advertised[0]))
        .map(str::to_string);
    if levels.is_empty() && !fixed {
        return MagpieEffort::RuntimeLimited;
    }
    MagpieEffort::Known { levels, fixed, default }
}

#[cfg(test)]
#[test]
fn magpie_reasoning_distinguishes_advertised_levels_from_runtime_support() {
    use serde_json::json;
    assert_eq!(parse_magpie_effort(&json!({"reasoning": false})), MagpieEffort::Unsupported);
    assert_eq!(parse_magpie_effort(&json!({"reasoning": true})), MagpieEffort::Unknown);
    assert_eq!(parse_magpie_effort(&json!({"reasoning_levels": ["ultra"]})), MagpieEffort::Known {
        levels: vec![], fixed: true, default: Some("ultra".into()),
    });
    assert_eq!(parse_magpie_effort(&json!({
        "supported_reasoning_levels": [{"effort": "none"}, {"effort": "low"}, {"effort": "high"}, {"effort": "ultra"}],
        "default_reasoning_effort": "high",
    })), MagpieEffort::Known { levels: vec!["low", "high"], fixed: false, default: Some("high".into()) });
}

pub fn save(draft: CustomProviderDraft) -> Result<CustomProviderSummary, String> {
    save_with_reload_status(draft).map(|(summary, _)| summary)
}

pub fn save_with_reload_status(draft: CustomProviderDraft) -> Result<(CustomProviderSummary, RuntimeReloadStatus), String> {
    let path = config_path()?;
    let summary = save_at(
        &path,
        draft,
        &|env_file, env_key, secret| {
            save_named_api_key(env_file, env_key, secret).map_err(|e| e.to_string())
        },
        &|env_key, env_file| load_api_key_from_env_or_config(env_key, env_file).is_some(),
    )?;
    Ok((summary, reload_runtime_server_gracefully()))
}

pub fn save_at(
    path: &Path,
    mut draft: CustomProviderDraft,
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
    for (id, edit) in &draft.model_settings {
        if !seen.contains(id) {
            return Err(format!("Model '{id}' is missing from the model list."));
        }
        edit.validate(id)?;
    }

    let _lock = lock_provider_writes()?;
    let (original_content, existing_config) = read_config_at(path)?;
    let (raw_state, mut state) = read_desktop_state_at(path)?;
    let previous_state = state.clone();
    let editing_key = draft.original_name.as_deref().filter(|old| !old.is_empty()).unwrap_or(&name);
    let active = existing_config.providers.get(editing_key);
    let staged = state.disabled_providers.get(editing_key);
    let previous = active.or(staged);
    let enabled = active.is_some() || staged.is_none();
    if editing_key != name
        && (existing_config.providers.contains_key(&name) || state.disabled_providers.contains_key(&name))
    {
        return Err(format!("A custom provider named '{name}' already exists. Pick another name."));
    }

    let typed_key = draft.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty());
    let same_format = previous.is_some_and(|profile| CustomApiFormat::from(&profile.provider_type) == draft.format);
    let default_auth = || draft.format.default_auth();
    let (auth, auth_header, api_key_env, env_file, inline_api_key, requires_api_key) =
        if typed_key.is_some() {
            let (auth, header) = previous.filter(|profile| same_format && profile.auth != NamedProviderAuth::None)
                .map_or_else(default_auth, |profile| (profile.auth.clone(), profile.auth_header.clone()));
            (auth, header, Some(derived_api_key_env(&name)), Some(format!("provider-{name}.env")), None, Some(true))
        } else if let Some(prev) = previous {
            let (auth, header) = if same_format || prev.auth == NamedProviderAuth::None {
                (prev.auth.clone(), prev.auth_header.clone())
            } else {
                default_auth()
            };
            (auth, header, prev.api_key_env.clone(), prev.env_file.clone(), prev.api_key.clone(), prev.requires_api_key)
        } else if api_base_uses_localhost(&api_base) || looks_local_host(&api_base) {
            (NamedProviderAuth::None, None, None, None, None, Some(false))
        } else {
            return Err("Paste an API key for this remote endpoint (local 127.0.0.1 / LAN servers can omit it).".into());
        };

    let mut desktop_models = state.model_meta.remove(editing_key).unwrap_or_default();
    desktop_models.retain(|model, _| seen.contains(model));

    // Preserve rich per-model metadata (context_window, input, reasoning) for
    // models that already existed on this profile (e.g. Magpie-written entries).
    let models: Vec<NamedProviderModelConfig> = model_ids
        .into_iter()
        .map(|id| {
            let edit = draft.model_settings.remove(&id);
            let mut model = previous
                .and_then(|prev| prev.models.iter().find(|m| m.id == id))
                .cloned()
                .unwrap_or(NamedProviderModelConfig {
                    id,
                    reasoning: None,
                    reasoning_effort: None,
                    context_window: None,
                    input: Vec::new(),
                });
            if let Some(edit) = edit {
                let desktop = desktop_models.entry(model.id.clone()).or_default();
                edit.apply(&mut model, desktop);
            }
            model
        })
        .collect();

    let profile = NamedProviderConfig {
        provider_type: draft.format.into(),
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
        supports_reasoning_effort: draft.supports_reasoning_effort.or_else(|| previous.and_then(|p| p.supports_reasoning_effort)),
        disable_reasoning_heuristics: previous.is_some_and(|p| p.disable_reasoning_heuristics),
    };

    desktop_models.retain(|_, meta| *meta != DesktopModelMeta::default());
    if !desktop_models.is_empty() {
        state.model_meta.insert(name.clone(), desktop_models);
    }
    let previous_catalog = (editing_key != name)
        .then(|| state.cached_catalog.remove(editing_key)).flatten();
    if let Some(catalog) = draft.cached_catalog.or(previous_catalog) {
        state.cached_catalog.insert(name.clone(), catalog);
    }
    state.disabled_providers.remove(editing_key);
    let mut config_update = None;
    let mut is_default = false;
    if enabled {
        let mut updated = if active.is_some() {
            let spliced = remove_named_provider_sections(&original_content, editing_key);
            if validate_updated_config(&spliced).is_ok_and(|config| !config.providers.contains_key(editing_key)) {
                spliced
            } else {
                remove_active_provider(&original_content, &existing_config, editing_key)?
            }
        } else {
            original_content
        };
        if draft.set_default || existing_config.provider.default_provider.as_deref() == Some(editing_key) {
            updated = upsert_provider_defaults(updated, &name, &default_model);
        }
        updated = append_profile_section(updated, &name, &profile)?;
        let parsed = validate_updated_config(&updated)?;
        if parsed.providers.get(&name) != Some(&profile)
            || (editing_key != name && parsed.providers.contains_key(editing_key))
        {
            return Err("Could not update the named provider section in config.toml.".into());
        }
        is_default = parsed.provider.default_provider.as_deref() == Some(&name);
        config_update = Some(updated);
    } else {
        state.disabled_providers.insert(name.clone(), profile.clone());
    }
    if let Some(secret) = typed_key {
        store_key(env_file_for(&profile), profile.api_key_env.as_deref().expect("typed key has an env name"), secret)?;
    }
    write_provider_state_at(path, config_update.as_deref(), raw_state.as_deref(), &state, state != previous_state)?;
    drop(_lock);
    Ok(summary_from_config(
        &name,
        &profile,
        is_default.then_some(name.as_str()),
        enabled,
        has_stored_key,
    ))
}

pub fn set_enabled_with_reload_status(name: &str, enabled: bool) -> Result<RuntimeReloadStatus, String> {
    set_enabled_at(&config_path()?, name, enabled)?;
    Ok(reload_runtime_server_gracefully())
}

pub fn set_enabled_at(path: &Path, name: &str, enabled: bool) -> Result<(), String> {
    let _lock = lock_provider_writes()?;
    let (content, config) = read_config_at(path)?;
    let (raw_state, mut state) = read_desktop_state_at(path)?;
    let active = config.providers.get(name);
    let staged = state.disabled_providers.get(name);
    if enabled {
        if active.is_none() {
            let profile = staged.ok_or_else(|| format!("Custom provider '{name}' was not found."))?;
            let updated = append_profile_section(content, name, profile)?;
            let parsed = validate_updated_config(&updated)?;
            if parsed.providers.get(name) != Some(profile) {
                return Err("Could not restore the named provider in config.toml.".into());
            }
            // Keep the disabled copy recoverable until the active config is durable.
            write_atomic(path, updated.as_bytes())?;
        }
        if state.disabled_providers.remove(name).is_some() {
            write_desktop_state_at(path, &state)?;
        }
    } else if let Some(profile) = active {
        let updated = remove_active_provider(&content, &config, name)?;
        state.disabled_providers.insert(name.to_owned(), profile.clone());
        write_provider_state_at(path, Some(&updated), raw_state.as_deref(), &state, true)?;
    } else if staged.is_none() {
        return Err(format!("Custom provider '{name}' was not found."));
    }
    Ok(())
}

fn validate_updated_config(content: &str) -> Result<Config, String> {
    parse_config_or_default(content)
        .map_err(|error| format!("Updated config.toml failed validation: {error}"))
}

fn remove_active_provider(content: &str, config: &Config, name: &str) -> Result<String, String> {
    let was_default = config.provider.default_provider.as_deref() == Some(name);
    let mut updated = remove_named_provider_sections(content, name);
    if was_default {
        updated = clear_provider_defaults(updated);
    }
    if let Ok(parsed) = validate_updated_config(&updated)
        && !parsed.providers.contains_key(name)
        && (!was_default || parsed.provider.default_provider.as_deref() != Some(name))
    {
        return Ok(updated);
    }
    // Dotted keys and inline provider tables have no section to splice.
    let mut document: toml::Table = toml::from_str(content).map_err(|error| error.to_string())?;
    if let Some(providers) = document.get_mut("providers").and_then(toml::Value::as_table_mut) {
        providers.remove(name);
    }
    if was_default
        && let Some(defaults) = document.get_mut("provider").and_then(toml::Value::as_table_mut)
    {
        defaults.remove("default_provider");
        defaults.remove("default_model");
    }
    let updated = toml::to_string_pretty(&document).map_err(|error| error.to_string())?;
    validate_updated_config(&updated)?;
    Ok(updated)
}

pub fn make_default(name: &str) -> Result<Option<String>, String> {
    make_default_with_reload_status(name).map(|(spec, _)| spec)
}

pub fn make_default_with_reload_status(name: &str) -> Result<(Option<String>, RuntimeReloadStatus), String> {
    let path = config_path()?;
    let spec = make_default_at(&path, name)?;
    Ok((spec, reload_runtime_server_gracefully()))
}

pub fn make_default_at(path: &Path, name: &str) -> Result<Option<String>, String> {
    let _lock = lock_provider_writes()?;
    let (content, config) = read_config_at(path)?;
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
    remove_with_reload_status(name).map(|_| ())
}

pub fn remove_with_reload_status(name: &str) -> Result<RuntimeReloadStatus, String> {
    let path = config_path()?;
    remove_at(&path, name)?;
    Ok(reload_runtime_server_gracefully())
}

pub fn remove_at(path: &Path, name: &str) -> Result<(), String> {
    let _lock = lock_provider_writes()?;
    let (content, config) = read_config_at(path)?;
    let (raw_state, mut state) = read_desktop_state_at(path)?;
    let updated = config.providers.contains_key(name)
        .then(|| remove_active_provider(&content, &config, name)).transpose()?;
    let mut state_changed = state.disabled_providers.remove(name).is_some();
    state_changed |= state.cached_catalog.remove(name).is_some();
    state_changed |= state.model_meta.remove(name).is_some();
    write_provider_state_at(path, updated.as_deref(), raw_state.as_deref(), &state, state_changed)
}

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeReloadStatus {
    NotRunning,
    /// The reload command succeeded, including when the daemon was already current.
    Reloaded,
    Deferred,
}

const GRACEFUL_RELOAD_ARGS: [&str; 3] = ["server", "reload", "--quiet"];

pub fn reload_runtime_server() {
    reload_runtime_server_gracefully();
}

/// Ask the shared daemon to reload when safe, and refresh desktop account data.
pub fn reload_runtime_server_gracefully() -> RuntimeReloadStatus {
    let legacy = jcode_sdk::api::legacy_socket_path();
    let status = if jcode_sdk::socket_accepts(&legacy) {
        let jcode = crate::platform::companion_executable("jcode");
        let mut command = std::process::Command::new(&jcode);
        crate::platform::no_console_window(&mut command);
        let result = command
            .args(GRACEFUL_RELOAD_ARGS)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if result.is_ok_and(|status| status.success()) {
            RuntimeReloadStatus::Reloaded
        } else {
            RuntimeReloadStatus::Deferred
        }
    } else {
        RuntimeReloadStatus::NotRunning
    };
    crate::accounts::request_refresh();
    status
}

#[cfg(test)]
#[test]
fn custom_provider_reload_uses_non_forcing_args() {
    assert_eq!(GRACEFUL_RELOAD_ARGS, ["server", "reload", "--quiet"]);
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
        Ok(())
    };
    write().map_err(|error| format!("Could not write {}: {error}", path.display()))
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
    fn unconfigured_presets_only_drop_uncredentialed_route_methods() {
        let deepseek = openai_compatible_profiles()
            .iter()
            .find(|profile| profile.id == "deepseek")
            .copied()
            .expect("deepseek preset");
        let key_env = resolve_openai_compatible_profile(deepseek).api_key_env;

        // No credentials anywhere: key-required presets are dropped.
        let set = unconfigured_preset_route_methods_with(&HashSet::new(), |_, _| None);
        assert!(set.contains("openai-compatible:deepseek"));
        assert!(set.contains("openai-compatible:ollama"));

        // A preset holding its API key stays.
        let keyed = |var: &str, _: &str| (var == key_env).then(|| "sk".to_string());
        let set = unconfigured_preset_route_methods_with(&HashSet::new(), keyed);
        assert!(!set.contains("openai-compatible:deepseek"));
        assert!(set.contains("openai-compatible:ollama"));

        // The local no-auth rule keeps no-key presets but not key-required ones.
        let local = |var: &str, _: &str| {
            (var == OPENAI_COMPAT_LOCAL_ENABLED_ENV).then(|| "true".to_string())
        };
        let set = unconfigured_preset_route_methods_with(&HashSet::new(), local);
        assert!(!set.contains("openai-compatible:ollama"));
        assert!(set.contains("openai-compatible:deepseek"));

        // A user profile reusing a preset id is never treated as the preset.
        let named: HashSet<String> = ["deepseek".to_string()].into_iter().collect();
        let set = unconfigured_preset_route_methods_with(&named, |_, _| None);
        assert!(!set.contains("openai-compatible:deepseek"));
        assert!(set.contains("openai-compatible:ollama"));
    }

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
                model_settings: Default::default(),
                set_default: true,
                model_catalog: true,
                ..Default::default()
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
                model_settings: Default::default(),
                set_default: true,
                model_catalog: false,
                ..Default::default()
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
    fn configured_model_metadata_reads_profile_model_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"[providers.magpie]
base_url = "https://magpie.example.com/v1"
default_model = "codex/gpt-6.2-sol"
models = [
  { id = "codex/gpt-6.2-sol", context_window = 272000, reasoning = true, input = ["text", "image"] },
  { id = "truesota/deepseek-flash", input = ["text"] },
  { id = "workbuddy/deepseek-v4.1-flash" },
]
"#,
        )
        .unwrap();

        let meta = configured_model_metadata_at(&path);
        assert_eq!(meta.len(), 3);
        let codex = &meta[&("magpie".to_string(), "codex/gpt-6.2-sol".to_string())];
        assert_eq!(codex.context_window, Some(272_000));
        assert_eq!(codex.image_input, Some(true));
        assert_eq!(codex.reasoning, Some(true));
        let flash = &meta[&("magpie".to_string(), "truesota/deepseek-flash".to_string())];
        assert_eq!(flash.context_window, None);
        assert_eq!(flash.image_input, Some(false));
        assert_eq!(flash.reasoning, None);
        assert_eq!(
            meta[&("magpie".to_string(), "workbuddy/deepseek-v4.1-flash".to_string())],
            ConfiguredModelMeta::default()
        );
        assert!(
            configured_model_metadata_at(&temp.path().join("missing.toml")).is_empty()
        );
    }

    #[test]
    fn model_settings_roundtrip_updates_two_exact_ids_and_preserves_profile() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let fixture = r#"
[provider]
default_provider = "other"
default_model = "shared"

[providers.other]
base_url = "http://127.0.0.1:9010/v1"
models = [{ id = "Pool/Model", context_window = 8192 }]

[providers.gateway-test]
base_url = "https://gateway.example.test/v1"
auth = "header"
auth_header = "X-Custom-Key"
api_key_env = "GATEWAY_TEST_KEY"
env_file = "gateway-test.env"
headers = { "X-Test-App" = "desktop" }
default_model = "Pool/Model"
extra_body = { service_tier = "priority" }
supports_reasoning_effort = true
model_catalog = true
models = [
  { id = "Pool/Model", context_window = 272000, reasoning = true, reasoning_effort = "high", input = ["text", "image"] },
  { id = "pool/model", reasoning_effort = "ultra", input = ["text", "image", "audio"] },
  { id = "untouched", context_window = 8192, reasoning = false, reasoning_effort = "none", input = ["audio"] },
]
"#;
        std::fs::write(&path, fixture).unwrap();
        let initial = model_settings_from_config(fixture, "gateway-test");
        assert_eq!(initial["Pool/Model"].context_window, Some(272_000));
        assert_eq!(initial["pool/model"].reasoning_effort.as_deref(), Some("ultra"));
        let other = model_settings_from_config(fixture, "other");
        assert_eq!(other["Pool/Model"].context_window, Some(8_192));
        assert!(model_settings_from_config(fixture, "Gateway-Test").is_empty());

        let draft = CustomProviderDraft {
            original_name: Some("gateway-test".into()),
            name: "gateway-test".into(),
            base_url: "https://gateway.example.test/v1".into(),
            default_model: "Pool/Model".into(),
            models: vec!["pool/model".into(), "untouched".into()],
            model_catalog: true,
            model_settings: [
                ("Pool/Model".into(), ModelSettingsEdit {
                    context_window: Some(None),
                    reasoning: Some(None),
                    reasoning_effort: Some(None),
                    image_input: Some(None),
                    ..Default::default()
                }),
                ("pool/model".into(), ModelSettingsEdit {
                    context_window: Some(Some(196_607)),
                    reasoning: Some(Some(false)),
                    image_input: Some(Some(false)),
                    ..ModelSettingsEdit::default()
                }),
            ].into(),
            ..CustomProviderDraft::default()
        };
        save_at(&path, draft, &|_, _, _| panic!("key write"), &|_, _| true).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        let settings = model_settings_from_config(&saved, "gateway-test");
        assert_eq!(settings["Pool/Model"], ModelSettings::default());
        assert_eq!(settings["pool/model"], ModelSettings {
            context_window: Some(196_607),
            reasoning: Some(false),
            reasoning_effort: Some("ultra".into()),
            image_input: Some(false),
            pdf_input: Some(false),
            ..Default::default()
        });
        assert_eq!(settings["untouched"], initial["untouched"]);
        assert_eq!(model_settings_from_config(&saved, "other"), other);

        let before = parse_config_or_default(fixture).unwrap();
        let after = parse_config_or_default(&saved).unwrap();
        let previous = &before.providers["gateway-test"];
        let profile = &after.providers["gateway-test"];
        assert!(matches!(&profile.auth, NamedProviderAuth::Header));
        assert_eq!(profile.auth_header, previous.auth_header);
        assert_eq!(profile.api_key_env, previous.api_key_env);
        assert_eq!(profile.env_file, previous.env_file);
        assert_eq!(profile.headers, previous.headers);
        assert_eq!(profile.extra_body, previous.extra_body);
        assert_eq!(profile.supports_reasoning_effort, previous.supports_reasoning_effort);
        assert_eq!(profile.default_model, previous.default_model);
        assert_eq!(profile.models[1].input, ["text", "audio"]);
        assert_eq!(profile.models[2].input, ["audio"]);
        assert_eq!(after.provider.default_provider, before.provider.default_provider);
        assert_eq!(after.provider.default_model, before.provider.default_model);
    }

    #[test]
    fn invalid_model_settings_leave_config_and_credentials_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let original = "# keep this config\n";
        std::fs::write(&path, original).unwrap();
        for (edit, field) in [
            (ModelSettingsEdit { context_window: Some(Some(0)), ..Default::default() }, "context_window"),
            (ModelSettingsEdit { max_output_tokens: Some(Some(0)), ..Default::default() }, "max_output_tokens"),
            (ModelSettingsEdit { reasoning_levels: Some(Some(vec!["ultra".into()])), ..Default::default() }, "reasoning_levels"),
            (ModelSettingsEdit { reasoning_effort: Some(Some("none".into())), ..Default::default() }, "reasoning_effort"),
            (ModelSettingsEdit { reasoning_effort: Some(Some("ultra".into())), ..Default::default() }, "reasoning_effort"),
        ] {
            let draft = CustomProviderDraft {
                name: "gateway-test".into(),
                base_url: "https://gateway.example.test/v1".into(),
                api_key: Some("fixture-key".into()),
                default_model: "Pool/Model".into(),
                model_settings: [("Pool/Model".into(), edit)].into(),
                ..CustomProviderDraft::default()
            };
            let error = save_at(&path, draft, &|_, _, _| panic!("key write"), &|_, _| false)
                .unwrap_err();
            assert!(error.contains("Pool/Model") && error.contains(field), "{error}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert!(!path.with_file_name(DESKTOP_PROVIDERS_FILE).exists());
        }
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

    fn service_draft(name: &str) -> CustomProviderDraft {
        CustomProviderDraft {
            name: name.into(),
            base_url: "http://127.0.0.1:3425/v1".into(),
            default_model: "sample/model".into(),
            ..Default::default()
        }
    }

    fn fixture_key<'a>(root: &'a Path) -> impl Fn(&str, &str, &str) -> Result<(), String> + 'a {
        move |file, env, key| std::fs::write(root.join(file), format!("{env}={key}\n"))
            .map_err(|error| error.to_string())
    }

    fn test_config(path: &Path) -> Config {
        read_config_at(path).unwrap().1
    }

    #[test]
    fn custom_api_formats_roundtrip_with_native_authentication() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let check = |_: &str, file: &str| temp.path().join(file).is_file();
        for (name, format, display, auth, header) in [
            ("fixture-chat", CustomApiFormat::OpenAiCompatible, "OpenAI Chat Completions", NamedProviderAuth::Bearer, None),
            ("fixture-messages", CustomApiFormat::AnthropicCompatible, "Anthropic Messages", NamedProviderAuth::Header, Some("x-api-key")),
            ("fixture-router", CustomApiFormat::OpenRouter, "OpenRouter", NamedProviderAuth::Bearer, None),
        ] {
            let saved = save_at(&path, CustomProviderDraft {
                format,
                api_key: Some("fixture-secret".into()),
                supports_reasoning_effort: Some(false),
                ..service_draft(name)
            }, &fixture_key(temp.path()), &check).unwrap();
            assert_eq!(saved.format, format);
            assert_eq!(saved.host_summary(), "127.0.0.1:3425");
            assert!(saved.enabled && saved.has_api_key);
            assert_eq!(format.display_name(), display);
            let config = test_config(&path);
            let native = &config.providers[name];
            assert_eq!(native.provider_type, format.into());
            assert_eq!(native.auth, auth);
            assert_eq!(native.auth_header.as_deref(), header);
            assert_eq!(native.supports_reasoning_effort, Some(false));
            assert_eq!(native.api_key_env.as_deref(), Some(derived_api_key_env(name).as_str()));
            assert_eq!(native.env_file.as_deref(), Some(format!("provider-{name}.env").as_str()));
            assert!(!native.model_catalog);
            assert!(native.api_key.is_none());
            assert_eq!(list_at(&path, &check).into_iter().find(|row| row.name == name).unwrap(), saved);
        }
        assert!(!std::fs::read_to_string(&path).unwrap().contains("fixture-secret"));
    }

    #[test]
    fn disable_enable_preserves_full_service_and_private_key() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let check = |_: &str, file: &str| temp.path().join(file).is_file();
        let name = "fixture-service";
        save_at(&path, CustomProviderDraft {
            format: CustomApiFormat::AnthropicCompatible,
            api_key: Some("fixture-secret".into()),
            set_default: true,
            model_settings: [("sample/model".into(), ModelSettingsEdit {
                context_window: Some(Some(200_000)),
                reasoning: Some(Some(true)),
                pdf_input: Some(Some(true)),
                ..Default::default()
            })].into(),
            ..service_draft(name)
        }, &fixture_key(temp.path()), &check).unwrap();
        let config = test_config(&path);
        let key_path = temp.path().join("provider-fixture-service.env");
        let key_bytes = std::fs::read(&key_path).unwrap();
        assert_eq!(config.provider.default_provider.as_deref(), Some(name));
        set_enabled_at(&path, name, false).unwrap();
        set_enabled_at(&path, name, false).unwrap();
        let disabled = test_config(&path);
        assert!(!disabled.providers.contains_key(name));
        assert!(disabled.provider.default_provider.is_none());
        assert!(disabled.provider.default_model.is_none());
        let summary = list_at(&path, &check).pop().unwrap();
        assert!(!summary.enabled && !summary.is_default && summary.has_api_key);
        assert_eq!(summary.format, CustomApiFormat::AnthropicCompatible);
        assert_eq!(fetch_profile_at(&path, name), Some(config.providers[name].clone()));
        assert!(!std::fs::read_to_string(path.with_file_name(DESKTOP_PROVIDERS_FILE)).unwrap().contains("fixture-secret"));
        assert_eq!(provider_model_settings_at(&path, name)["sample/model"].pdf_input, Some(true));
        assert!(make_default_at(&path, name).is_err());
        set_enabled_at(&path, name, true).unwrap();
        set_enabled_at(&path, name, true).unwrap();
        assert_eq!(test_config(&path).providers[name], config.providers[name]);
        assert!(list_at(&path, &check)[0].enabled);
        assert!(read_desktop_state_at(&path).unwrap().1.disabled_providers.is_empty());
        assert_eq!(std::fs::read(&key_path).unwrap(), key_bytes);
    }

    #[test]
    fn candidate_catalog_preserves_unselected_models() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let candidates: Vec<_> = offline_catalog_models("fixture-service").into_iter().take(10).collect();
        assert_eq!(candidates.len(), 10);
        assert!(candidates.iter().all(|entry| entry.display_name.is_some()
            && entry.context_window.is_some() && entry.max_output_tokens.is_some()
            && !entry.reasoning_levels.is_empty()));
        let selected: Vec<_> = candidates.iter().take(3).map(|entry| entry.id.clone()).collect();
        let name = "fixture-service";
        save_cached_catalog_at(&path, name, candidates.clone()).unwrap();
        save_at(&path, CustomProviderDraft {
            default_model: selected[0].clone(),
            models: selected.clone(),
            ..service_draft(name)
        }, &|_, _, _| unreachable!(), &|_, _| false).unwrap();
        let config = test_config(&path);
        let profile = &config.providers[name];
        assert!(!profile.model_catalog);
        assert_eq!(profile.models.iter().map(|row| row.id.clone()).collect::<Vec<_>>(), selected);
        assert_eq!(cached_catalog_at(&path, name), candidates);
        let updated = candidates[1..].to_vec();
        save_at(&path, CustomProviderDraft {
            original_name: Some(name.into()),
            default_model: selected[0].clone(),
            models: selected,
            cached_catalog: Some(updated.clone()),
            model_catalog: true,
            ..service_draft(name)
        }, &|_, _, _| unreachable!(), &|_, _| false).unwrap();
        assert!(test_config(&path).providers[name].model_catalog);
        assert_eq!(cached_catalog_at(&path, name), updated);
    }

    #[test]
    fn extended_model_settings_migrate_and_clear_on_disabled_rename_and_delete() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let name = "fixture-service";
        let renamed = "fixture-renamed";
        let model = "sample/model";
        let catalog = offline_catalog_models(name);
        let save = |draft| save_at(&path, draft, &|_, _, _| unreachable!(), &|_, _| false).unwrap();
        save(CustomProviderDraft {
            cached_catalog: Some(catalog.clone()),
            model_settings: [(model.into(), ModelSettingsEdit {
                context_window: Some(Some(270_000)),
                reasoning: Some(Some(true)),
                reasoning_effort: Some(Some("high".into())),
                image_input: Some(Some(true)),
                pdf_input: Some(Some(true)),
                max_output_tokens: Some(Some(48_000)),
                alias: Some(Some("Personal model".into())),
                reasoning_levels: Some(Some(vec!["low".into(), "high".into()])),
                subagent_schedulable: Some(Some(false)),
            })].into(),
            ..service_draft(name)
        });
        let config = test_config(&path);
        assert_eq!(config.providers[name].models[0].input, ["text", "image", "pdf"]);
        let settings = ModelSettings {
            context_window: Some(270_000), reasoning: Some(true), reasoning_effort: Some("high".into()),
            image_input: Some(true), pdf_input: Some(true), max_output_tokens: Some(48_000),
            alias: Some("Personal model".into()), reasoning_levels: Some(vec!["low".into(), "high".into()]),
            subagent_schedulable: Some(false),
        };
        assert_eq!(provider_model_settings_at(&path, name)[model], settings);
        assert_eq!(configured_model_metadata_at(&path)[&(name.into(), model.into())], settings);
        assert_eq!(custom_provider_effort_at(&path, name, model, &|_, _| unreachable!()), MagpieEffort::Known {
            levels: vec!["low", "high"], fixed: false, default: Some("high".into()),
        });
        set_enabled_at(&path, name, false).unwrap();
        let summary = save(CustomProviderDraft { original_name: Some(name.into()), ..service_draft(renamed) });
        assert!(!summary.enabled);
        assert!(cached_catalog_at(&path, name).is_empty());
        assert_eq!(cached_catalog_at(&path, renamed), catalog);
        assert_eq!(provider_model_settings_at(&path, renamed)[model], settings);
        assert!(provider_model_settings_at(&path, name).is_empty());
        assert!(configured_model_metadata_at(&path).is_empty());
        set_enabled_at(&path, renamed, true).unwrap();
        save(CustomProviderDraft {
            original_name: Some(renamed.into()),
            model_settings: [(model.into(), ModelSettingsEdit {
                image_input: Some(None), alias: Some(None), ..Default::default()
            })].into(),
            ..service_draft(renamed)
        });
        assert_eq!(test_config(&path).providers[renamed].models[0].input, ["text", "pdf"]);
        assert_eq!(provider_model_settings_at(&path, renamed)[model].alias, None);
        remove_at(&path, renamed).unwrap();
        assert!(list_at(&path, &|_, _| false).is_empty());
        assert!(read_desktop_state_at(&path).unwrap().1 == DesktopProviderState::default());
    }

    #[test]
    fn rich_catalog_and_connection_report_use_native_headers_and_complete_pages() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let fixture = [
                ("GET /models ", "x-api-key: fixture-anthropic", 404, ""),
                ("GET /v1/models ", "x-api-key: fixture-anthropic", 200,
                    r#"{"data":[{"id":"a","display_name":"Model A","max_input_tokens":200000,"max_tokens":64000,"capabilities":{"effort":{"supported":true,"low":{"supported":true},"high":{"supported":true}}}}],"has_more":true,"last_id":"a"}"#),
                ("GET /v1/models?after_id=a ", "x-api-key: fixture-anthropic", 200,
                    r#"{"data":[{"id":"a"},{"id":"b","display_name":"Model B","reasoning_levels":["low","high"],"default_reasoning_effort":"high"}],"has_more":false}"#),
                ("GET /v1/models ", "authorization: bearer fixture-router", 200,
                    r#"{"data":[{"id":"router/a","name":"Router A","context_length":128000,"top_provider":{"max_completion_tokens":16000}}]}"#),
                ("GET /v1/models ", "x-api-key: fixture-anthropic", 200, r#"{"data":[],"has_more":false}"#),
            ];
            for (request_line, header, status, body) in fixture {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 4096);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(request_line), "{request}");
                let headers = request.to_ascii_lowercase();
                assert!(headers.contains(header), "{headers}");
                if header.starts_with("x-api-key") {
                    assert!(headers.contains("anthropic-version: 2023-06-01\r\n"), "{headers}");
                }
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let origin = format!("http://{addr}");
        let (working, catalog) = fetch_model_catalog_rich(&origin, Some("fixture-anthropic"), None, CustomApiFormat::AnthropicCompatible).unwrap();
        assert_eq!(working, format!("{origin}/v1"));
        assert_eq!(catalog.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(catalog[0].display_name.as_deref(), Some("Model A"));
        assert_eq!(catalog[0].context_window, Some(200_000));
        assert_eq!(catalog[0].max_output_tokens, Some(64_000));
        assert_eq!(catalog[0].reasoning_levels, ["low", "high"]);
        assert_eq!(catalog[1].default_reasoning_effort.as_deref(), Some("high"));
        let report = test_connection(&working, Some("fixture-router"), None, CustomApiFormat::OpenRouter).unwrap();
        assert_eq!(report.working_base_url, working);
        assert_eq!(report.model_count, 1);
        let empty = test_connection(&working, Some("fixture-anthropic"), None, CustomApiFormat::AnthropicCompatible).unwrap();
        assert_eq!(empty.model_count, 0);
        server.join().unwrap();
    }

    #[test]
    fn stored_anthropic_catalog_auth_uses_profile_header_and_key_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, r#"
[providers.fixture]
type = "anthropic-compatible"
base_url = "http://127.0.0.1:3425/v1"
auth = "header"
auth_header = "x-fixture-key"
api_key_env = "FIXTURE_API_KEY"
env_file = "fixture.env"
"#).unwrap();
        let profile = fetch_profile_at(&path, "fixture").unwrap();
        let auth = CatalogAuth::new(CustomApiFormat::AnthropicCompatible, None, Some(&profile), &|name, file| {
            assert_eq!(name, "FIXTURE_API_KEY");
            assert_eq!(file, "fixture.env");
            Some("fixture-stored-key".into())
        });
        let req = auth.authenticate(reqwest::blocking::Client::new().get(&profile.base_url)).build().unwrap();
        assert_eq!(req.headers()["x-fixture-key"], "fixture-stored-key");
        assert_eq!(req.headers()["anthropic-version"], "2023-06-01");
    }

    #[test]
    fn invalid_sidecar_cannot_discard_active_provider_or_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let name = "fixture-service";
        save_at(
            &path,
            CustomProviderDraft { api_key: Some("fixture-key".into()), ..service_draft(name) },
            &fixture_key(temp.path()),
            &|_, _| true,
        ).unwrap();
        let config = std::fs::read(&path).unwrap();
        let key = temp.path().join("provider-fixture-service.env");
        let key_bytes = std::fs::read(&key).unwrap();
        let sidecar = path.with_file_name(DESKTOP_PROVIDERS_FILE);
        std::fs::write(&sidecar, b"{ invalid JSON").unwrap();
        assert!(set_enabled_at(&path, name, false).unwrap_err().contains(DESKTOP_PROVIDERS_FILE));
        assert!(save_cached_catalog_at(&path, name, Vec::new()).is_err());
        assert!(list_at(&path, &|_, _| true)[0].enabled);
        assert_eq!(std::fs::read(&path).unwrap(), config);
        assert_eq!(std::fs::read(&key).unwrap(), key_bytes);
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"{ invalid JSON");
    }

    #[test]
    fn cached_effort_respects_native_support_and_configured_default() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let name = "fixture-service";
        let model = "sample/model";
        let save = |draft| save_at(&path, draft, &|_, _, _| unreachable!(), &|_, _| false).unwrap();
        save(CustomProviderDraft {
            cached_catalog: Some(vec![CatalogModelEntry {
                id: model.into(),
                reasoning_levels: vec!["low".into(), "high".into()],
                default_reasoning_effort: Some("low".into()),
                ..Default::default()
            }]),
            ..service_draft(name)
        });
        let effort = || custom_provider_effort_at(&path, name, model, &|_, _| unreachable!());
        assert_eq!(effort(), MagpieEffort::RuntimeLimited);
        save(CustomProviderDraft {
            supports_reasoning_effort: Some(true),
            model_settings: [(model.into(), ModelSettingsEdit {
                reasoning_effort: Some(Some("high".into())),
                ..Default::default()
            })].into(),
            ..service_draft(name)
        });
        assert_eq!(effort(), MagpieEffort::Known {
            levels: vec!["low", "high"], fixed: false, default: Some("high".into()),
        });
        save(CustomProviderDraft {
            model_settings: [(model.into(), ModelSettingsEdit {
                reasoning: Some(Some(false)), ..Default::default()
            })].into(),
            ..service_draft(name)
        });
        assert_eq!(effort(), MagpieEffort::Unsupported);
    }
}
