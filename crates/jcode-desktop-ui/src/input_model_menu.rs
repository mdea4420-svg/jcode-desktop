//! Model picker metadata comes from the daemon, including shared TUI preferences.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use jcode_sdk::{ModelRouteInfo, ModelUsage, compare_model_usage};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelDetails {
    pub model: String,
    pub recommended: bool,
    pub provider: String,
    pub api_method: String,
    pub usage: Option<ModelUsage>,
}

/// Route identity: everything that decides its spec and recommendation.
/// Usage is deliberately absent, since usage ticks are what rebroadcast.
type RouteKey = (String, String, String, String);

thread_local! {
    static ROUTE_SPECS: std::cell::RefCell<HashMap<RouteKey, (String, bool)>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Spec and recommendation, memoized. Every model usage tick resends the
/// full catalog, and re-deriving routing policy for each route was a steady
/// share of UI-thread time in live profiles. Bounded so catalog churn cannot
/// grow it without limit.
fn route_identity(route: &ModelRouteInfo) -> (String, bool) {
    const MAX_ENTRIES: usize = 8192;
    let key = (
        route.model.clone(),
        route.provider.clone(),
        route.api_method.clone(),
        route.detail.clone(),
    );
    ROUTE_SPECS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
        let value = (
            route_spec(route),
            jcode_provider_core::model_route_metadata_is_recommended(
                jcode_provider_core::explicit_model_provider_prefix(&route.model)
                    .map_or(route.model.as_str(), |(_, _, bare)| bare),
                &route.provider,
                &route.api_method,
                true,
            ),
        );
        if cache.len() >= MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(key, value.clone());
        value
    })
}

pub(crate) fn from_routes(routes: &[ModelRouteInfo]) -> HashMap<String, ModelDetails> {
    let mut details = HashMap::<String, ModelDetails>::with_capacity(routes.len());
    for route in routes.iter().filter(|route| route.available) {
        let (spec, recommended) = route_identity(route);
        let candidate = ModelDetails {
            model: route.model.clone(),
            recommended,
            provider: route.provider.clone(),
            api_method: route.api_method.clone(),
            usage: route.usage.clone(),
        };
        match details.get(&spec) {
            Some(existing)
                if !compare_model_usage(candidate.usage.as_ref(), existing.usage.as_ref())
                    .is_lt() => {}
            _ => {
                details.insert(spec, candidate);
            }
        }
    }
    details
}

/// Use the same routing policy as the daemon and TUI, never infer auth from a label.
fn route_spec(route: &ModelRouteInfo) -> String {
    use jcode_provider_core::{ModelRoute, RouteSelection, explicit_model_provider_prefix};
    let bare = explicit_model_provider_prefix(&route.model)
        .map_or(route.model.as_str(), |(_, _, bare)| bare);
    let mut selection = RouteSelection::from_model_route(&ModelRoute {
        model: bare.to_string(),
        provider: route.provider.clone(),
        api_method: route.api_method.clone(),
        available: route.available,
        detail: route.detail.clone(),
        cheapness: None,
        usage: route.usage.clone(),
    });
    let spec = selection.routed_model_spec();
    selection.model.clear();
    let prefix = selection.routed_model_spec();
    if !prefix.is_empty() && bare.starts_with(&prefix) {
        bare.to_string()
    } else {
        spec
    }
}

pub(crate) fn current_spec<'a>(
    current: Option<&str>,
    models: &'a [String],
    details: &HashMap<String, ModelDetails>,
) -> Option<&'a str> {
    let current = current?;
    models
        .iter()
        .find(|model| model.as_str() == current)
        .or_else(|| {
            models
                .iter()
                .find(|model| details.get(*model).is_some_and(|d| d.model == current))
        })
        .map(String::as_str)
}

/// Order routes by usage, then recommendation, then spec.
///
/// Every usage tick rebroadcasts the whole catalog to every panel, so this
/// runs often on the UI thread. Looking each route up once, instead of four
/// SipHash lookups per comparison, took it from the top live-profile hotspot
/// to noise.
pub(crate) fn rank(models: &mut Vec<String>, details: &HashMap<String, ModelDetails>) {
    let mut keyed: Vec<_> = std::mem::take(models)
        .into_iter()
        .map(|model| {
            let detail = details.get(&model);
            let usage = detail.and_then(|detail| detail.usage.as_ref());
            let recommended = detail.is_some_and(|detail| detail.recommended);
            (usage, recommended, model)
        })
        .collect();
    keyed.sort_by(|(a_usage, a_rec, a), (b_usage, b_rec, b)| {
        compare_model_usage(*a_usage, *b_usage)
            .then_with(|| b_rec.cmp(a_rec))
            .then_with(|| a.cmp(b))
    });
    models.extend(keyed.into_iter().map(|(_, _, model)| model));
}

pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn relative_time(timestamp: u64, now: u64) -> String {
    let age = now.saturating_sub(timestamp);
    match age {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", age / 60),
        3600..86400 => format!("{}h ago", age / 3600),
        _ => format!("{}d ago", age / 86400),
    }
}

/// How a route authenticates, phrased and iconed for the group header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthKind {
    /// Signed in with a provider account (ChatGPT, Claude, GitHub, Google).
    Account,
    ApiKey,
    Subscription,
    CloudCredentials,
    Other,
}

impl AuthKind {
    pub(crate) fn of(api_method: &str) -> Self {
        use jcode_provider_core::ModelRouteApiMethod as M;
        match M::parse(api_method) {
            M::ClaudeOAuth
            | M::OpenAIOAuth
            | M::CodeAssistOAuth
            | M::Copilot
            | M::Cursor
            | M::AntigravityHttps
            | M::GrokBuild => Self::Account,
            M::AnthropicApiKey | M::OpenAIApiKey | M::OpenRouter | M::OpenAiCompatible { .. } => {
                Self::ApiKey
            }
            M::JcodeSubscription => Self::Subscription,
            M::Bedrock => Self::CloudCredentials,
            M::Other(method) => {
                let method = method.to_ascii_lowercase();
                if method.contains("oauth") {
                    Self::Account
                } else if method.contains("key") || method.ends_with("-api") {
                    Self::ApiKey
                } else {
                    Self::Other
                }
            }
            M::RemoteCatalog | M::Current => Self::Other,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Account => "Signed in",
            Self::ApiKey => "API key",
            Self::Subscription => "Subscription",
            Self::CloudCredentials => "Cloud credentials",
            Self::Other => "Connected",
        }
    }
}

/// Group header parts: provider brand, its logo id, and the auth method.
pub(crate) struct HeaderParts {
    pub provider: String,
    pub logo: &'static str,
    pub auth: AuthKind,
}

