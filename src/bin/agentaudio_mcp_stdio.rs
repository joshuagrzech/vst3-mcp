//! Stdio MCP shim for Cursor/VSCode-style clients.
//!
//! Exposes the same tool names as `agentaudio-mcp-routerd`, but over stdio.
//! Each call is forwarded to the router daemon's Streamable HTTP MCP endpoint.

use std::{borrow::Cow, sync::Arc};

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolRequestParams, CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use vst3_mcp_host::doc_search;

#[derive(Clone)]
struct RouterStdioShim {
    router_mcp_url: Arc<str>,
    tool_router: ToolRouter<Self>,
}

impl RouterStdioShim {
    fn new() -> Self {
        let base = std::env::var("AGENTAUDIO_MCP_ROUTERD")
            .ok()
            .unwrap_or_else(|| "http://127.0.0.1:38765".to_string());
        let base = base.trim().trim_end_matches('/').to_string();
        let router_mcp_url = if base.ends_with("/mcp") {
            base
        } else {
            format!("{base}/mcp")
        };

        Self {
            router_mcp_url: Arc::from(router_mcp_url),
            tool_router: Self::tool_router(),
        }
    }

    async fn call_router(
        &self,
        tool_name: &'static str,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<String, String> {
        let transport =
            rmcp::transport::StreamableHttpClientTransport::from_uri(self.router_mcp_url.clone());
        let service = ()
            .serve(transport)
            .await
            .map_err(|e| format!("Failed to connect to routerd MCP endpoint: {e}"))?;

        let result = service
            .call_tool(CallToolRequestParams {
                meta: None,
                name: Cow::Borrowed(tool_name),
                arguments,
                task: None,
            })
            .await
            .map_err(|e| format!("Router tool call failed: {e}"))?;

        let _ = service.cancel().await;
        call_tool_result_to_text(result)
    }
}

