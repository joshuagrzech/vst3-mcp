//! Stdio MCP shim for Cursor/VSCode-style clients.
//!
//! Exposes the same tool names as `agentaudio-mcp-routerd`, but over stdio.
//! Each call is forwarded to the router daemon's Streamable HTTP MCP endpoint.

use std::{borrow::Cow, sync::Arc};

use rmcp::{
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolRequestParams, CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

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
        let transport = rmcp::transport::StreamableHttpClientTransport::from_uri(self.router_mcp_url.clone());
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

    #[tool(description = "Set a default instance_id for subsequent proxy calls (process-global on routerd).")]
    async fn select_instance(
        &self,
        Parameters(req): Parameters<SelectInstanceRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("select_instance", args).await
    }

    #[tool(description = "Proxy to wrapper scan_plugins via routerd.")]
    async fn scan_plugins(
        &self,
        Parameters(req): Parameters<ProxyScanPluginsRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "path": req.path })
            .as_object()
            .cloned();
        self.call_router("scan_plugins", args).await
    }

    #[tool(description = "Proxy to wrapper load_child_plugin via routerd.")]
    async fn load_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyLoadChildRequest>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id, "uid": req.uid })
            .as_object()
            .cloned();
        self.call_router("load_child_plugin", args).await
    }

    #[tool(description = "Proxy to wrapper unload_child_plugin via routerd.")]
    async fn unload_child_plugin(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("unload_child_plugin", args).await
    }

    #[tool(description = "Proxy to wrapper open_child_editor via routerd.")]
    async fn open_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("open_child_editor", args).await
    }

    #[tool(description = "Proxy to wrapper close_child_editor via routerd.")]
    async fn close_child_editor(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("close_child_editor", args).await
    }

    #[tool(description = "Proxy to wrapper list_params via routerd.")]
    async fn list_params(
        &self,
        Parameters(req): Parameters<ProxyInstanceOnly>,
    ) -> Result<String, String> {
        let args = serde_json::json!({ "instance_id": req.instance_id })
            .as_object()
            .cloned();
        self.call_router("list_params", args).await
    }

    #[tool(description = "Proxy to wrapper set_param_realtime via routerd.")]
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

    #[tool(description = "Proxy to wrapper batch_set_realtime via routerd.")]
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

    #[tool(description = "Proxy to wrapper wrapper_status via routerd.")]
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
                "AgentAudio MCP stdio shim. Forwards calls to agentaudio-mcp-routerd over HTTP."
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