pub(crate) fn header_parts(model: &str, details: &HashMap<String, ModelDetails>) -> HeaderParts {
    let Some(detail) = details.get(model) else {
        return HeaderParts {
            provider: "Other models".into(),
            logo: "",
            auth: AuthKind::Other,
        };
    };
    let provider = if detail.provider.is_empty() {
        crate::accounts::pretty_provider_name(&detail.api_method)
    } else {
        crate::accounts::pretty_provider_name(&detail.provider)
    };
    // Custom profile ids arrive all-lowercase (`magpie`); show them as names
    // while leaving stylized brands like `xAI` alone.
    let provider = if provider
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
        && !provider.chars().any(|c| c.is_ascii_uppercase())
    {
        let mut chars = provider.chars();
        chars
            .next()
            .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
            .unwrap_or(provider)
    } else {
        provider
    };
    HeaderParts {
        logo: provider_logo(&detail.provider, &detail.api_method),
        provider,
        auth: AuthKind::of(&detail.api_method),
    }
}

/// Logo id (see `accounts::logo`) for the service a route goes through.
fn provider_logo(provider: &str, api_method: &str) -> &'static str {
    // OpenAI-compatible gateways route many profiles through one method, so
    // the profile/provider name alone decides the logo; a profile named
    // `deepseek` still earns its logo while `magpie` falls back to the
    // lettermark instead of inheriting "openai" from the method string.
    let haystack = if let Some(profile) = api_method.strip_prefix("openai-compatible:") {
        format!("{provider} {profile}").to_ascii_lowercase()
    } else if api_method == "openai-compatible" {
        provider.to_ascii_lowercase()
    } else {
        format!("{provider} {api_method}").to_ascii_lowercase()
    };
    for (needle, logo) in [
        ("openrouter", "openrouter"),
        ("copilot", "copilot"),
        ("cursor", "cursor"),
        ("bedrock", "bedrock"),
        ("azure", "azure"),
        ("antigravity", "antigravity"),
        ("jcode", "jcode"),
        ("anthropic", "anthropic-api"),
        ("claude", "anthropic-api"),
        ("openai", "openai"),
        ("chatgpt", "openai"),
        ("gemini", "gemini"),
        ("code-assist", "gemini"),
        ("google", "gemini"),
        ("grok", "xai"),
        ("xai", "xai"),
        ("mistral", "mistral"),
        ("deepseek", "deepseek"),
        ("groq", "groq"),
        ("ollama", "ollama"),
        ("lmstudio", "lmstudio"),
    ] {
        if haystack.contains(needle) {
            return logo;
        }
    }
    ""
}

/// Logo id for the model's own family, so an OpenAI model reads as OpenAI
/// whether it is served directly or through OpenRouter. Unknown families
/// fall back to the serving provider's logo.
pub(crate) fn model_logo(model: &str, details: &HashMap<String, ModelDetails>) -> &'static str {
    let detail = details.get(model);
    let raw = detail.map_or(model, |detail| detail.model.as_str());
    let raw = jcode_provider_core::explicit_model_provider_prefix(raw)
        .map_or(raw, |(_, _, bare)| bare)
        .to_ascii_lowercase();
    let (vendor, name) = raw.rsplit_once('/').unwrap_or(("", raw.as_str()));
    let vendor = vendor.rsplit('/').next().unwrap_or(vendor);
    let by_vendor = match vendor {
        "anthropic" => "anthropic-api",
        "openai" => "openai",
        "google" => "gemini",
        "x-ai" | "xai" => "xai",
        "mistralai" | "mistral" => "mistral",
        "deepseek" | "deepseek-ai" => "deepseek",
        "moonshotai" => "kimi",
        "qwen" => "alibaba-coding-plan",
        "z-ai" | "zai" | "zhipuai" => "zai",
        "minimax" => "minimax",
        _ => "",
    };
    if !by_vendor.is_empty() {
        return by_vendor;
    }
    // Bedrock-style `anthropic.claude-...` ids name the family after a dot.
    let name = name.split_once('.').map_or(name, |(head, rest)| {
        if head.chars().all(|c| c.is_ascii_alphabetic()) {
            rest
        } else {
            name
        }
    });
    for (prefix, logo) in [
        ("claude", "anthropic-api"),
        ("gpt", "openai"),
        ("codex", "openai"),
        ("o1", "openai"),
        ("o3", "openai"),
        ("o4", "openai"),
        ("gemini", "gemini"),
        ("gemma", "gemini"),
        ("grok", "xai"),
        ("mistral", "mistral"),
        ("codestral", "mistral"),
        ("devstral", "mistral"),
        ("deepseek", "deepseek"),
        ("kimi", "kimi"),
        ("qwen", "alibaba-coding-plan"),
        ("glm", "zai"),
        ("minimax", "minimax"),
    ] {
        if name.starts_with(prefix) {
            return logo;
        }
    }
    detail.map_or("", |detail| {
        provider_logo(&detail.provider, &detail.api_method)
    })
}

/// Namespaces that name a model's maker. OpenRouter-style `openai/gpt-x`
/// ids drop these, since the row logo already shows the maker.
const MODEL_MAKERS: &[&str] = &[
    "anthropic", "openai", "google", "x-ai", "xai", "mistralai", "mistral", "deepseek",
    "deepseek-ai", "moonshotai", "qwen", "z-ai", "zai", "zhipuai", "minimax", "meta-llama",
    "meta", "nvidia", "microsoft", "cohere", "amazon", "nousresearch", "perplexity",
];

/// A gateway's upstream channel, such as `codex` in `codex/gpt-6.2-sol` behind
/// an OpenAI-compatible router. The picker title otherwise drops the
/// namespace, which makes the same model on different channels identical.
fn channel_namespace<'a>(raw: &'a str, detail: Option<&ModelDetails>) -> Option<&'a str> {
    if detail.is_some_and(|detail| provider_logo(&detail.provider, &detail.api_method) == "openrouter")
    {
        return None;
    }
    let mut bare = jcode_provider_core::explicit_model_provider_prefix(raw)
        .map_or(raw, |(_, _, bare)| bare);
    // Profile prefixes (`magpie:codex/gpt-x`) are named by the group header.
    while let Some((head, rest)) = bare.split_once(':') {
        if head.is_empty()
            || rest.is_empty()
            || !head.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            break;
        }
        bare = rest;
    }
    let (namespace, _) = bare.rsplit_once('/')?;
    let channel = namespace.rsplit('/').next().unwrap_or(namespace).trim();
    (!channel.is_empty() && !MODEL_MAKERS.contains(&channel.to_ascii_lowercase().as_str()))
        .then_some(channel)
}

