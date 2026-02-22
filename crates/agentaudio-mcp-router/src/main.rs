use std::{
    borrow::Cow,
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolRequestParams, CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info_span, instrument};
use tokio_util::sync::CancellationToken;

type SharedRegistry = Arc<RwLock<HashMap<String, RegisteredInstance>>>;

#[derive(Debug, Clone, Serialize)]
struct RegisteredInstance {
    instance_id: String,
    endpoint: String,
    mcp_name: String,
    last_seen_ms: u64,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    instance_id: String,
    endpoint: String,
    mcp_name: String,
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    instance_id: String,
    /// Optional: update the displayed name (e.g. when plugin load/unload changes).
    #[serde(default)]
    mcp_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UnregisterRequest {
    instance_id: String,
}

const PARAM_CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct AppState {
    registry: SharedRegistry,
    default_instance_id: Arc<RwLock<Option<String>>>,
    param_cache: Arc<RwLock<HashMap<String, (Vec<serde_json::Value>, Instant)>>>,
    started_at: Instant,
}

// ---- MCP tool parameter types ----

#[derive(Debug, Deserialize, JsonSchema)]
struct SelectInstanceRequest {
    pub instance_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyScanPluginsRequest {
    pub instance_id: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyLoadChildRequest {
    pub instance_id: Option<String>,
    pub uid: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyInstanceOnly {
    pub instance_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyListParamsRequest {
    pub instance_id: Option<String>,
    /// Optional name prefix filter (case-insensitive). Only returns params whose names start with this prefix.
    pub prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxySearchParamsRequest {
    pub instance_id: Option<String>,
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxySetParamRequest {
    pub instance_id: Option<String>,
    pub id: u32,
    pub value: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ParamChange {
    pub id: u32,
    pub value: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyBatchSetRequest {
    pub instance_id: Option<String>,
    pub changes: Vec<ParamChange>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyFindVstParameterRequest {
    pub instance_id: Option<String>,
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyPreviewVstParameterValuesRequest {
    pub instance_id: Option<String>,
    pub ids: Option<Vec<u32>>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyGetParamInfoRequest {
    pub instance_id: Option<String>,
    pub id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxySavePresetRequest {
    pub instance_id: Option<String>,
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyLoadPresetRequest {
    pub instance_id: Option<String>,
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxySetParamByNameRequest {
    pub instance_id: Option<String>,
    pub name: String,
    pub value: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyGetParamsByNameRequest {
    pub instance_id: Option<String>,
    pub names: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyGetPatchStateRequest {
    pub instance_id: Option<String>,
    pub diff_only: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GuardAudioRoutingRequest {
    pub user_message: String,
    pub requested_tool: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchPluginDocsRequest {
    /// Plugin name to search docs for (e.g., "Vital", "Serum").
    pub plugin_name: String,
    /// Targeted question (e.g., "modulation routing", "filter envelope quirks").
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchSoundDesignRequest {
    /// Broad sound design topic or target outcome (e.g., "neuro bass", "reese bass", "vocal compression").
    pub topic: String,
    /// Optional deeper query to refine the recipe search.
    pub query: Option<String>,
}

const AUDIO_INTENT_THRESHOLD: f64 = 0.55;

const AUDIO_INTENT_TERMS: [(&str, f64); 28] = [
    ("vst", 1.0),
    ("plugin", 1.0),
    ("preset", 1.0),
    ("patch", 0.8),
    ("sound", 0.9),
    ("tone", 0.9),
    ("parameter", 0.9),
    ("knob", 0.9),
    ("automation", 0.9),
    ("compressor", 0.8),
    ("eq", 0.8),
    ("reverb", 0.8),
    ("attack", 0.7),
    ("release", 0.7),
    ("serum", 0.9),
    ("fabfilter", 0.9),
    ("pro-q", 0.8),
    ("pro q", 0.8),
    ("cutoff", 0.7),
    ("resonance", 0.7),
    ("harsh", 0.6),
    ("bright", 0.6),
    ("brighter", 0.6),
    ("tipper", 0.9),
    ("squelch", 0.9),
    ("psytrance", 0.9),
    ("acid", 0.8),
    ("reese", 0.9),
];

const DOCS_OR_NEWS_TERMS: [&str; 10] = [
    "docs",
    "documentation",
    "manual",
    "api reference",
    "release notes",
    "what's new",
    "changelog",
    "news",
    "latest update",
    "blog post",
];

const CODE_PATCH_TERMS: [&str; 8] = [
    "git patch",
    "code patch",
    "diff",
    "pull request",
    "commit",
    "apply patch",
    ".patch",
    ".diff",
];

const PARAMETER_TUNING_TERMS: [&str; 16] = [
    "parameter",
    "knob",
    "automation",
    "automate",
    "attack",
    "release",
    "threshold",
    "ratio",
    "cutoff",
    "resonance",
    "frequency",
    "q",
    "reverb",
    "brighter",
    "harsh",
    "less harsh",
];

const HARD_AUDIO_ROUTE_TERMS_NON_PATCH: [&str; 17] = [
    "vst",
    "plugin",
    "preset",
    "sound",
    "tone",
    "parameter",
    "knob",
    "automation",
    "automate",
    "compressor",
    "eq",
    "reverb",
    "synth",
    "squelch",
    "psytrance",
    "acid",
    "reese",
];

// ---- Doc search helpers (inline, no extra dependency) ----

const PLUGIN_DOCS_ENV: &str = "AGENTAUDIO_PLUGIN_DOCS_DIR";
const SOUND_GUIDE_ENV: &str = "AGENTAUDIO_SOUND_DESIGN_DIR";
const DEFAULT_PLUGIN_DOCS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/plugins");
const DEFAULT_SOUND_GUIDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/sound-design");
const DOC_MAX_EXCERPTS: usize = 3;
const DOC_MAX_EXCERPT_CHARS: usize = 600;

const DOC_STOPWORDS: [&str; 31] = [
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "into", "is",
    "it", "of", "on", "or", "that", "the", "their", "there", "these", "this", "to", "use", "using",
    "what", "with", "you", "your",
];

fn docs_base_dir(env_var: &str, default: &str) -> std::path::PathBuf {
    std::env::var(env_var)
        .ok()
        .map(|s| std::path::PathBuf::from(s.trim().to_string()))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from(default))
}

fn collect_md_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_md_files(&path));
            } else if matches!(
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .as_deref(),
                Some("md" | "txt" | "json")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn doc_tokenize(s: &str) -> Vec<String> {
    let stop: std::collections::HashSet<&str> = DOC_STOPWORDS.into_iter().collect();
    let mut terms: Vec<String> = s
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2 && !stop.contains(*t))
        .map(|t| t.to_string())
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

fn score_doc_chunk(chunk: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let lower = chunk.to_lowercase();
    let mut unique_hits = 0usize;
    let mut total_hits = 0usize;
    for term in terms {
        let n = if term.len() <= 2 {
            lower
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|tok| *tok == term.as_str())
                .count()
        } else {
            lower.matches(term.as_str()).count()
        };
        if n > 0 {
            unique_hits += 1;
            total_hits += n;
        }
    }
    unique_hits * 20 + total_hits
}

fn truncate_doc_excerpt(s: &str) -> String {
    if s.chars().count() <= DOC_MAX_EXCERPT_CHARS {
        return s.to_string();
    }
    let mut idx = 0;
    let mut cnt = 0;
    for (i, _) in s.char_indices() {
        if cnt == DOC_MAX_EXCERPT_CHARS {
            break;
        }
        idx = i;
        cnt += 1;
    }
    let mut out = s[..=idx].to_string();
    if let Some(sp) = out.rfind(' ') {
        out.truncate(sp);
    }
    out.push_str("...");
    out
}

/// Score paragraphs in `files` against `terms`. Returns (score, source_filename, excerpt) sorted desc.
fn score_files_for_excerpts(
    files: &[std::path::PathBuf],
    terms: &[String],
) -> Vec<(usize, String, String)> {
    let mut scored: Vec<(usize, String, String)> = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let source = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        for chunk in text.replace("\r\n", "\n").split("\n\n") {
            let clean: String = chunk
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if clean.is_empty() {
                continue;
            }
            let s = score_doc_chunk(&clean, terms);
            if s > 0 {
                scored.push((s, source.clone(), truncate_doc_excerpt(&clean)));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);
    scored
}

fn plugin_key(name: &str) -> String {
    name.to_ascii_lowercase().replace([' ', '-', '_'], "")
}

fn plugin_docs_exist(plugin_name: &str) -> bool {
    let dir = docs_base_dir(PLUGIN_DOCS_ENV, DEFAULT_PLUGIN_DOCS);
    let key = plugin_key(plugin_name);
    collect_md_files(&dir).iter().any(|p| {
        let stem = plugin_key(p.file_stem().and_then(|s| s.to_str()).unwrap_or(""));
        stem.contains(&key) || key.contains(&stem)
    })
}

fn search_plugin_docs_impl(plugin_name: &str, query: &str) -> Result<serde_json::Value, String> {
    let dir = docs_base_dir(PLUGIN_DOCS_ENV, DEFAULT_PLUGIN_DOCS);
    let all_files = collect_md_files(&dir);
    if all_files.is_empty() {
        return Err(format!(
            "No plugin docs found in '{}'. Set {} or add .md files.",
            dir.display(),
            PLUGIN_DOCS_ENV
        ));
    }
    let key = plugin_key(plugin_name);
    let files: Vec<_> = all_files
        .iter()
        .filter(|p| {
            let stem = plugin_key(p.file_stem().and_then(|s| s.to_str()).unwrap_or(""));
            stem.contains(&key) || key.contains(&stem)
        })
        .cloned()
        .collect();
    if files.is_empty() {
        let available: Vec<_> = all_files
            .iter()
            .filter_map(|p| p.file_stem()?.to_str().map(|s| s.to_string()))
            .collect();
        return Err(format!(
            "No docs matched plugin '{}'. Available: [{}]. Add docs/plugins/{}.md or set {}.",
            plugin_name,
            available.join(", "),
            plugin_name,
            PLUGIN_DOCS_ENV
        ));
    }
    let terms = doc_tokenize(query);
    let scored = score_files_for_excerpts(&files, &terms);
    if scored.is_empty() {
        return Err(format!(
            "No excerpts matched query '{}' in {plugin_name} docs. Try a broader query or use find_vst_parameter directly.",
            query
        ));
    }
    let top: Vec<_> = scored.into_iter().take(DOC_MAX_EXCERPTS).collect();
    Ok(serde_json::json!({
        "plugin_name": plugin_name,
        "query": query,
        "excerpts": top.iter().map(|(_, _, text)| text).collect::<Vec<_>>(),
        "sources": top.iter().map(|(_, src, _)| src).collect::<Vec<_>>(),
    }))
}

fn search_sound_design_impl(topic: &str, query: Option<&str>) -> Result<serde_json::Value, String> {
    let dir = docs_base_dir(SOUND_GUIDE_ENV, DEFAULT_SOUND_GUIDE);
    let files = collect_md_files(&dir);
    if files.is_empty() {
        return Err(format!(
            "No sound design guides found in '{}'. Set {} or add .md files.",
            dir.display(),
            SOUND_GUIDE_ENV
        ));
    }
    let search = match query {
        Some(q) => format!("{topic} {q}"),
        None => topic.to_string(),
    };
    let terms = doc_tokenize(&search);
    let scored = score_files_for_excerpts(&files, &terms);
    if scored.is_empty() {
        return Err(format!(
            "No sound design guide excerpts matched '{topic}'. Try a different topic or use find_vst_parameter directly."
        ));
    }
    // Group by source, pick the highest-scoring guide
    let mut by_source: std::collections::HashMap<String, Vec<(usize, String)>> =
        std::collections::HashMap::new();
    for (score, source, text) in scored {
        by_source.entry(source).or_default().push((score, text));
    }
    let mut ranked: Vec<(usize, String, Vec<String>)> = by_source
        .into_iter()
        .map(|(source, mut entries)| {
            entries.sort_by(|a, b| b.0.cmp(&a.0));
            let total: usize = entries.iter().take(DOC_MAX_EXCERPTS).map(|(s, _)| *s).sum();
            let texts: Vec<String> = entries
                .into_iter()
                .take(DOC_MAX_EXCERPTS)
                .map(|(_, t)| t)
                .collect();
            (total, source, texts)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    let (_, guide, excerpts) = ranked.into_iter().next().unwrap();
    Ok(serde_json::json!({
        "topic": topic,
        "guide": guide,
        "excerpts": excerpts,
        "step_by_step_recipe": excerpts.join("\n\n"),
    }))
}

// ---- Audio intent routing helpers ----

fn contains_any(lower: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| contains_term(lower, term))
}

fn contains_term(lower: &str, term: &str) -> bool {
    if term.len() <= 2 {
        lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token == term)
    } else {
        lower.contains(term)
    }
}

/// Returns (primary_terms, alias_terms) separately so scoring can weight them differently.
fn query_terms(query: &str) -> (Vec<String>, Vec<String>) {
    let lower = query.to_lowercase();
    let primary: Vec<String> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
        .collect();

    let mut aliases: Vec<String> = Vec::new();
    if lower.contains("brighter") {
        aliases.extend(
            ["bright", "brightness", "treble", "presence"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    if lower.contains("harsh") {
        aliases.extend(
            ["harsh", "resonance", "q", "presence"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    if lower.contains("reverb") {
        // Omit "decay" and "mix" — too ambiguous (match unrelated Envelope/Volume params)
        aliases.extend(["room", "wet"].iter().map(|s| s.to_string()));
    }

    // Remove aliases that duplicate primary terms
    aliases.retain(|a| !primary.contains(a));
    aliases.sort();
    aliases.dedup();
    (primary, aliases)
}

/// Score a param against primary and alias terms. Returns 0 if no match.
/// Higher score = better match. Primary terms outweigh aliases.
fn score_param(param: &serde_json::Value, primary: &[String], aliases: &[String]) -> u32 {
    let name = param
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let name_words: Vec<&str> = name.split_whitespace().collect();
    let display = param
        .get("display")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let mut score = 0u32;

    for term in primary {
        if name_words.iter().any(|w| *w == term.as_str()) {
            score += 100;
        }
        // exact word in name
        else if name.starts_with(term.as_str()) {
            score += 80;
        }
        // name prefix
        else if name.contains(term.as_str()) {
            score += 40;
        } // substring in name
        if display.contains(term.as_str()) {
            score += 5;
        } // in display value
    }
    for term in aliases {
        if name_words.iter().any(|w| *w == term.as_str()) {
            score += 10;
        } else if name.contains(term.as_str()) {
            score += 3;
        }
    }
    score
}

fn parse_params_from_list_result(raw: &str) -> Result<Vec<serde_json::Value>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse list_params JSON: {e}"))?;

    if let Some(arr) = parsed.as_array() {
        return Ok(arr.clone());
    }

    let params = parsed
        .get("parameters")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "list_params response is missing a 'parameters' array".to_string())?;
    Ok(params.clone())
}

fn audio_intent_analysis(user_message: &str) -> (f64, Vec<String>, &'static str) {
    let lower = user_message.to_lowercase();
    let mut score = 0.0;
    let mut matched_terms: Vec<String> = Vec::new();

    for (term, weight) in AUDIO_INTENT_TERMS {
        if lower.contains(term) {
            score += weight;
            matched_terms.push(term.to_string());
        }
    }

    let patch_mentioned = lower.contains("patch");
    let code_patch_context = patch_mentioned && contains_any(&lower, &CODE_PATCH_TERMS);
    let strong_audio_context = contains_any(
        &lower,
        &[
            "vst",
            "plugin",
            "preset",
            "sound",
            "tone",
            "parameter",
            "knob",
            "automation",
        ],
    );

    let patch_interpretation = if patch_mentioned && code_patch_context && !strong_audio_context {
        // "patch" can mean git/code patch; avoid false audio routing in that case.
        score = (score - 0.9).max(0.0);
        "code_patch"
    } else if patch_mentioned {
        score += 0.3;
        "audio_patch"
    } else {
        "none"
    };

    let confidence = (score / 3.0).clamp(0.0, 1.0);
    matched_terms.sort();
    matched_terms.dedup();
    (confidence, matched_terms, patch_interpretation)
}

fn choose_audio_first_tool(user_message: &str) -> &'static str {
    let lower = user_message.to_lowercase();
    if contains_any(&lower, &PARAMETER_TUNING_TERMS) {
        "find_vst_parameter"
    } else {
        "list_instances"
    }
}

fn hard_audio_route_trigger(user_message: &str, patch_interpretation: &str) -> bool {
    let lower = user_message.to_lowercase();
    contains_any(&lower, &HARD_AUDIO_ROUTE_TERMS_NON_PATCH)
        || (lower.contains("patch") && patch_interpretation == "audio_patch")
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let now_ms = now_ms();
    let mut reg = state.registry.write().await;
    reg.insert(
        req.instance_id.clone(),
        RegisteredInstance {
            instance_id: req.instance_id,
            endpoint: req.endpoint,
            mcp_name: req.mcp_name,
            last_seen_ms: now_ms,
        },
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "registered" })),
    )
}

async fn heartbeat(
    State(state): State<AppState>,
    Json(req): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    let now_ms = now_ms();
    let mut reg = state.registry.write().await;
    if let Some(inst) = reg.get_mut(&req.instance_id) {
        inst.last_seen_ms = now_ms;
        if let Some(name) = req.mcp_name {
            inst.mcp_name = name;
        }
        (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "status": "unknown_instance" })),
        )
    }
}

async fn unregister(
    State(state): State<AppState>,
    Json(req): Json<UnregisterRequest>,
) -> impl IntoResponse {
    let mut reg = state.registry.write().await;
    reg.remove(&req.instance_id);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "unregistered" })),
    )
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let reg = state.registry.read().await;
    let instance_count = reg.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "instance_count": instance_count,
        })),
    )
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_string(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn format_structured_error(error: &str, message: &str) -> String {
    serde_json::json!({ "error": error, "message": message }).to_string()
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone)]
struct RouterMcpServer {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

impl RouterMcpServer {
    fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    async fn resolve_instance_id(&self, instance_id: Option<String>) -> Result<String, String> {
        if let Some(id) = instance_id {
            return Ok(id);
        }

        if let Some(id) = self.state.default_instance_id.read().await.clone() {
            return Ok(id);
        }

        let reg = self.state.registry.read().await;
        match reg.len() {
            0 => Err("No wrapper instances registered. Start your DAW and insert the wrapper, or register manually via POST /register.".to_string()),
            1 => Ok(reg.keys().next().cloned().unwrap_or_default()),
            _ => Err("Multiple wrapper instances registered. Provide instance_id (or call select_instance).".to_string()),
        }
    }

    async fn endpoint_for(&self, instance_id: &str) -> Result<String, String> {
        let reg = self.state.registry.read().await;
        reg.get(instance_id)
            .map(|i| i.endpoint.clone())
            .ok_or_else(|| {
                format_structured_error(
                    "UnknownInstance",
                    &format!(
                        "Unknown instance_id '{instance_id}'. Call list_instances to get valid IDs."
                    ),
                )
            })
    }

    #[instrument(skip(self, arguments), fields(instance_id = %instance_id, tool = %tool_name))]
    async fn call_wrapper_tool(
        &self,
        instance_id: &str,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<String, String> {
        const WRAPPER_TIMEOUT: Duration = Duration::from_secs(30);
        const MAX_ATTEMPTS: u32 = 2;

        let endpoint = self.endpoint_for(instance_id).await?;

        let mut last_err = String::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let result = tokio::time::timeout(
                WRAPPER_TIMEOUT,
                async {
                    let transport =
                        rmcp::transport::StreamableHttpClientTransport::from_uri(endpoint.clone());
                    let service = ()
                        .serve(transport)
                        .await
                        .map_err(|e| format!("Failed to connect to wrapper MCP endpoint: {e}"))?;

                    let result = service
                        .call_tool(CallToolRequestParams {
                            meta: None,
                            name: Cow::Owned(tool_name.to_string()),
                            arguments: arguments.clone(),
                            task: None,
                        })
                        .await
                        .map_err(|e| format!("Wrapper tool call failed: {e}"))?;

                    let _ = service.cancel().await;
                    Ok::<_, String>(result)
                },
            )
            .await;

            match result {
                Ok(Ok(res)) => return call_tool_result_to_text(res),
                Ok(Err(e)) => {
                    last_err = e;
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
                Err(_) => {
                    last_err = format_structured_error(
                        "Timeout",
                        &format!(
                            "Wrapper call timed out after {:?}. Instance {} may be unresponsive.",
                            WRAPPER_TIMEOUT, instance_id
                        ),
                    );
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    #[instrument(skip(self), fields(instance_id = %instance_id))]
    async fn get_cached_params_or_fetch(&self, instance_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let now = Instant::now();
        {
            let cache = self.state.param_cache.read().await;
            if let Some((params, cached_at)) = cache.get(instance_id) {
                if now.duration_since(*cached_at) < PARAM_CACHE_TTL {
                    tracing::info!(cache_hit = true, param_count = params.len(), "param cache hit");
                    return Ok(params.clone());
                }
            }
        }
        tracing::info!(cache_hit = false, "param cache miss, fetching list_params");
        let raw = self.call_wrapper_tool(instance_id, "list_params", None).await?;
        let params = parse_params_from_list_result(&raw)?;
        {
            let mut cache = self.state.param_cache.write().await;
            cache.insert(instance_id.to_string(), (params.clone(), now));
        }
        Ok(params)
    }

    async fn invalidate_param_cache(&self, instance_id: &str) {
        let mut cache = self.state.param_cache.write().await;
        cache.remove(instance_id);
    }
}

#[tool_router]
impl RouterMcpServer {
    #[tool(description = "Get router daemon status.")]
    async fn router_status(&self) -> Result<String, String> {
        let reg = self.state.registry.read().await;
        let default_instance_id = self.state.default_instance_id.read().await.clone();

        let response = serde_json::json!({
            "status": "ok",
            "uptime_ms": self.state.started_at.elapsed().as_millis(),
            "instance_count": reg.len(),
            "default_instance_id": default_instance_id,
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "List registered wrapper instances and their endpoints.")]
    async fn list_instances(&self) -> Result<String, String> {
        let reg = self.state.registry.read().await;
        let mut instances: Vec<_> = reg.values().cloned().collect();
        instances.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        serde_json::to_string_pretty(&instances).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Set a default instance_id for subsequent proxy calls (process-global).")]
    async fn select_instance(
        &self,
        Parameters(req): Parameters<SelectInstanceRequest>,
    ) -> Result<String, String> {
        // Validate it exists.
        let _ = self.endpoint_for(&req.instance_id).await?;
        *self.state.default_instance_id.write().await = Some(req.instance_id.clone());
        Ok(format!(
            "{{\"status\":\"selected\",\"instance_id\":\"{}\"}}",
            req.instance_id
        ))
    }

    // ---- Proxy tools ----
    // Scan and load are not exposed. MCP only operates on running instances; user loads plugins in the wrapper GUI.
    // First step is param introspection (list_params, list_param_groups, find_vst_parameter).

    #[tool(description = "Unload currently loaded child plugin.")]
    async fn unload_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "unload_child_plugin", None)
            .await
    }

    #[tool(description = "Open child plugin editor window.")]
    async fn open_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "open_child_editor", None).await
    }

    #[tool(description = "Close child plugin editor window.")]
    async fn close_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "close_child_editor", None)
            .await
    }

    #[tool(
        description = "List plugin parameters/knobs and current values. Supports optional prefix filter (e.g. prefix='Reverb') to narrow results. Use list_param_groups to discover valid prefixes."
    )]
    async fn list_params(
        &self,
        Parameters(req): Parameters<ProxyListParamsRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = if req.prefix.is_some() {
            serde_json::json!({ "prefix": req.prefix })
                .as_object()
                .cloned()
        } else {
            None
        };
        self.call_wrapper_tool(&id, "list_params", args).await
    }

    #[tool(
        description = "Search parameters by exact name substring. Faster and more precise than find_vst_parameter when you know the param name."
    )]
    async fn search_params(
        &self,
        Parameters(req): Parameters<ProxySearchParamsRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "query": req.query })
            .as_object()
            .cloned();
        self.call_wrapper_tool(&id, "search_params", args).await
    }

    #[tool(
        description = "List logical parameter groups available in the loaded plugin (e.g. 'Reverb', 'Envelope 1', 'Filter 1'). Use before list_params or find_vst_parameter to discover available sections."
    )]
    async fn list_param_groups(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "list_param_groups", None).await
    }