// ---- Tool parameter types ----

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
    pub prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxySearchParamsRequest {
    pub instance_id: Option<String>,
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProxyGetParamInfoRequest {
    pub instance_id: Option<String>,
    pub id: u32,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchPluginDocsRequest {
    #[schemars(description = "Plugin name or UID to search (e.g., Serum)")]
    pub plugin_name: String,
    #[schemars(description = "Targeted feature/parameter question (e.g., LFO routing)")]
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchSoundDesignGuideRequest {
    #[schemars(description = "Broad topic or sound goal (e.g., vocal compression, reese bass)")]
    pub topic: String,
    #[schemars(description = "Optional deeper query to refine the guide search")]
    pub query: Option<String>,
}

#[tool_router]
impl RouterStdioShim {
    #[tool(description = "Get router daemon status.")]
    async fn router_status(&self) -> Result<String, String> {
        self.call_router("router_status", None).await
    }

    #[tool(description = "List registered wrapper instances and their endpoints.")]
    async fn list_instances(&self) -> Result<String, String> {
        self.call_router("list_instances", None).await
    }

    #[tool(
        description = "Set a default instance_id for subsequent proxy calls (process-global on routerd)."
    )]
    async fn select_instance(
        &self,
        Parameters(req): Parameters<SelectInstanceRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("select_instance", args).await
    }

    #[tool(
        description = "Scan installed VST plugins. Use first when user says plugin/VST/synth/preset/patch/sound/tone."
    )]
    async fn scan_plugins(
        &self,
        Parameters(req): Parameters<ProxyScanPluginsRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "path": req.path })
            .as_object()
            .cloned();
        self.call_router("scan_plugins", args).await
    }

    #[tool(description = "Load child plugin by UID after scan_plugins.")]
    async fn load_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyLoadChildRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "uid": req.uid })
            .as_object()
            .cloned();
        self.call_router("load_child_plugin", args).await
    }

    #[tool(description = "Alias for load_child_plugin. Natural-language 'load plugin' tool name.")]
    async fn load_plugin(
        &self,
        Parameters(req): Parameters<ProxyLoadChildRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "uid": req.uid })
            .as_object()
            .cloned();
        self.call_router("load_plugin", args).await
    }

    #[tool(description = "Unload currently loaded child plugin.")]
    async fn unload_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("unload_child_plugin", args).await
    }

    #[tool(description = "Open child plugin editor window.")]
    async fn open_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("open_child_editor", args).await
    }

    #[tool(description = "Close child plugin editor window.")]
    async fn close_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("close_child_editor", args).await
    }

    #[tool(
        description = "List plugin parameters/knobs and current values. Supports optional prefix filter."
    )]
    async fn list_params(
        &self,
        Parameters(req): Parameters<ProxyListParamsRequest>,
    ) -> Result<String, String> {
        let mut obj = serde_json::Map::new();
        if let Some(ref id) = req.instance_id {
            obj.insert("instance_id".to_string(), serde_json::json!(id));
        }
        if let Some(ref prefix) = req.prefix {
            obj.insert("prefix".to_string(), serde_json::json!(prefix));
        }
        let args = if obj.is_empty() { None } else { Some(obj) };
        self.call_router("list_params", args).await
    }

    #[tool(
        description = "List logical parameter groups (e.g. 'Filter 1', 'Envelope 1'). Use before list_params to discover sections."
    )]
    async fn list_param_groups(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("list_param_groups", args).await
    }

    #[tool(
        description = "Search parameters by exact name substring. Faster when you know the param name."
    )]
    async fn search_params(
        &self,
        Parameters(req): Parameters<ProxySearchParamsRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "query": req.query })
            .as_object()
            .cloned();
        self.call_router("search_params", args).await
    }

    #[tool(
        description = "Get parameter metadata and range probe. Use to understand display range before setting values."
    )]
    async fn get_param_info(
        &self,
        Parameters(req): Parameters<ProxyGetParamInfoRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "id": req.id })
            .as_object()
            .cloned();
        self.call_router("get_param_info", args).await
    }

    #[tool(
        description = "Batch lookup of parameter IDs by name (fuzzy match). Returns best match for each query."
    )]
    async fn get_params_by_name(
        &self,
        Parameters(req): Parameters<ProxyGetParamsByNameRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "names": req.names })
            .as_object()
            .cloned();
        self.call_router("get_params_by_name", args).await
    }

    #[tool(
        description = "Get current patch state (all non-default parameters). Useful for verifying changes."
    )]
    async fn get_current_patch_state(
        &self,
        Parameters(req): Parameters<ProxyGetPatchStateRequest>,
    ) -> Result<String, String> {
        let mut obj = serde_json::Map::new();
        if let Some(ref id) = req.instance_id {
            obj.insert("instance_id".to_string(), serde_json::json!(id));
        }
        if let Some(diff_only) = req.diff_only {
            obj.insert("diff_only".to_string(), serde_json::json!(diff_only));
        }
        let args = if obj.is_empty() { None } else { Some(obj) };
        self.call_router("get_current_patch_state", args).await
    }

    #[tool(
        description = "Save current plugin state to a .vstpreset file. Use after patch edits."
    )]
    async fn save_preset(
        &self,
        Parameters(req): Parameters<ProxySavePresetRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "path": req.path })
            .as_object()
            .cloned();
        self.call_router("save_preset", args).await
    }

    #[tool(
        description = "Load plugin state from a .vstpreset file. Requires a plugin already loaded."
    )]
    async fn load_preset(
        &self,
        Parameters(req): Parameters<ProxyLoadPresetRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "path": req.path })
            .as_object()
            .cloned();
        self.call_router("load_preset", args).await
    }

    #[tool(
        description = "Set a parameter by name instead of id. Uses case-insensitive match."
    )]
    async fn set_param_by_name(
        &self,
        Parameters(req): Parameters<ProxySetParamByNameRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "instance_id": req.instance_id,
            "name": req.name,
            "value": req.value
        })
        .as_object()
        .cloned();
        self.call_router("set_param_by_name", args).await
    }

    #[tool(description = "Set one realtime parameter value by id.")]
    async fn set_param_realtime(
        &self,
        Parameters(req): Parameters<ProxySetParamRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "instance_id": req.instance_id,
            "id": req.id,
            "value": req.value
        })
        .as_object()
        .cloned();
        self.call_router("set_param_realtime", args).await
    }

    #[tool(
        description = "Set multiple realtime parameters in one call for coordinated patch/tone edits."
    )]
    async fn batch_set_realtime(
        &self,
        Parameters(req): Parameters<ProxyBatchSetRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "instance_id": req.instance_id,
            "changes": req.changes.into_iter().map(|c| serde_json::json!({"id": c.id, "value": c.value})).collect::<Vec<_>>()
        })
        .as_object()
        .cloned();
        self.call_router("batch_set_realtime", args).await
    }

    #[tool(
        description = "Alias for batch_set_realtime. Edit VST patch/preset/sound via parameter changes."
    )]
    async fn edit_vst_patch(
        &self,
        Parameters(req): Parameters<ProxyBatchSetRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "instance_id": req.instance_id,
            "changes": req.changes.into_iter().map(|c| serde_json::json!({"id": c.id, "value": c.value})).collect::<Vec<_>>()
        })
        .as_object()
        .cloned();
        self.call_router("edit_vst_patch", args).await
    }

    #[tool(
        description = "Search plugin parameters by natural language (e.g. 'attack', 'release', 'make brighter', 'reduce reverb')."
    )]
    async fn find_vst_parameter(
        &self,
        Parameters(req): Parameters<ProxyFindVstParameterRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "instance_id": req.instance_id,
            "query": req.query,
            "limit": req.limit
        })
        .as_object()
        .cloned();
        self.call_router("find_vst_parameter", args).await
    }

    #[tool(
        description = "Preview current values for selected parameters before editing patch/preset/tone. Optionally pass ids."
    )]
    async fn preview_vst_parameter_values(
        &self,
        Parameters(req): Parameters<ProxyPreviewVstParameterValuesRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "instance_id": req.instance_id,
            "ids": req.ids,
            "limit": req.limit
        })
        .as_object()
        .cloned();
        self.call_router("preview_vst_parameter_values", args).await
    }

    #[tool(
        description = "Search local plugin documentation by plugin_name + query and return top 2-3 excerpts."
    )]
    async fn search_plugin_docs(
        &self,
        Parameters(req): Parameters<SearchPluginDocsRequest>,
    ) -> Result<String, String> {
        let response = doc_search::search_plugin_docs(&req.plugin_name, &req.query)?;
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Search local sound design guides by topic (and optional query) and return the best recipe excerpts."
    )]
    async fn search_sound_design_guide(
        &self,
        Parameters(req): Parameters<SearchSoundDesignGuideRequest>,
    ) -> Result<String, String> {
        let response = doc_search::search_sound_design_guide(&req.topic, req.query.as_deref())?;
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Deterministic routing guardrail. Run before WebSearch; blocks web route when audio intent is high and docs/news are not explicitly requested."
    )]
    async fn guard_audio_routing(
        &self,
        Parameters(req): Parameters<GuardAudioRoutingRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({
            "user_message": req.user_message,
            "requested_tool": req.requested_tool
        })
        .as_object()
        .cloned();
        self.call_router("guard_audio_routing", args).await
    }

    #[tool(
        description = "Get param queue utilization. Use to detect when the queue is full."
    )]
    async fn param_queue_status(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("param_queue_status", args).await
    }

    #[tool(description = "Get wrapper status and endpoint details.")]
    async fn wrapper_status(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("wrapper_status", args).await
    }
}

#[tool_handler]
impl ServerHandler for RouterStdioShim {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "AgentAudio MCP stdio shim.\n\
Hard routing rule: If user mentions VST/plugin/preset/patch/sound/tone/parameter/knob/automation, use Audio MCP tools first. Do not use web search unless user explicitly asks for docs/news.\n\
Disambiguation: In audio context, patch = preset/sound configuration, not code diff.\n\
Before editing an unfamiliar plugin, call search_plugin_docs to pull only relevant plugin documentation excerpts.\n\
For target outcomes (e.g., reese bass, vocal compression), call search_sound_design_guide to fetch a recipe before tweaking parameters.\n\
Run guard_audio_routing before any web search call.\n\
Workflow: scan_plugins -> load_plugin -> find_vst_parameter -> preview_vst_parameter_values -> set_param_realtime/batch_set_realtime (or edit_vst_patch)."
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
            "Tool returned an error.".to_string()
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing to stderr (stdout is reserved for MCP JSON-RPC protocol)
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let shim = RouterStdioShim::new();
    let service = shim.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