/// Whether the row's group header carries the channel, so its title drops
/// the channel suffix the header already shows.
fn channel_in_header(model: &str, details: &HashMap<String, ModelDetails>) -> bool {
    details
        .get(model)
        .is_some_and(|detail| channel_namespace(&detail.model, Some(detail)).is_some())
}

/// Group identity: provider, auth method, and channel. The channel splits a
/// gateway's upstream pools (Magpie `codex/…` vs `cpr-fast/…`) into groups.
pub(crate) fn group_key(model: &str, details: &HashMap<String, ModelDetails>) -> String {
    let Some(detail) = details.get(model) else {
        return "Other models".into();
    };
    let channel = channel_namespace(&detail.model, Some(detail)).unwrap_or("");
    format!("{} · {} · {}", detail.provider, detail.api_method, channel)
}

/// Header text before auth disambiguation: pretty provider plus
/// ` · {channel}` for channel groups.
fn group_base_label(model: &str, details: &HashMap<String, ModelDetails>) -> String {
    if details.get(model).is_none() {
        return "Other models".into();
    }
    let parts = header_parts(model, details);
    let detail = details.get(model).expect("checked above");
    match channel_namespace(&detail.model, Some(detail)) {
        Some(channel) => format!("{} · {}", parts.provider, channel),
        None => parts.provider,
    }
}

/// Header label per group key. Auth stays out of headers except when it is
/// the only thing telling two same-labeled groups apart (OpenAI via OAuth
/// vs API key): then ` · {auth}` is appended to those groups only.
fn group_labels(
    groups: &[(String, Vec<String>)],
    details: &HashMap<String, ModelDetails>,
) -> HashMap<String, String> {
    let base: Vec<String> = groups
        .iter()
        .map(|(_, members)| group_base_label(&members[0], details))
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for label in &base {
        *counts.entry(label.as_str()).or_default() += 1;
    }
    groups
        .iter()
        .zip(&base)
        .map(|((key, members), base_label)| {
            let mut label = base_label.clone();
            if counts[label.as_str()] > 1 {
                let auth = members
                    .first()
                    .map(|model| header_parts(model, details).auth.label())
                    .unwrap_or("Connected");
                label = format!("{label} · {auth}");
            }
            (key.clone(), label)
        })
        .collect()
}

/// Group ranked specs by key, preserving first-seen group order.
fn group_ranked(
    ranked: Vec<String>,
    details: &HashMap<String, ModelDetails>,
) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut group_indices = HashMap::new();
    for model in ranked {
        let key = group_key(&model, details);
        let index = *group_indices.entry(key.clone()).or_insert_with(|| {
            groups.push((key, Vec::new()));
            groups.len() - 1
        });
        groups[index].1.push(model);
    }
    groups
}

/// Non-favorite specs with recorded activity, newest first with rank order
/// breaking ties, capped at `limit`. Each entry pairs the spec with its
/// last activity timestamp for the row's `used …` suffix.
pub(crate) fn recent_models(
    ranked: &[String],
    details: &HashMap<String, ModelDetails>,
    favorites: &std::collections::HashSet<&str>,
    limit: usize,
) -> Vec<(String, u64)> {
    let mut recent: Vec<(usize, &String, u64)> = ranked
        .iter()
        .enumerate()
        .filter(|(_, model)| !favorites.contains(model.as_str()))
        .filter_map(|(index, model)| {
            let usage = details.get(model)?.usage.as_ref()?;
            let activity = usage
                .last_used_unix_secs
                .into_iter()
                .chain(usage.last_selected_unix_secs)
                .max()?;
            (activity > 0).then_some((index, model, activity))
        })
        .collect();
    recent.sort_by_key(|(index, _, activity)| (std::cmp::Reverse(*activity), *index));
    recent
        .into_iter()
        .take(limit)
        .map(|(_, model, activity)| (model.clone(), activity))
        .collect()
}

/// Friendly picker title for a route spec (`claude-oauth:claude-opus-4-8`
/// reads `Claude Opus 4.8`) with the channel suffix (` · codex`). The exact
/// id stays visible in the detail line.
pub(crate) fn pretty_title(model: &str, details: &HashMap<String, ModelDetails>) -> String {
    let (pretty, channel) = title_parts(model, details);
    match channel {
        Some(channel) => format!("{pretty} · {channel}"),
        None => pretty,
    }
}

/// Title inside a group whose header already carries the channel, so the
/// suffix is dropped.
pub(crate) fn pretty_title_in_group(
    model: &str,
    details: &HashMap<String, ModelDetails>,
) -> String {
    title_parts(model, details).0
}

fn title_parts<'a>(
    model: &'a str,
    details: &'a HashMap<String, ModelDetails>,
) -> (String, Option<&'a str>) {
    let detail = details.get(model);
    let raw = detail.map_or(model, |detail| detail.model.as_str());
    let pretty = jcode_provider_core::model_names::pretty_picker_model_name(raw);
    let pretty = if pretty.is_empty() {
        raw.to_string()
    } else {
        pretty
    };
    (pretty, channel_namespace(raw, detail))
}

/// Capability metadata shown as the row's detail line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ModelMeta {
    pub context_window: Option<usize>,
    pub image_input: Option<bool>,
    pub reasoning: Option<bool>,
}

impl ModelMeta {
    fn parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(window) = self.context_window {
            parts.push(format!("{} context", format_context_window(window)));
        }
        if self.image_input == Some(true) {
            parts.push("Images".to_string());
        }
        if self.reasoning == Some(true) {
            parts.push("Reasoning".to_string());
        }
        parts
    }
}

fn format_context_window(window: usize) -> String {
    if window >= 1_000_000 {
        let millions = window as f64 / 1_000_000.0;
        let text = format!("{millions:.1}");
        format!("{}M", text.trim_end_matches(".0"))
    } else {
        format!("{}K", window / 1000)
    }
}

