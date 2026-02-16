use std::{
    borrow::Cow,
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolRequestParams, CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
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

async fn register(State(state): State<AppState>, Json(req): Json<RegisterRequest>) -> impl IntoResponse {
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
    (StatusCode::OK, Json(serde_json::json!({ "status": "registered" })))
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
    (StatusCode::OK, Json(serde_json::json!({ "status": "unregistered" })))
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
        Ok(format!("{{\"status\":\"selected\",\"instance_id\":\"{}\"}}", req.instance_id))
    }

    // ---- Proxy tools ----

    #[tool(description = "Proxy to wrapper scan_plugins.")]
    async fn scan_plugins(
        &self,
        Parameters(req): Parameters<ProxyScanPluginsRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "path": req.path }).as_object().cloned();
        self.call_wrapper_tool(&id, "scan_plugins", args).await
    }

    #[tool(description = "Proxy to wrapper load_child_plugin.")]
    async fn load_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyLoadChildRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "uid": req.uid }).as_object().cloned();
        self.call_wrapper_tool(&id, "load_child_plugin", args).await
    }

    #[tool(description = "Proxy to wrapper unload_child_plugin.")]
    async fn unload_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "unload_child_plugin", None).await
    }

    #[tool(description = "Proxy to wrapper open_child_editor.")]
    async fn open_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "open_child_editor", None).await
    }

    #[tool(description = "Proxy to wrapper close_child_editor.")]
    async fn close_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "close_child_editor", None).await
    }

    #[tool(description = "Proxy to wrapper list_params.")]
    async fn list_params(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        self.call_wrapper_tool(&id, "list_params", None).await
    }

    #[tool(description = "Proxy to wrapper set_param_realtime.")]
    async fn set_param_realtime(
        &self,
        Parameters(req): Parameters<ProxySetParamRequest>,
    ) -> Result<String, String> {
        let id = self.resolve_instance_id(req.instance_id).await?;
        let args = serde_json::json!({ "id": req.id, "value": req.value })
            .as_object()
            .cloned();
        self.call_wrapper_tool(&id, "set_param_realtime", args).await
    }

    #[tool(description = "Proxy to wrapper batch_set_realtime.")]
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
        self.call_wrapper_tool(&id, "batch_set_realtime", args).await
    }

    #[tool(description = "Proxy to wrapper wrapper_status.")]
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
                "AgentAudio MCP router. Use list_instances to find wrapper instances, then proxy wrapper tools via this server."
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