    #[tool(
        description = "Set one realtime parameter value by id. Use for single knob/parameter/tone tweaks."
    )]
    async fn set_param_realtime(
        &self,
        Parameters(req): Parameters<ProxySetParamRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "id": req.id, "value": req.value })
            .as_object()
            .cloned();
        self.call_wrapper_tool(&id, "set_param_realtime", args)
            .await
    }

    #[tool(
        description = "Set multiple realtime parameters in one call. Use for coordinated tone/preset/patch edits."
    )]
    async fn batch_set_realtime(
        &self,
        Parameters(req): Parameters<ProxyBatchSetRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({
            "changes": req.changes.into_iter().map(|c| serde_json::json!({"id": c.id, "value": c.value})).collect::<Vec<_>>()
        })
        .as_object()
        .cloned();
        self.call_wrapper_tool(&id, "batch_set_realtime", args)
            .await
    }

    #[tool(
        description = "Alias for batch_set_realtime. Edit VST patch/preset/sound by applying multiple parameter changes."
    )]
    async fn edit_vst_patch(
        &self,
        Parameters(req): Parameters<ProxyBatchSetRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({
            "changes": req.changes.into_iter().map(|c| serde_json::json!({"id": c.id, "value": c.value})).collect::<Vec<_>>()
        })
        .as_object()
        .cloned();
        self.call_wrapper_tool(&id, "batch_set_realtime", args)
            .await
    }

    #[tool(
        description = "Search plugin parameters by natural language (e.g. 'attack', 'release', 'make brighter', 'reduce reverb'). Results are ranked by relevance."
    )]
    async fn find_vst_parameter(
        &self,
        Parameters(req): Parameters<ProxyFindVstParameterRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let params = self.get_cached_params_or_fetch(&id).await?;
        let source_count = params.len();
        let (primary, aliases) = query_terms(&req.query);
        let limit = req.limit.unwrap_or(20).max(1);

        let mut scored: Vec<(u32, serde_json::Value)> = params
            .into_iter()
            .filter_map(|p| {
                let s = score_param(&p, &primary, &aliases);
                if s > 0 { Some((s, p)) } else { None }
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let matches: Vec<serde_json::Value> =
            scored.into_iter().take(limit).map(|(_, p)| p).collect();

        let mut all_terms: Vec<String> = primary.iter().chain(aliases.iter()).cloned().collect();
        all_terms.sort();
        all_terms.dedup();

        let response = serde_json::json!({
            "query": req.query,
            "terms": all_terms,
            "count": matches.len(),
            "source_count": source_count,
            "matches": matches,
            "next_step": "Call get_param_info on target ids to understand ranges, then set_param_by_name or batch_set_realtime to apply. If you haven't called search_plugin_docs yet, do that first — it contains critical quirks.",
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Preview current values for selected parameters before editing a patch/preset/tone. Optionally pass specific ids."
    )]
    async fn preview_vst_parameter_values(
        &self,
        Parameters(req): Parameters<ProxyPreviewVstParameterValuesRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let params = self.get_cached_params_or_fetch(&id).await?;
        let limit = req.limit.unwrap_or(20).max(1);

        let selected: Vec<serde_json::Value> = if let Some(ids) = req.ids {
            params
                .iter()
                .filter(|p| {
                    p.get("id")
                        .and_then(|v| v.as_u64())
                        .map(|id| ids.contains(&(id as u32)))
                        .unwrap_or(false)
                })
                .take(limit)
                .cloned()
                .collect()
        } else {
            params.iter().take(limit).cloned().collect()
        };

        let response = serde_json::json!({
            "count": selected.len(),
            "values": selected,
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Deterministic routing guardrail. Run before WebSearch: if audio intent confidence is high and user did not explicitly ask for docs/news, block web and route to audio tools."
    )]
    async fn guard_audio_routing(
        &self,
        Parameters(req): Parameters<GuardAudioRoutingRequest>,
    ) -> Result<String, String> {
        let (confidence, matched_terms, patch_interpretation) =
            audio_intent_analysis(&req.user_message);
        let hard_trigger = hard_audio_route_trigger(&req.user_message, patch_interpretation);
        let requested_tool = req.requested_tool.unwrap_or_default();
        let explicit_docs_or_news =
            contains_any(&req.user_message.to_lowercase(), &DOCS_OR_NEWS_TERMS)
                || contains_any(&requested_tool.to_lowercase(), &DOCS_OR_NEWS_TERMS);
        let block_web_search =
            (confidence >= AUDIO_INTENT_THRESHOLD || hard_trigger) && !explicit_docs_or_news;
        let recommended_first_tool = if block_web_search {
            choose_audio_first_tool(&req.user_message)
        } else {
            "none"
        };

        let response = serde_json::json!({
            "audio_intent_confidence": confidence,
            "threshold": AUDIO_INTENT_THRESHOLD,
            "hard_audio_trigger": hard_trigger,
            "matched_terms": matched_terms,
            "patch_interpretation": patch_interpretation,
            "explicit_docs_or_news_request": explicit_docs_or_news,
            "block_web_search": block_web_search,
            "recommended_route": if block_web_search { "audio_mcp" } else { "web_or_general" },
            "recommended_first_tool": recommended_first_tool,
            "recommended_workflow": [
                "1. list_instances — see running wrapper instances (plugin already loaded in wrapper GUI)",
                "2. Param introspection: list_params, list_param_groups, or find_vst_parameter",
                "3. search_plugin_docs (plugin-specific quirks and parameter mappings — do BEFORE editing)",
                "4. search_sound_design_guide (step-by-step recipe if user has a sound goal)",
                "5. get_param_info (probe range for a specific id before editing)",
                "6. set_param_by_name OR set_param_realtime/batch_set_realtime/edit_vst_patch",
                "7. save_preset (persist to .vstpreset when done)"
            ],
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Get param queue utilization. Use to detect when the queue is full and param changes are being dropped."
    )]
    async fn param_queue_status(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "param_queue_status", None).await
    }

    #[tool(description = "Get wrapper status and endpoint details.")]
    async fn wrapper_status(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "wrapper_status", None).await
    }

    #[tool(
        description = "Get parameter metadata and display range probe by id. Use before setting values to understand the range."
    )]
    async fn get_param_info(
        &self,
        Parameters(req): Parameters<ProxyGetParamInfoRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "id": req.id }).as_object().cloned();
        self.call_wrapper_tool(&id, "get_param_info", args).await
    }

    #[tool(
        description = "Save current plugin state to a .vstpreset file. Call after patch/preset edits to persist changes."
    )]
    async fn save_preset(
        &self,
        Parameters(req): Parameters<ProxySavePresetRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "path": req.path }).as_object().cloned();
        self.call_wrapper_tool(&id, "save_preset", args).await
    }

    #[tool(
        description = "Load plugin state from a .vstpreset file. Requires a plugin already loaded in the instance."
    )]
    async fn load_preset(
        &self,
        Parameters(req): Parameters<ProxyLoadPresetRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "path": req.path }).as_object().cloned();
        let result = self.call_wrapper_tool(&id, "load_preset", args).await;
        if result.is_ok() {
            self.invalidate_param_cache(&id).await;
        }
        result
    }

    #[tool(
        description = "Set a plugin parameter by name instead of numeric id. Uses case-insensitive fuzzy match. Returns resolved id and applied value."
    )]
    async fn set_param_by_name(
        &self,
        Parameters(req): Parameters<ProxySetParamByNameRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "name": req.name, "value": req.value })
            .as_object()
            .cloned();
        self.call_wrapper_tool(&id, "set_param_by_name", args).await
    }

    #[tool(
        description = "Batch lookup of parameter IDs by name (fuzzy match). Returns best match for each query."
    )]
    async fn get_params_by_name(
        &self,
        Parameters(req): Parameters<ProxyGetParamsByNameRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "names": req.names })
            .as_object()
            .cloned();
        self.call_wrapper_tool(&id, "get_params_by_name", args)
            .await
    }

    #[tool(description = "Get current patch state (all non-default parameters).")]
    async fn get_current_patch_state(
        &self,
        Parameters(req): Parameters<ProxyGetPatchStateRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        // Map request to wrapper tool "get_patch_state"
        let args = serde_json::json!({ "diff_only": req.diff_only })
            .as_object()
            .cloned();
        self.call_wrapper_tool(&id, "get_patch_state", args).await
    }

    #[tool(
        description = "Search local plugin documentation for plugin-specific quirks, parameter mappings, and routing notes (e.g. 'Vital modulation matrix', 'Serum LFO routing'). Call BEFORE editing an unfamiliar plugin — docs contain critical quirks not discoverable via parameter search alone. Returns targeted excerpts."
    )]
    async fn search_plugin_docs(
        &self,
        Parameters(req): Parameters<SearchPluginDocsRequest>,
    ) -> Result<String, String> {
        let result = search_plugin_docs_impl(&req.plugin_name, &req.query)?;
        serde_json::to_string_pretty(&result).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Search local sound design guides for step-by-step recipes (e.g. 'neuro bass', 'reese bass', 'vocal compression chain'). Call BEFORE parameter editing when the user describes a sound goal — the recipe tells you which parameters to target and in what order."
    )]
    async fn search_sound_design_guide(
        &self,
        Parameters(req): Parameters<SearchSoundDesignRequest>,
    ) -> Result<String, String> {
        let result = search_sound_design_impl(&req.topic, req.query.as_deref())?;
        serde_json::to_string_pretty(&result).map_err(|e| format!("Serialization failed: {e}"))
    }
}