/// The row's second line: capability metadata, plus the last-activity time
/// on Recently used rows. `None` renders a single-line row.
pub(crate) fn detail_label(
    meta: Option<&ModelMeta>,
    used_unix_secs: Option<u64>,
    now: u64,
) -> Option<String> {
    let mut parts = meta.map_or_else(Vec::new, ModelMeta::parts);
    if let Some(used) = used_unix_secs {
        parts.push(format!("used {}", relative_time(used, now)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(count: u64, last: Option<u64>, selections: u64) -> ModelUsage {
        ModelUsage {
            count,
            last_used_unix_secs: last,
            tracking_started_unix_secs: Some(10),
            selection_count: selections,
            last_selected_unix_secs: Some(20),
        }
    }

    fn route(model: &str, method: &str, count: u64) -> ModelRouteInfo {
        ModelRouteInfo {
            model: model.into(),
            provider: "OpenAI".into(),
            api_method: method.into(),
            available: true,
            detail: String::new(),
            usage: Some(usage(count, Some(count), 0)),
        }
    }

    #[test]
    fn gateway_channels_stay_visible_but_makers_do_not() {
        let mut magpie = route("codex/gpt-6.2-sol", "openai-compatible", 0);
        magpie.provider = "magpie".into();
        let mut other = route("cpr-fast/gpt-6.2-sol", "openai-compatible", 0);
        other.provider = "magpie".into();
        let maker = route("openai/gpt-6.2-sol", "openai-compatible", 0);
        let mut routed = route("acme/gpt-6.2-sol", "openrouter", 0);
        routed.provider = "OpenRouter".into();
        let details = from_routes(&[magpie, other, maker, routed]);
        let title = |model: &str| {
            let spec = details
                .iter()
                .find(|(_, detail)| detail.model == model)
                .map(|(spec, _)| spec.clone())
                .unwrap();
            pretty_title(&spec, &details)
        };
        assert!(title("codex/gpt-6.2-sol").ends_with(" · codex"));
        assert!(title("cpr-fast/gpt-6.2-sol").ends_with(" · cpr-fast"));
        assert_ne!(title("codex/gpt-6.2-sol"), title("cpr-fast/gpt-6.2-sol"));
        assert!(!title("openai/gpt-6.2-sol").contains('·'));
        assert!(!title("acme/gpt-6.2-sol").contains('·'));
    }

    #[test]
    fn groups_keep_auth_routes_distinct_and_canonicalize_aliases() {
        let routes = [
            route("atlas", "openai-oauth", 2),
            route("atlas", "openai-api-key", 1),
            route("openai-oauth:atlas", "openai-oauth", 0),
        ];
        let details = from_routes(&routes);
        assert_eq!(details.len(), 2);
        assert!(details.contains_key("openai-oauth:atlas"));
        assert!(details.contains_key("openai-api:atlas"));
        assert_eq!(
            route_spec(&route("openai:atlas", "openai-api-key", 0)),
            "openai-api:atlas"
        );
        let rows = grouped_rows(
            &details.keys().cloned().collect::<Vec<_>>(),
            &details,
            None,
            &Default::default(),
            &[],
            true,
            false,
            "",
        );
        // Recently used leads; the two same-named groups disambiguate on auth.
        let headers: Vec<_> = rows
            .iter()
            .filter_map(|row| row.header.as_deref())
            .collect();
        assert_eq!(
            headers,
            [RECENT_HEADER, "OpenAI · Signed in", "OpenAI · API key"]
        );
        assert_eq!(rows[0].value, "/model openai-oauth:atlas");
        assert_eq!(rows[1].value, "/model openai-api:atlas");
        assert_eq!(
            route_spec(&route("bedrock:anthropic.claude-v1:0", "bedrock", 0)),
            "bedrock:anthropic.claude-v1:0"
        );
    }

    #[test]
    fn groups_show_three_distinct_ranked_models_and_search_hidden_choices() {
        let routes: Vec<_> = (1..=6)
            .map(|n| route(&format!("atlas-{n}"), "openai-oauth", n))
            .collect();
        let details = from_routes(&routes);
        let mut models: Vec<_> = details.keys().cloned().collect();
        models.push("openai-oauth:atlas-6".into());
        let closed = grouped_rows(
            &models,
            &details,
            Some("atlas-1"),
            &Default::default(),
            &[],
            true,
            false,
            "",
        );
        // Five recent rows lead the single capped group.
        assert_eq!(closed[0].header.as_deref(), Some(RECENT_HEADER));
        assert!(closed[0].recent);
        assert_eq!(
            closed.iter().map(|r| r.value.as_str()).collect::<Vec<_>>(),
            [
                "/model openai-oauth:atlas-6",
                "/model openai-oauth:atlas-5",
                "/model openai-oauth:atlas-4",
                "/model openai-oauth:atlas-3",
                "/model openai-oauth:atlas-2",
                "/model openai-oauth:atlas-1",
                "/model openai-oauth:atlas-6",
                "/model openai-oauth:atlas-5",
                "Show 3 more models"
            ]
        );
        assert_eq!(closed[5].header.as_deref(), Some("OpenAI"));
        let key = closed.last().unwrap().toggle.clone().unwrap();
        let expanded = [key].into_iter().collect();
        let open = grouped_rows(
            &models,
            &details,
            Some("atlas-1"),
            &expanded,
            &[],
            true,
            false,
            "",
        );
        assert_eq!(open.len(), 12);
        assert_eq!(open[11].value, "Show fewer models");
        let search = grouped_rows(
            &models,
            &details,
            None,
            &Default::default(),
            &[],
            true,
            false,
            "ATLAS-2",
        );
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].value, "/model openai-oauth:atlas-2");
        assert!(search[0].toggle.is_none());
        assert!(search.iter().all(|row| !row.recent));
        assert_eq!(
            grouped_rows(&models, &details, None, &Default::default(), &[], true, false, "oauth")
                .len(),
            6
        );
        assert!(grouped_rows(&models, &details, None, &expanded, &[], true, false, "missing")
            .is_empty());
    }

    #[test]
    fn recommendation_breaks_usage_ties_and_unavailable_routes_are_excluded() {
        let mut unavailable = route("unavailable", "openai-oauth", 100);
        unavailable.available = false;
        let recommended = jcode_provider_core::DEFAULT_OPENAI_MODEL;
        let details = from_routes(&[
            route("a-unknown", "openai-oauth", 0),
            route(recommended, "openai-oauth", 0),
            unavailable,
        ]);
        let mut models: Vec<_> = details.keys().cloned().collect();
        rank(&mut models, &details);
        assert_eq!(
            models,
            [
                format!("openai-oauth:{recommended}"),
                "openai-oauth:a-unknown".to_string()
            ]
        );
    }

    #[test]
    fn relative_time_ages() {
        assert_eq!(relative_time(100, 864100), "10d ago");
        assert_eq!(relative_time(200, 100), "just now");
    }

    #[test]
    fn row_logo_names_the_model_maker_and_headers_name_route_and_auth() {
        let mut routed = route("openai/gpt-6-astra", "openrouter", 0);
        routed.provider = "OpenRouter".into();
        let details = from_routes(&[
            route("gpt-6-astra", "openai-api-key", 3),
            route("gpt-6-astra", "openai-oauth", 1),
            routed,
        ]);
        assert_eq!(details.len(), 3);
        for (spec, detail) in &details {
            assert_eq!(model_logo(spec, &details), "openai", "{spec}");
            let parts = header_parts(spec, &details);
            let expected = match detail.api_method.as_str() {
                "openrouter" => ("OpenRouter", "openrouter", AuthKind::ApiKey),
                "openai-oauth" => ("OpenAI", "openai", AuthKind::Account),
                _ => ("OpenAI", "openai", AuthKind::ApiKey),
            };
            assert_eq!((parts.provider.as_str(), parts.logo, parts.auth), expected);
        }
        assert_eq!(AuthKind::of("jcode-subscription"), AuthKind::Subscription);
        assert_eq!(AuthKind::of("bedrock"), AuthKind::CloudCredentials);
        assert_eq!(AuthKind::of("claude-oauth"), AuthKind::Account);
        let mut bedrock = route("anthropic.claude-v1:0", "bedrock", 0);
        bedrock.provider = "Bedrock".into();
        let details = from_routes(&[bedrock]);
        let spec = details.keys().next().unwrap();
        assert_eq!(model_logo(spec, &details), "anthropic-api");
        // Unknown families fall back to the serving provider.
        let mut private = route("house-model", "openrouter", 0);
        private.provider = "OpenRouter".into();
        let details = from_routes(&[private]);
        let spec = details.keys().next().unwrap();
        assert_eq!(model_logo(spec, &details), "openrouter");
    }

    #[test]
    fn openai_compatible_headers_use_the_profile_not_the_openai_logo() {
        assert_eq!(provider_logo("magpie", "openai-compatible:magpie"), "");
        assert_eq!(
            provider_logo("deepseek", "openai-compatible:deepseek"),
            "deepseek"
        );
        assert_eq!(provider_logo("magpie", "openai-compatible"), "");
        assert_eq!(provider_logo("openai", "openai-api-key"), "openai");
    }

    #[test]
    fn rank_uses_usage_then_stable_names_and_retains_every_model() {
        let mut models = vec![
            "unknown-z".into(),
            "frequent".into(),
            "unknown-a".into(),
            "recent".into(),
            "legacy".into(),
        ];
        let details = [
            ("frequent", usage(12, Some(100), 0)),
            ("recent", usage(12, Some(200), 0)),
            ("legacy", usage(0, None, 9)),
        ]
        .into_iter()
        .map(|(model, usage)| {
            (
                model.into(),
                ModelDetails {
                    model: model.into(),
                    recommended: false,
                    provider: String::new(),
                    api_method: String::new(),
                    usage: Some(usage),
                },
            )
        })
        .collect();
        rank(&mut models, &details);
        assert_eq!(
            models,
            ["recent", "frequent", "legacy", "unknown-a", "unknown-z"]
        );
    }

    fn magpie_route(model: &str, count: u64, last_used: Option<u64>) -> ModelRouteInfo {
        let mut route = route(model, "openai-compatible:magpie", count);
        route.provider = "magpie".into();
        route.usage = Some(ModelUsage {
            count,
            last_used_unix_secs: last_used,
            tracking_started_unix_secs: Some(10),
            selection_count: 0,
            last_selected_unix_secs: None,
        });
        route
    }

    #[test]
    fn channels_split_one_gateway_into_labeled_groups_without_auth() {
        let details = from_routes(&[
            magpie_route("codex/gpt-6.2-sol", 0, None),
            magpie_route("cpr-fast/gpt-6.2-sol", 0, None),
            magpie_route("truesota/deepseek-flash", 0, None),
        ]);
        let models: Vec<_> = details.keys().cloned().collect();
        let rows = grouped_rows(&models, &details, None, &Default::default(), &[], true, false, "");
        let headers: Vec<_> = rows.iter().filter_map(|row| row.header.as_deref()).collect();
        assert_eq!(
            headers,
            ["Magpie · codex", "Magpie · cpr-fast", "Magpie · truesota"]
        );
        assert!(rows.iter().all(|row| row.channel_header));
        // Rows inside a channel group drop the suffix the header carries.
        for row in &rows {
            let spec = row.value.strip_prefix("/model ").unwrap();
            let title = pretty_title_in_group(spec, &details);
            assert!(!title.contains('·'), "{title}");
            assert!(pretty_title(spec, &details).contains('·'));
        }
    }

    #[test]
    fn favorites_then_recent_then_groups_and_recent_excludes_favorites() {
        let mut first = route("atlas-1", "openai-oauth", 0);
        first.usage = Some(usage(0, Some(100), 0));
        let mut second = route("atlas-2", "openai-oauth", 0);
        second.usage = Some(usage(0, Some(300), 0));
        let mut third = route("atlas-3", "openai-oauth", 0);
        third.usage = Some(usage(0, Some(200), 0));
        let details = from_routes(&[first, second, third]);
        let models: Vec<_> = details.keys().cloned().collect();
        let favorites = vec!["openai-oauth:atlas-1".to_string()];
        let rows = grouped_rows(
            &models,
            &details,
            None,
            &Default::default(),
            &favorites,
            true,
            false,
            "",
        );
        assert_eq!(rows[0].header.as_deref(), Some(FAVORITES_HEADER));
        assert_eq!(rows[0].value, "/model openai-oauth:atlas-1");
        // The favorite is skipped in Recently used, which lists newest first.
        let recent_start = rows
            .iter()
            .position(|row| row.header.as_deref() == Some(RECENT_HEADER))
            .unwrap();
        assert!(rows[recent_start].recent);
        assert_eq!(rows[recent_start].value, "/model openai-oauth:atlas-2");
        assert_eq!(rows[recent_start + 1].value, "/model openai-oauth:atlas-3");
        let group_start = rows[recent_start + 2..]
            .iter()
            .position(|row| row.header.is_some())
            .map(|index| recent_start + 2 + index)
            .unwrap();
        assert_eq!(rows[group_start].header.as_deref(), Some("OpenAI"));
        // Favorites section titles keep the channel suffix.
        assert!(!rows[0].channel_header);
    }

    #[test]
    fn search_shows_no_sections_and_matches_the_channel() {
        let details = from_routes(&[
            magpie_route("codex/gpt-6.2-sol", 0, Some(500)),
            magpie_route("cpr-fast/gpt-6.2-sol", 0, None),
        ]);
        let models: Vec<_> = details.keys().cloned().collect();
        let favorites = vec!["magpie:cpr-fast/gpt-6.2-sol".to_string()];
        let rows = grouped_rows(
            &models,
            &details,
            None,
            &Default::default(),
            &favorites,
            true,
            false,
            "codex",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "/model magpie:codex/gpt-6.2-sol");
        assert_eq!(rows[0].header.as_deref(), Some("Magpie · codex"));
        assert!(rows[0].channel_header);
        assert!(rows.iter().all(|row| !row.recent));
    }

    #[test]
    fn detail_label_formats_meta_and_used_time() {
        let meta = |context_window, image_input, reasoning| ModelMeta {
            context_window,
            image_input,
            reasoning,
        };
        assert_eq!(
            detail_label(Some(&meta(Some(272_000), Some(true), Some(true))), None, 0).as_deref(),
            Some("272K context · Images · Reasoning")
        );
        assert_eq!(
            detail_label(Some(&meta(Some(1_000_000), None, None)), None, 0).as_deref(),
            Some("1M context")
        );
        assert_eq!(
            detail_label(Some(&meta(Some(1_500_000), Some(false), None)), None, 0).as_deref(),
            Some("1.5M context")
        );
        assert_eq!(detail_label(Some(&ModelMeta::default()), None, 0), None);
        assert_eq!(detail_label(None, None, 0), None);
        assert_eq!(
            detail_label(None, Some(86_400), 86_400 + 3_600).as_deref(),
            Some("used 1h ago")
        );
        assert_eq!(
            detail_label(
                Some(&meta(Some(272_000), None, None)),
                Some(86_400),
                86_400 + 3_600
            )
            .as_deref(),
            Some("272K context · used 1h ago")
        );
    }
}

/// A route header belongs to its first selectable row, not the keyboard index space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GroupRow {
    pub value: String,
    pub header: Option<String>,
    pub toggle: Option<String>,
    /// The group header already carries the channel, so titles drop it.
    pub channel_header: bool,
    /// A "Recently used" row; its detail line appends the last-activity time.
    pub recent: bool,
}

