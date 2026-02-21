use std::{
    borrow::Cow,
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
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
}

#[derive(Debug, Deserialize)]
struct UnregisterRequest {
    instance_id: String,
}

#[derive(Clone)]
struct AppState {
    registry: SharedRegistry,
    default_instance_id: Arc<RwLock<Option<String>>>,
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
struct GuardAudioRoutingRequest {
    pub user_message: String,
    pub requested_tool: Option<String>,
}

const AUDIO_INTENT_THRESHOLD: f64 = 0.55;

const AUDIO_INTENT_TERMS: [(&str, f64); 23] = [
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

const HARD_AUDIO_ROUTE_TERMS_NON_PATCH: [&str; 13] = [
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
];

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

fn query_terms(query: &str) -> Vec<String> {
    let lower = query.to_lowercase();
    let mut terms: Vec<String> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
        .collect();

    if lower.contains("brighter") {
        terms.extend(
            ["bright", "brightness", "high", "treble", "presence"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    if lower.contains("harsh") {
        terms.extend(
            ["harsh", "resonance", "q", "high", "presence"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    if lower.contains("reverb") {
        terms.extend(
            ["reverb", "decay", "room", "wet", "mix"]
                .iter()
                .map(|s| s.to_string()),
        );
    }

    terms.sort();
    terms.dedup();
    terms
}

fn parameter_matches_query(param: &serde_json::Value, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }

    let name = param
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let display = param
        .get("display")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    let haystack = format!("{name} {display}");

    terms.iter().any(|term| contains_term(&haystack, term))
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
        "scan_plugins"
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

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_string(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
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
            .ok_or_else(|| format!("Unknown instance_id '{instance_id}'. Call list_instances."))
    }

    async fn call_wrapper_tool(
        &self,
        instance_id: &str,
        tool_name: &str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<String, String> {
        let endpoint = self.endpoint_for(instance_id).await?;

        let transport = rmcp::transport::StreamableHttpClientTransport::from_uri(endpoint);
        let service = ()
            .serve(transport)
            .await
            .map_err(|e| format!("Failed to connect to wrapper MCP endpoint: {e}"))?;

        let result = service
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Owned(tool_name.to_string()),
                arguments,
                task: None,
            })
            .await
            .map_err(|e| format!("Wrapper tool call failed: {e}"))?;

        let _ = service.cancel().await;
        call_tool_result_to_text(result)
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

    #[tool(
        description = "Scan installed VST plugins. Use first for requests mentioning plugin/VST/synth/preset/patch/sound/tone."
    )]
    async fn scan_plugins(
        &self,
        Parameters(req): Parameters<ProxyScanPluginsRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "path": req.path }).as_object().cloned();
        self.call_wrapper_tool(&id, "scan_plugins", args).await
    }

    #[tool(
        description = "Load a child VST plugin by UID after scan_plugins. Use for requests like 'load Serum' or 'open FabFilter plugin'."
    )]
    async fn load_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyLoadChildRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "uid": req.uid }).as_object().cloned();
        self.call_wrapper_tool(&id, "load_child_plugin", args).await
    }

    #[tool(description = "Alias for load_child_plugin. Natural-language name for 'load plugin'.")]
    async fn load_plugin(
        &self,
        Parameters(req): Parameters<ProxyLoadChildRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "uid": req.uid }).as_object().cloned();
        self.call_wrapper_tool(&id, "load_child_plugin", args).await
    }

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
        description = "List plugin parameters/knobs and current values. Use when user says parameter/knob/automation/make brighter/reduce reverb."
    )]
    async fn list_params(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "list_params", None).await
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
        description = "Search plugin parameters by natural language (e.g. 'attack', 'release', 'make brighter', 'reduce reverb')."
    )]
    async fn find_vst_parameter(
        &self,
        Parameters(req): Parameters<ProxyFindVstParameterRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let raw = self.call_wrapper_tool(&id, "list_params", None).await?;
        let params = parse_params_from_list_result(&raw)?;
        let terms = query_terms(&req.query);
        let limit = req.limit.unwrap_or(20).max(1);

        let matches: Vec<serde_json::Value> = params
            .iter()
            .filter(|p| parameter_matches_query(p, &terms))
            .take(limit)
            .cloned()
            .collect();

        let response = serde_json::json!({
            "query": req.query,
            "terms": terms,
            "count": matches.len(),
            "source_count": params.len(),
            "matches": matches,
            "next_step": "Use preview_vst_parameter_values, then set_param_realtime/batch_set_realtime (or edit_vst_patch).",
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
        let raw = self.call_wrapper_tool(&id, "list_params", None).await?;
        let params = parse_params_from_list_result(&raw)?;
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
                "scan_plugins",
                "load_plugin",
                "find_vst_parameter",
                "preview_vst_parameter_values",
                "set_param_realtime_or_batch_set_realtime",
                "save_preset_when_available"
            ],
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Get wrapper status and endpoint details.")]
    async fn wrapper_status(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "wrapper_status", None).await
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
Recommended workflow: scan_plugins -> load_plugin -> find_vst_parameter (search params) -> preview_vst_parameter_values (probe param) -> set_param_realtime/batch_set_realtime (or edit_vst_patch). Save preset when available."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

fn call_tool_result_to_text(result: CallToolResult) -> Result<String, String> {
    if result.is_error.unwrap_or(false) {
        if let Some(v) = result.structured_content {
            return Err(v.to_string());
        }
        let msg = contents_to_text(&result.content);
        return Err(if msg.is_empty() {
            "Wrapper tool returned an error.".to_string()
        } else {
            msg
        });
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