#[tool_handler]
impl ServerHandler for RouterMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "AgentAudio MCP router.\n\
Hard routing rule: If user mentions VST/plugin/preset/patch/sound/tone/parameter/knob/automation, use Audio MCP tools first. Do not use web search unless user explicitly asks for docs/news.\n\
Disambiguation: In audio context, patch = preset/sound configuration, not code diff.\n\
Run guard_audio_routing before any web search call.\n\
Recommended workflow:\n\
  Operate on running instances only (load plugins in the wrapper GUI).\n\
  1. list_instances — see which wrapper instances are running and their loaded plugin\n\
  2. Param introspection first: list_params, list_param_groups, or find_vst_parameter\n\
  3. search_plugin_docs — before editing: plugin-specific quirks and parameter mappings\n\
  4. search_sound_design_guide — when user has a sound goal (e.g. neuro bass, reese bass)\n\
  5. get_param_info — probe display range for a parameter id before editing\n\
  6. set_param_by_name OR set_param_realtime / batch_set_realtime / edit_vst_patch\n\
  7. save_preset — persist to .vstpreset when done"
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Parse a `load_child_plugin` JSON response and inject a `docs_hint` field indicating
/// whether local plugin docs exist and what tool to call next.
fn inject_docs_hint(raw: String) -> Result<String, String> {
    if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&raw) {
        if let Some(name) = val
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
        {
            let hint = if plugin_docs_exist(&name) {
                format!(
                    "Plugin docs found — call search_plugin_docs(\"{name}\", \"<your question>\") BEFORE editing parameters.",
                )
            } else {
                format!(
                    "No local docs for '{name}'. Explore with find_vst_parameter, or add docs/plugins/{name}.md for future sessions.",
                )
            };
            val["docs_hint"] = serde_json::json!(hint);
            return serde_json::to_string_pretty(&val)
                .map_err(|e| format!("Serialization failed: {e}"));
        }
    }
    Ok(raw)
}