pub(crate) const STAR_ICON: &[u8] = include_bytes!("../../../assets/icons/star.svg");
pub(crate) const STAR_FILLED_ICON: &[u8] =
    include_bytes!("../../../assets/icons/star-filled.svg");

/// Section headers the renderer draws without a provider logo.
pub(crate) const FAVORITES_HEADER: &str = "Favorites";
pub(crate) const RECENT_HEADER: &str = "Recently used";

/// Rows for the `/model` menu and the model browser's grouped view.
///
/// With an empty query and `sections`, Favorites (all available, in catalog
/// rank order) and Recently used (up to five non-favorites, newest first)
/// lead, followed by the provider/channel groups. Models appear in a section
/// and again in their group. `uncapped` lists every group row without the
/// "Show N more" toggles, which is how the browser shows All models.
/// Usage-ranked unique specs: the catalog order every picker view shares.
pub(crate) fn ranked_specs(
    models: &[String],
    details: &HashMap<String, ModelDetails>,
) -> Vec<String> {
    let mut ranked = models.to_vec();
    rank(&mut ranked, details);
    let mut seen = std::collections::HashSet::with_capacity(ranked.len());
    ranked.retain(|model| seen.insert(model.clone()));
    ranked
}

/// Provider/channel groups for the browser's filter pills and grouped list:
/// `(key, header label, member specs)` with the current model's group first.
pub(crate) fn groups_of(
    models: &[String],
    details: &HashMap<String, ModelDetails>,
    current: Option<&str>,
) -> Vec<(String, String, Vec<String>)> {
    let mut ranked = ranked_specs(models, details);
    let current = current_spec(current, &ranked, details).map(str::to_string);
    ranked.sort_by_key(|model| current.as_deref() != Some(model.as_str()));
    let groups = group_ranked(ranked, details);
    let labels = group_labels(&groups, details);
    groups
        .into_iter()
        .map(|(key, members)| {
            let label = labels[&key].clone();
            (key, label, members)
        })
        .collect()
}

/// Which flat list the model browser shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelFilter {
    All,
    Favorites,
    Recent,
    /// One provider/channel group by its `group_key`.
    Group(String),
}

/// One rendered row in the model browser dialog.
#[derive(Clone, Debug)]
pub(crate) struct BrowserRow {
    pub spec: String,
    pub title: String,
    pub detail: Option<String>,
    /// Group header shown above this row.
    pub header: Option<String>,
    /// Service logo id for `header` (`""` when there is none).
    pub header_logo: &'static str,
    /// Model-maker logo id; the header logo comes from `header_parts`.
    pub logo: &'static str,
    pub current: bool,
    pub favorite: bool,
}

/// One filter pill: `All models N`, `Favorites N`, `Recently used N`, then
/// one pill per provider/channel group with its logo.
#[derive(Clone, Debug)]
pub(crate) struct BrowserFilter {
    pub filter: ModelFilter,
    pub label: String,
    pub logo: &'static str,
    pub count: usize,
}

/// The browser's filter pills in display order.
pub(crate) fn browser_filters(
    models: &[String],
    details: &HashMap<String, ModelDetails>,
    favorites: &[String],
    current: Option<&str>,
) -> Vec<BrowserFilter> {
    let ranked = ranked_specs(models, details);
    let favorite_set: std::collections::HashSet<&str> =
        favorites.iter().map(String::as_str).collect();
    let favorite_count = ranked
        .iter()
        .filter(|model| favorite_set.contains(model.as_str()))
        .count();
    let recent_count = recent_models(&ranked, details, &favorite_set, 5).len();
    let mut filters = vec![
        BrowserFilter {
            filter: ModelFilter::All,
            label: "All models".into(),
            logo: "",
            count: ranked.len(),
        },
        BrowserFilter {
            filter: ModelFilter::Favorites,
            label: "Favorites".into(),
            logo: "",
            count: favorite_count,
        },
        BrowserFilter {
            filter: ModelFilter::Recent,
            label: "Recently used".into(),
            logo: "",
            count: recent_count,
        },
    ];
    for (key, label, members) in groups_of(models, details, current) {
        let logo = members
            .first()
            .map(|model| header_parts(model, details).logo)
            .unwrap_or("");
        filters.push(BrowserFilter {
            filter: ModelFilter::Group(key),
            label,
            logo,
            count: members.len(),
        });
    }
    filters
}