fn call_tool_result_to_text(result: CallToolResult) -> Result<String, String> {
    if result.is_error.unwrap_or(false) {
        if let Some(v) = result.structured_content {
            let s = v.to_string();
            if s.trim_start().starts_with('{') {
                return Err(s);
            }
            return Err(format_structured_error("ToolError", &s));
        }
        let msg = contents_to_text(&result.content);
        return Err(format_structured_error(
            "ToolError",
            &if msg.is_empty() {
                "Wrapper tool returned an error.".to_string()
            } else {
                msg
            },
        ));
    }

    if let Some(v) = result.structured_content {
        return serde_json::to_string_pretty(&v).map_err(|e| format!("Serialization failed: {e}"));
    }

    Ok(contents_to_text(&result.content))
}

fn contents_to_text(content: &[Content]) -> String {
    let mut out = String::new();
    for c in content {
        if let Some(t) = c.raw.as_text() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&t.text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize)]
    struct RoutingEvalCase {
        prompt: String,
        expected_block_web_search: bool,
        expected_first_tool: String,
    }

    #[test]
    fn patch_disambiguation_prefers_audio_context() {
        let (confidence, _, patch_kind) =
            audio_intent_analysis("Edit patch in Serum and make the tone brighter");
        assert!(confidence >= AUDIO_INTENT_THRESHOLD);
        assert_eq!(patch_kind, "audio_patch");
    }

    #[test]
    fn patch_disambiguation_detects_code_context() {
        let (confidence, _, patch_kind) =
            audio_intent_analysis("Review this git patch and apply the diff");
        assert!(confidence < AUDIO_INTENT_THRESHOLD);
        assert_eq!(patch_kind, "code_patch");
    }

    #[test]
    fn audio_routing_eval_suite() {
        let corpus = include_str!("../evals/audio_routing_first_tool.json");
        let cases: Vec<RoutingEvalCase> =
            serde_json::from_str(corpus).expect("routing eval corpus must be valid JSON");
        assert!(cases.len() >= 20, "expected at least 20 eval prompts");

        for case in cases {
            let (confidence, _, patch_interpretation) = audio_intent_analysis(&case.prompt);
            let hard_trigger = hard_audio_route_trigger(&case.prompt, patch_interpretation);
            let explicit_docs_or_news =
                contains_any(&case.prompt.to_lowercase(), &DOCS_OR_NEWS_TERMS);
            let block_web_search =
                (confidence >= AUDIO_INTENT_THRESHOLD || hard_trigger) && !explicit_docs_or_news;
            assert_eq!(
                block_web_search, case.expected_block_web_search,
                "block_web_search mismatch for prompt: {}",
                case.prompt
            );

            let first_tool = if block_web_search {
                choose_audio_first_tool(&case.prompt)
            } else {
                "none"
            };
            assert_eq!(
                first_tool, case.expected_first_tool,
                "first tool mismatch for prompt: {}",
                case.prompt
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let bind = env_string("AGENTAUDIO_MCP_ROUTERD_BIND", "127.0.0.1:38765");
    let bind: SocketAddr = bind.parse()?;

    let ttl_ms = env_u64("AGENTAUDIO_MCP_ROUTERD_TTL_MS", 15_000);
    let prune_every_ms = env_u64("AGENTAUDIO_MCP_ROUTERD_PRUNE_EVERY_MS", 2_000);

    let registry: SharedRegistry = Arc::new(RwLock::new(HashMap::new()));
    let state = AppState {
        registry: Arc::clone(&registry),
        default_instance_id: Arc::new(RwLock::new(None)),
        param_cache: Arc::new(RwLock::new(HashMap::new())),
        started_at: Instant::now(),
    };

    let cancel = CancellationToken::new();
    let prune_cancel = cancel.child_token();
    let prune_state = state.clone();
    tokio::spawn(async move {
        let ttl = Duration::from_millis(ttl_ms);
        let every = Duration::from_millis(prune_every_ms);
        loop {
            tokio::select! {
                _ = prune_cancel.cancelled() => break,
                _ = tokio::time::sleep(every) => {
                    let cutoff_ms = now_ms().saturating_sub(ttl.as_millis() as u64);
                    let mut reg = prune_state.registry.write().await;
                    reg.retain(|_, inst| inst.last_seen_ms >= cutoff_ms);
                }
            }
        }
    });

    let state_for_mcp = state.clone();
    let mcp_service: rmcp::transport::streamable_http_server::tower::StreamableHttpService<
        RouterMcpServer,
        rmcp::transport::streamable_http_server::session::local::LocalSessionManager,
    > = rmcp::transport::streamable_http_server::tower::StreamableHttpService::new(
        move || Ok(RouterMcpServer::new(state_for_mcp.clone())),
        Default::default(),
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig {
            stateful_mode: true,
            sse_keep_alive: None,
            cancellation_token: cancel.child_token(),
            ..Default::default()
        },
    );

    let app = Router::new()
        .route("/health", get(health))
        .route("/register", post(register))
        .route("/heartbeat", post(heartbeat))
        .route("/unregister", post(unregister))
        .nest_service("/mcp", mcp_service)
        .with_state(state);

    tracing::info!("agentaudio-mcp-routerd listening on http://{bind}");
    axum::serve(tokio::net::TcpListener::bind(bind).await?, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        })
        .await?;

    Ok(())
}