/// Rows for the active browser filter. `All` keeps provider/channel headers
/// and expands every group; Favorites/Recent/Group are flat lists. A query
/// filters inside the active filter through `model_search` ranking.
pub(crate) fn browser_rows(
    filter: &ModelFilter,
    models: &[String],
    details: &HashMap<String, ModelDetails>,
    metas: &HashMap<String, ModelMeta>,
    favorites: &[String],
    current: Option<&str>,
    query: &str,
    now: u64,
) -> Vec<BrowserRow> {
    let favorite_set: std::collections::HashSet<&str> =
        favorites.iter().map(String::as_str).collect();
    let query_str = query;
    let query = super::model_search::ModelQuery::new(query_str);
    let ranked = ranked_specs(models, details);
    let current = current_spec(current, &ranked, details).map(str::to_string);
    let row = |spec: &str, title: String, used: Option<u64>| BrowserRow {
        spec: spec.to_string(),
        title,
        detail: detail_label(metas.get(spec), used, now),
        header: None,
        header_logo: "",
        logo: model_logo(spec, details),
        current: current.as_deref() == Some(spec),
        favorite: favorite_set.contains(spec),
    };
    match filter {
        ModelFilter::All => {
            let rows = grouped_rows(
                models,
                details,
                current.as_deref(),
                &Default::default(),
                &[],
                false,
                true,
                query_str,
            );
            rows.into_iter()
                .filter(|row| row.toggle.is_none())
                .map(|group_row| {
                    let spec = group_row.value.trim_start_matches("/model ");
                    let title = if group_row.channel_header {
                        pretty_title_in_group(spec, details)
                    } else {
                        pretty_title(spec, details)
                    };
                    let mut row = row(spec, title, None);
                    row.header_logo = group_row
                        .header
                        .as_ref()
                        .map(|_| header_parts(spec, details).logo)
                        .unwrap_or("");
                    row.header = group_row.header;
                    row
                })
                .collect()
        }
        ModelFilter::Favorites => {
            let specs: Vec<String> = ranked
                .iter()
                .filter(|model| favorite_set.contains(model.as_str()))
                .cloned()
                .collect();
            apply_query(&specs, details, &query)
                .into_iter()
                .map(|spec| row(spec, pretty_title(spec, details), None))
                .collect()
        }
        ModelFilter::Recent => {
            let recent = recent_models(&ranked, details, &favorite_set, 5);
            let used: HashMap<String, u64> =
                recent.iter().map(|(spec, at)| (spec.clone(), *at)).collect();
            let specs: Vec<String> = recent.into_iter().map(|(spec, _)| spec).collect();
            apply_query(&specs, details, &query)
                .into_iter()
                .map(|spec| {
                    row(
                        spec,
                        pretty_title(spec, details),
                        used.get(spec).copied(),
                    )
                })
                .collect()
        }
        ModelFilter::Group(key) => {
            let members = groups_of(models, details, current.as_deref())
                .into_iter()
                .find(|(group_key, _, _)| group_key == key)
                .map(|(_, _, members)| members)
                .unwrap_or_default();
            apply_query(&members, details, &query)
                .into_iter()
                .map(|spec| {
                    let title = if channel_in_header(spec, details) {
                        pretty_title_in_group(spec, details)
                    } else {
                        pretty_title(spec, details)
                    };
                    row(spec, title, None)
                })
                .collect()
        }
    }
}

/// `specs` ranked by `model_search` for `query`, or unchanged when empty.
fn apply_query<'a>(
    specs: &'a [String],
    details: &HashMap<String, ModelDetails>,
    query: &super::model_search::ModelQuery,
) -> Vec<&'a String> {
    if query.is_empty() {
        return specs.iter().collect();
    }
    super::model_search::rank_matches(
        specs.iter(),
        details,
        |model| {
            let key = group_key(model, details);
            format!("{} {}", group_base_label(model, details), key)
        },
        query,
    )
    .into_iter()
    .map(|(model, _)| model)
    .collect()
}

pub(crate) fn grouped_rows(
    models: &[String],
    details: &HashMap<String, ModelDetails>,
    current: Option<&str>,
    expanded: &std::collections::HashSet<String>,
    favorites: &[String],
    sections: bool,
    uncapped: bool,
    query: &str,
) -> Vec<GroupRow> {
    let mut ranked = ranked_specs(models, details);
    let favorite_set: std::collections::HashSet<&str> =
        favorites.iter().map(String::as_str).collect();
    let query = super::model_search::ModelQuery::new(query);
    let searching = !query.is_empty();
    let mut rows = Vec::new();
    if sections && !searching {
        let mut first = true;
        for model in ranked
            .iter()
            .filter(|model| favorite_set.contains(model.as_str()))
        {
            rows.push(GroupRow {
                header: first.then(|| FAVORITES_HEADER.to_string()),
                value: format!("/model {model}"),
                toggle: None,
                channel_header: false,
                recent: false,
            });
            first = false;
        }
        let mut first = true;
        for (model, _) in recent_models(&ranked, details, &favorite_set, 5) {
            rows.push(GroupRow {
                header: first.then(|| RECENT_HEADER.to_string()),
                value: format!("/model {model}"),
                toggle: None,
                channel_header: false,
                recent: true,
            });
            first = false;
        }
    }
    let current = current_spec(current, &ranked, details).map(str::to_string);
    ranked.sort_by_key(|model| current.as_deref() != Some(model.as_str()));
    let groups = group_ranked(ranked, details);
    let labels = group_labels(&groups, details);
    if searching {
        // Rank every route, then keep provider grouping but order groups by
        // their best hit so Enter always selects the strongest match.
        let ranked_matches = super::model_search::rank_matches(
            groups.iter().flat_map(|(_, members)| members.iter()),
            details,
            |model| {
                let key = group_key(model, details);
                format!("{} {}", labels[&key], key)
            },
            &query,
        );
        let scores: HashMap<&String, (usize, i32)> = ranked_matches
            .iter()
            .enumerate()
            .map(|(order, (model, score))| (*model, (order, *score)))
            .collect();
        let mut matched: Vec<(usize, Vec<String>)> = groups
            .iter()
            .filter_map(|(_, members)| {
                let mut hits: Vec<_> = members
                    .iter()
                    .filter_map(|model| {
                        scores.get(model).map(|(order, _)| (*order, model.clone()))
                    })
                    .collect();
                hits.sort_by_key(|(order, _)| *order);
                let best = hits.first()?.0;
                Some((best, hits.into_iter().map(|(_, model)| model).collect()))
            })
            .collect();
        matched.sort_by_key(|(best, _)| *best);
        for (_, members) in matched {
            let channel_header = members
                .first()
                .is_some_and(|model| channel_in_header(model, details));
            for (index, model) in members.into_iter().enumerate() {
                rows.push(GroupRow {
                    header: (index == 0)
                        .then(|| labels[&group_key(&model, details)].clone()),
                    value: format!("/model {model}"),
                    toggle: None,
                    channel_header,
                    recent: false,
                });
            }
        }
        return rows;
    }
    for (key, matching) in groups {
        let count = matching.len();
        let open = uncapped || expanded.contains(&key);
        let channel_header = matching
            .first()
            .is_some_and(|model| channel_in_header(model, details));
        for (index, model) in matching
            .into_iter()
            .take(if open { usize::MAX } else { 3 })
            .enumerate()
        {
            rows.push(GroupRow {
                header: (index == 0).then(|| labels[&key].clone()),
                value: format!("/model {model}"),
                toggle: None,
                channel_header,
                recent: false,
            });
        }
        if count > 3 && !uncapped {
            rows.push(GroupRow {
                value: if open {
                    "Show fewer models".into()
                } else {
                    format!("Show {} more models", count - 3)
                },
                header: None,
                toggle: Some(key),
                channel_header: false,
                recent: false,
            });
        }
    }
    rows
}

#[cfg(test)]
mod grouping_performance_tests {
    use super::*;
    use std::collections::HashSet;

    fn fixture(count: usize, group_count: usize) -> (Vec<String>, HashMap<String, ModelDetails>) {
        let models: Vec<_> = (0..count)
            .map(|i| format!("provider:model-{i:04}"))
            .collect();
        let details = models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                (
                    model.clone(),
                    ModelDetails {
                        model: format!("model-{i:04}"),
                        recommended: i % 7 == 0,
                        provider: format!("Provider {}", i % group_count),
                        api_method: "openai-oauth".into(),
                        usage: None,
                    },
                )
            })
            .collect();
        (models, details)
    }

    #[test]
    fn indexed_grouping_matches_legacy_order_filtering_and_deduplication() {
        for group_count in [1, 8, 200] {
            let (mut models, details) = fixture(200, group_count);
            // Deliberately repeated and missing-metadata entries exercise stable
            // deduplication and the fallback group, not just unique route maps.
            models.extend(models.clone());
            models.extend(["unknown-model".into(), "unknown-model".into()]);
            let expanded: HashSet<_> = models.iter().map(|m| group_key(m, &details)).collect();
            for expansion in [&HashSet::new(), &expanded] {
                // Search is ranked by `model_search` and intentionally differs
                // from the legacy substring filter; browsing must not.
                for query in [""] {
                    for current in [None, Some("provider:model-0199"), Some("model-0005")] {
                        assert_eq!(
                            grouped_rows(&models, &details, current, expansion, &[], true, false, query),
                            legacy_grouped_rows(&models, &details, current, expansion, &[], true, false, query),
                            "groups={group_count}, query={query}, current={current:?}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "opt-in CPU microbenchmark, run with --ignored --nocapture"]
    fn grouped_rows_profile() {
        use std::hint::black_box;
        use std::time::Instant;
        type Grouping = fn(
            &[String],
            &HashMap<String, ModelDetails>,
            Option<&str>,
            &HashSet<String>,
            &[String],
            bool,
            bool,
            &str,
        ) -> Vec<GroupRow>;
        for count in [40, 200, 1000] {
            for group_count in [1, 8] {
                let (models, details) = fixture(count, group_count);
                let closed = HashSet::new();
                let expanded: HashSet<_> = models.iter().map(|m| group_key(m, &details)).collect();
                for (scenario, expansion, query) in [
                    ("collapsed", &closed, ""),
                    ("expanded", &expanded, ""),
                    ("broad-search", &closed, "model"),
                    ("narrow-search", &closed, "model-001"),
                    ("no-match", &closed, "missing"),
                ] {
                    let current = Some(models[count - 1].as_str());
                    if query.is_empty() {
                        assert_eq!(
                            grouped_rows(&models, &details, current, expansion, &[], true, false, query),
                            legacy_grouped_rows(&models, &details, current, expansion, &[], true, false, query)
                        );
                    }
                    for (implementation, group) in [
                        ("legacy", legacy_grouped_rows as Grouping),
                        ("indexed", grouped_rows as Grouping),
                    ] {
                        for _ in 0..20 {
                            black_box(group(&models, &details, current, expansion, &[], true, false, query));
                        }
                        let mut micros = Vec::with_capacity(200);
                        for _ in 0..200 {
                            let started = Instant::now();
                            black_box(group(
                                black_box(&models),
                                &details,
                                current,
                                expansion,
                                &[],
                                true,
                                false,
                                query,
                            ));
                            micros.push(started.elapsed().as_secs_f64() * 1_000_000.);
                        }
                        micros.sort_by(f64::total_cmp);
                        eprintln!(
                            "grouped_rows {implementation} n={count} groups={group_count} {scenario}: p50={:.1}us p95={:.1}us max={:.1}us",
                            micros[100], micros[189], micros[199]
                        );
                    }
                }
            }
        }
    }

    // Frozen pre-indexing implementation: a same-binary reference for both
    // correctness and performance, not a second production code path.
    fn legacy_grouped_rows(
        models: &[String],
        details: &HashMap<String, ModelDetails>,
        current: Option<&str>,
        expanded: &std::collections::HashSet<String>,
        _favorites: &[String],
        _sections: bool,
        uncapped: bool,
        query: &str,
    ) -> Vec<GroupRow> {
        let mut ranked = models.to_vec();
        rank(&mut ranked, details);
        let current = current_spec(current, &ranked, details).map(str::to_string);
        ranked.sort_by_key(|model| current.as_deref() != Some(model.as_str()));
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for model in ranked {
            let key = group_key(&model, details);
            let index = groups
                .iter()
                .position(|(group, _)| group == &key)
                .unwrap_or_else(|| {
                    groups.push((key.clone(), Vec::new()));
                    groups.len() - 1
                });
            let members = &mut groups[index].1;
            if !members.contains(&model) {
                members.push(model);
            }
        }
        let labels = group_labels(&groups, details);
        let query = query.trim().to_ascii_lowercase();
        let mut rows = Vec::new();
        for (key, members) in groups {
            let searching = !query.is_empty();
            let matching: Vec<_> = members
                .into_iter()
                .filter(|model| {
                    !searching
                        || model.to_ascii_lowercase().contains(&query)
                        || details.get(model).is_some_and(|detail| {
                            detail.model.to_ascii_lowercase().contains(&query)
                        })
                        || labels[&key].to_ascii_lowercase().contains(&query)
                        || key.to_ascii_lowercase().contains(&query)
                })
                .collect();
            let count = matching.len();
            let open = uncapped || expanded.contains(&key);
            let channel_header = matching
                .first()
                .is_some_and(|model| channel_in_header(model, details));
            for (index, model) in matching
                .into_iter()
                .take(if searching || open { usize::MAX } else { 3 })
                .enumerate()
            {
                rows.push(GroupRow {
                    header: (index == 0).then(|| labels[&key].clone()),
                    value: format!("/model {model}"),
                    toggle: None,
                    channel_header,
                    recent: false,
                });
            }
            if !searching && count > 3 && !uncapped {
                rows.push(GroupRow {
                    value: if open {
                        "Show fewer models".into()
                    } else {
                        format!("Show {} more models", count - 3)
                    },
                    header: None,
                    toggle: Some(key),
                    channel_header: false,
                    recent: false,
                });
            }
        }
        rows
    }
}
