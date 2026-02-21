use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_queue::ArrayQueue;
use nih_plug::prelude::*;
use nih_plug_egui::{
    EguiState, create_egui_editor,
    egui,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::StreamableHttpService;
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use vst3_mcp_host::gui;
use vst3_mcp_host::hosting::host_app::{ComponentHandler, HostApp};
use vst3_mcp_host::hosting::module::VstModule;
use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::hosting::scanner;
use vst3_mcp_host::hosting::types::{BusDirection, BusInfo, BusType, PluginInfo};

const PARAM_QUEUE_CAPACITY: usize = 4096;
const MAX_PARAM_EVENTS_PER_BLOCK: usize = 512;

/// Locate the agent-audio-scanner binary for out-of-process plugin scanning.
/// When found, scan_plugins uses it to isolate plugin load crashes from the host.
fn find_scanner_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AGENTAUDIO_SCANNER") {
        let path = PathBuf::from(&p);
        if path.is_file() {
            return Some(path);
        }
    }
    #[cfg(unix)]
    {
        unsafe {
            let mut info: libc::Dl_info = std::mem::zeroed();
            if libc::dladdr(find_scanner_binary as *const _, &mut info) != 0 && !info.dli_fname.is_null() {
                let fname = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy();
                let so_path = PathBuf::from(fname.as_ref());
                if let Some(parent) = so_path.parent() {
                    // Same directory as .so (e.g. Contents/x86_64-linux/)
                    let same_dir = parent.join("agent-audio-scanner");
                    if same_dir.is_file() {
                        return Some(same_dir);
                    }
                    // Bundle Resources (e.g. Contents/Resources/)
                    let resources = parent.parent().map(|c| c.join("Resources").join("agent-audio-scanner"));
                    if let Some(ref r) = resources {
                        if r.is_file() {
                            return Some(r.clone());
                        }
                    }
                }
            }
        }
    }
    None
}

#[derive(Params)]
struct WrapperParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,
}

impl Default for WrapperParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(560, 420),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct QueuedParamChange {
    id: u32,
    value: f64,
    enqueued_at_ms: u64,
}

#[derive(Default)]
struct EditorRuntime {
    close_signal: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone)]
struct SharedState {
    instance_id: Arc<String>,
    child_plugin: Arc<Mutex<Option<PluginInstance>>>,
    loaded_info: Arc<RwLock<Option<PluginInfo>>>,
    scan_cache: Arc<Mutex<Vec<PluginInfo>>>,
    sample_rate: Arc<RwLock<f64>>,
    max_block_size: Arc<RwLock<i32>>,
    endpoint: Arc<RwLock<Option<String>>>,
    param_queue: Arc<ArrayQueue<QueuedParamChange>>,
    editor_runtime: Arc<Mutex<EditorRuntime>>,
}

impl SharedState {
    fn new() -> Self {
        Self {
            instance_id: Arc::new(Uuid::new_v4().to_string()),
            child_plugin: Arc::new(Mutex::new(None)),
            loaded_info: Arc::new(RwLock::new(None)),
            scan_cache: Arc::new(Mutex::new(Vec::new())),
            sample_rate: Arc::new(RwLock::new(44100.0)),
            max_block_size: Arc::new(RwLock::new(1024)),
            endpoint: Arc::new(RwLock::new(None)),
            param_queue: Arc::new(ArrayQueue::new(PARAM_QUEUE_CAPACITY)),
            editor_runtime: Arc::new(Mutex::new(EditorRuntime::default())),
        }
    }

    fn mcp_name(&self) -> String {
        self.loaded_info
            .read()
            .ok()
            .and_then(|loaded| loaded.clone())
            .map(|info| format!("AgentAudio - {}", info.name))
            .unwrap_or_else(|| "AgentAudio - Unloaded".to_string())
    }

    fn endpoint(&self) -> Option<String> {
        self.endpoint.read().ok().and_then(|v| v.clone())
    }

    fn scan_plugins(&self, path: Option<&str>) -> Result<Vec<PluginInfo>, String> {
        let scanner_binary = find_scanner_binary();
        let plugins = scanner::scan_plugins_safe(path, scanner_binary.as_deref())
            .map_err(|e| format!("Scan failed: {e}"))?;
        let mut cache = self
            .scan_cache
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        *cache = plugins.clone();
        Ok(plugins)
    }

    fn find_plugin(&self, uid: &str) -> Result<PluginInfo, String> {
        let uid_upper = uid.to_uppercase();
        if let Some(found) = self
            .scan_cache
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?
            .iter()
            .find(|p| p.uid.to_uppercase() == uid_upper)
            .cloned()
        {
            return Ok(found);
        }

        let scanned = self.scan_plugins(None)?;
        scanned
            .into_iter()
            .find(|p| p.uid.to_uppercase() == uid_upper)
            .ok_or_else(|| format!("Plugin UID '{}' not found", uid))
    }

    fn close_editor(&self) -> bool {
        let mut editor = match self.editor_runtime.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        editor.close_signal.store(true, Ordering::Relaxed);
        if let Some(handle) = editor.thread.take() {
            let _ = handle.join();
            true
        } else {
            false
        }
    }

    fn open_editor(&self) -> Result<(), String> {
        let plugin_name = self
            .loaded_info
            .read()
            .map_err(|e| format!("Lock error: {e}"))?
            .as_ref()
            .map(|p| p.name.clone())
            .ok_or_else(|| "No child plugin loaded".to_string())?;

        self.close_editor();

        let plugin_arc = Arc::clone(&self.child_plugin);
        let close_signal = Arc::new(AtomicBool::new(false));
        let close_signal_clone = Arc::clone(&close_signal);

        let handle = std::thread::spawn(move || {
            let (tx, _rx) = std::sync::mpsc::channel();
            let _ = gui::open_editor_window(plugin_arc, plugin_name, tx, close_signal_clone);
        });

        let mut editor = self
            .editor_runtime
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        editor.close_signal = close_signal;
        editor.thread = Some(handle);
        Ok(())
    }

    fn unload_child_plugin(&self) -> Result<(), String> {
        self.close_editor();
        {
            let mut plugin = self
                .child_plugin
                .lock()
                .map_err(|e| format!("Lock error: {e}"))?;
            *plugin = None;
        }
        {
            let mut loaded = self
                .loaded_info
                .write()
                .map_err(|e| format!("Lock error: {e}"))?;
            *loaded = None;
        }
        while self.param_queue.pop().is_some() {}
        Ok(())
    }

    /// Load a child plugin by path to a .vst3 bundle. Scans only that bundle.
    fn load_child_plugin_by_path(&self, path: &str) -> Result<PluginInfo, String> {
        let path_buf = Path::new(path.trim());
        if !path_buf.exists() {
            return Err(format!("Path does not exist: {}", path_buf.display()));
        }
        let is_bundle = path_buf
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("vst3"));
        if !is_bundle || !path_buf.is_dir() {
            return Err(format!(
                "Path must be a .vst3 bundle directory (e.g. ~/.vst3/MyPlugin.vst3), got: {}",
                path_buf.display()
            ));
        }
        let bundle_path = path_buf.to_path_buf();

        let plugins = scanner::scan_single_bundle(&bundle_path)
            .map_err(|e| format!("Scan failed: {e}"))?;

        let info = plugins
            .into_iter()
            .next()
            .ok_or_else(|| format!("No audio plugins found in bundle: {}", bundle_path.display()))?;

        // Update cache for load_child_plugin (UID-based) consistency
        if let Ok(mut cache) = self.scan_cache.lock() {
            *cache = vec![info.clone()];
        }

        self.load_child_plugin(&info.uid)
    }

    fn load_child_plugin(&self, uid: &str) -> Result<PluginInfo, String> {
        let info = self.find_plugin(uid)?;
        let class_id = hex_to_tuid(&info.uid)?;
        let module = Arc::new(
            VstModule::load(&info.path)
                .map_err(|e| format!("Failed to load module {}: {e}", info.path.display()))?,
        );

        let host_app = HostApp::new();
        let handler = ComponentHandler::new();
        let mut instance = PluginInstance::from_factory(module, &class_id, host_app, handler)
            .map_err(|e| format!("Failed to create child instance: {e}"))?;

        let sample_rate = *self
            .sample_rate
            .read()
            .map_err(|e| format!("Lock error: {e}"))?;
        let max_block_size = *self
            .max_block_size
            .read()
            .map_err(|e| format!("Lock error: {e}"))?;

        instance
            .setup(sample_rate, max_block_size)
            .map_err(|e| format!("Child setup failed: {e}"))?;
        instance
            .activate()
            .map_err(|e| format!("Child activation failed: {e}"))?;
        instance
            .start_processing()
            .map_err(|e| format!("Child start processing failed: {e}"))?;

        let bus_info = instance.get_bus_info();
        validate_supported_routing(&bus_info)?;

        {
            let mut plugin = self
                .child_plugin
                .lock()
                .map_err(|e| format!("Lock error: {e}"))?;
            *plugin = Some(instance);
        }
        {
            let mut loaded = self
                .loaded_info
                .write()
                .map_err(|e| format!("Lock error: {e}"))?;
            *loaded = Some(info.clone());
        }
        Ok(info)
    }

    fn queue_param_change(&self, id: u32, value: f64) -> Result<(), ()> {
        self.param_queue
            .push(QueuedParamChange {
                id,
                value,
                enqueued_at_ms: now_ms(),
            })
            .map_err(|_| ())
    }

    fn mirror_param_immediate(&self, id: u32, value: f64) -> bool {
        let mut guard = match self.child_plugin.try_lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let Some(plugin) = guard.as_mut() else {
            return false;
        };
        plugin.set_parameter_immediate(id, value).is_ok()
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ScanPluginsRequest {
    pub path: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct LoadChildRequest {
    pub uid: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct LoadChildByPathRequest {
    /// Path to a .vst3 bundle (e.g. ~/.vst3/MyPlugin.vst3 or /usr/lib/vst3/Foo.vst3)
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SetParamRequest {
    pub id: u32,
    pub value: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ParamChange {
    pub id: u32,
    pub value: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct BatchSetRequest {
    pub changes: Vec<ParamChange>,
}

#[derive(Default)]
struct GuiState {
    /// User-entered path to a .vst3 bundle (e.g. /usr/lib/vst3/MyPlugin.vst3 or ~/.vst3/Synth.vst3)
    plugin_path: String,
    message: String,
}

struct WrapperMcpServer {
    shared: SharedState,
    tool_router: ToolRouter<Self>,
}

impl WrapperMcpServer {
    fn new(shared: SharedState) -> Self {
        Self {
            shared,
            tool_router: Self::tool_router(),
        }
    }

    fn with_child_plugin<R>(
        &self,
        f: impl FnOnce(&mut PluginInstance) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut guard = self
            .shared
            .child_plugin
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?;
        let plugin = guard
            .as_mut()
            .ok_or_else(|| "No child plugin loaded".to_string())?;
        f(plugin)
    }
}

#[tool_router]
impl WrapperMcpServer {
    #[tool(description = "Scan installed VST3 plugins for child-hosting")]
    fn scan_plugins(
        &self,
        Parameters(req): Parameters<ScanPluginsRequest>,
    ) -> Result<String, String> {
        let plugins = self.shared.scan_plugins(req.path.as_deref())?;
        serde_json::to_string_pretty(&plugins).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Load a child VST3 by path to a .vst3 bundle. No system-wide scan.")]
    fn load_child_plugin_by_path(
        &self,
        Parameters(req): Parameters<LoadChildByPathRequest>,
    ) -> Result<String, String> {
        let expanded = expand_tilde(&req.path);
        let info = self.shared.load_child_plugin_by_path(&expanded)?;
        let _ = self.shared.open_editor();

        let response = serde_json::json!({
            "status": "loaded",
            "uid": info.uid,
            "name": info.name,
            "vendor": info.vendor,
            "mcp_name": self.shared.mcp_name(),
            "instance_id": self.shared.instance_id.as_str(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Load a child VST3 by UID (requires scan_plugins or load_child_plugin_by_path first to populate cache).")]
    fn load_child_plugin(
        &self,
        Parameters(req): Parameters<LoadChildRequest>,
    ) -> Result<String, String> {
        let info = self.shared.load_child_plugin(&req.uid)?;
        let _ = self.shared.open_editor();

        let response = serde_json::json!({
            "status": "loaded",
            "uid": info.uid,
            "name": info.name,
            "vendor": info.vendor,
            "mcp_name": self.shared.mcp_name(),
            "instance_id": self.shared.instance_id.as_str(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Unload current child plugin.")]
    fn unload_child_plugin(&self) -> Result<String, String> {
        self.shared.unload_child_plugin()?;
        let response = serde_json::json!({
            "status": "unloaded",
            "instance_id": self.shared.instance_id.as_str(),
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Open child plugin editor window.")]
    fn open_child_editor(&self) -> Result<String, String> {
        self.shared.open_editor()?;
        Ok("{\"status\":\"opened\"}".to_string())
    }

    #[tool(description = "Close child plugin editor window.")]
    fn close_child_editor(&self) -> Result<String, String> {
        let closed = self.shared.close_editor();
        let response = serde_json::json!({ "status": if closed { "closed" } else { "not_open" } });
        serde_json::to_string(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "List writable child plugin parameters and current values.")]
    fn list_params(&self) -> Result<String, String> {
        self.with_child_plugin(|plugin| {
            let count = plugin.get_parameter_count();
            let mut parameters = Vec::new();
            for i in 0..count {
                if let Ok(info) = plugin.get_parameter_info(i) {
                    if info.is_writable() && !info.is_hidden() {
                        let value = plugin.get_parameter(info.id);
                        let display = plugin
                            .get_parameter_display(info.id)
                            .unwrap_or_else(|_| format!("{value:.3}"));
                        parameters.push(serde_json::json!({
                            "id": info.id,
                            "name": info.title,
                            "value": value,
                            "display": display,
                        }));
                    }
                }
            }

            let response = serde_json::json!({
                "count": parameters.len(),
                "parameters": parameters,
            });
            serde_json::to_string_pretty(&response)
                .map_err(|e| format!("Serialization failed: {e}"))
        })
    }

    #[tool(description = "Queue a realtime parameter update for audio-thread application.")]
    fn set_param_realtime(
        &self,
        Parameters(req): Parameters<SetParamRequest>,
    ) -> Result<String, String> {
        if !(0.0..=1.0).contains(&req.value) {
            return Err(format!(
                "Invalid parameter value {}. Must be in [0.0, 1.0]",
                req.value
            ));
        }
        let mirrored = self.shared.mirror_param_immediate(req.id, req.value);
        let accepted = self.shared.queue_param_change(req.id, req.value).is_ok();
        let response = serde_json::json!({
            "status": if accepted { "queued" } else { "dropped_queue_full" },
            "id": req.id,
            "value": req.value,
            "immediate_applied": mirrored,
            "queue_len": self.shared.param_queue.len(),
            "timestamp_ms": now_ms(),
            "instance_id": self.shared.instance_id.as_str(),
        });
        serde_json::to_string(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Queue multiple realtime parameter updates for audio-thread application.")]
    fn batch_set_realtime(
        &self,
        Parameters(req): Parameters<BatchSetRequest>,
    ) -> Result<String, String> {
        for change in &req.changes {
            if !(0.0..=1.0).contains(&change.value) {
                return Err(format!(
                    "Invalid parameter value {} for id {}. Must be in [0.0, 1.0]",
                    change.value, change.id
                ));
            }
        }

        let mut accepted = 0usize;
        let mut mirrored = 0usize;
        for change in &req.changes {
            if self.shared.mirror_param_immediate(change.id, change.value) {
                mirrored += 1;
            }
            if self
                .shared
                .queue_param_change(change.id, change.value)
                .is_ok()
            {
                accepted += 1;
            }
        }

        let response = serde_json::json!({
            "status": "queued",
            "accepted": accepted,
            "dropped": req.changes.len().saturating_sub(accepted),
            "immediate_applied": mirrored,
            "queue_len": self.shared.param_queue.len(),
            "timestamp_ms": now_ms(),
            "instance_id": self.shared.instance_id.as_str(),
        });
        serde_json::to_string(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(description = "Get wrapper status and endpoint details.")]
    fn wrapper_status(&self) -> Result<String, String> {
        let loaded = self
            .shared
            .loaded_info
            .read()
            .map_err(|e| format!("Lock error: {e}"))?
            .clone();
        let endpoint = self.shared.endpoint();
        let response = if let Some(info) = loaded {
            serde_json::json!({
                "status": "child_loaded",
                "name": info.name,
                "vendor": info.vendor,
                "uid": info.uid,
                "mcp_name": self.shared.mcp_name(),
                "endpoint": endpoint,
                "instance_id": self.shared.instance_id.as_str(),
                "queue_len": self.shared.param_queue.len(),
            })
        } else {
            serde_json::json!({
                "status": "no_child_loaded",
                "mcp_name": self.shared.mcp_name(),
                "endpoint": endpoint,
                "instance_id": self.shared.instance_id.as_str(),
                "queue_len": self.shared.param_queue.len(),
            })
        };
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }
}

#[tool_handler]
impl ServerHandler for WrapperMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                format!(
                    "{} realtime wrapper server (instance {}). Load a child plugin, then use set_param_realtime/batch_set_realtime.",
                    self.shared.mcp_name(),
                    self.shared.instance_id
                )
                .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

struct EmbeddedMcpServerHandle {
    cancel: CancellationToken,
    join_handle: Option<std::thread::JoinHandle<()>>,
    router_join_handle: Option<std::thread::JoinHandle<()>>,
    endpoint: String,
}

impl EmbeddedMcpServerHandle {
    fn start(shared: SharedState) -> Result<Self, String> {
        let cancel = CancellationToken::new();
        let child_cancel = cancel.child_token();
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let shared_for_thread = shared.clone();

        let join_handle = std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return,
            };

            runtime.block_on(async move {
                let service: StreamableHttpService<WrapperMcpServer, LocalSessionManager> =
                    StreamableHttpService::new(
                        move || Ok(WrapperMcpServer::new(shared_for_thread.clone())),
                        Default::default(),
                        StreamableHttpServerConfig {
                            stateful_mode: true,
                            sse_keep_alive: None,
                            cancellation_token: child_cancel.clone(),
                            ..Default::default()
                        },
                    );
                let router = axum::Router::new().nest_service("/mcp", service);
                let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                    Ok(listener) => listener,
                    Err(_) => return,
                };
                let addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(_) => return,
                };
                let _ = addr_tx.send(addr.to_string());

                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        child_cancel.cancelled_owned().await;
                    })
                    .await;
            });
        });

        let addr = addr_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "Embedded MCP server did not start in time".to_string())?;
        let endpoint = format!("http://{addr}/mcp");
        {
            if let Ok(mut ep) = shared.endpoint.write() {
                *ep = Some(endpoint.clone());
            }
        }

        let router_join_handle =
            start_router_registration_thread(cancel.clone(), shared.clone(), endpoint.clone());

        Ok(Self {
            cancel,
            join_handle: Some(join_handle),
            router_join_handle,
            endpoint,
        })
    }
}

fn start_router_registration_thread(
    cancel: CancellationToken,
    shared: SharedState,
    endpoint: String,
) -> Option<std::thread::JoinHandle<()>> {
    let router_base = std::env::var("AGENTAUDIO_MCP_ROUTERD")
        .ok()
        .unwrap_or_else(|| "http://127.0.0.1:38765".to_string());
    let router_base = router_base.trim().trim_end_matches('/').to_string();
    if router_base.is_empty() {
        return None;
    }

    let instance_id = shared.instance_id.to_string();
    let mcp_name = shared.mcp_name();
    let register_url = format!("{router_base}/register");
    let heartbeat_url = format!("{router_base}/heartbeat");
    let unregister_url = format!("{router_base}/unregister");

    Some(std::thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_millis(500))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };

        // Best-effort register. If routerd is down, all errors are ignored.
        let _ = client
            .post(&register_url)
            .json(&serde_json::json!({
                "instance_id": instance_id,
                "endpoint": endpoint,
                "mcp_name": mcp_name,
            }))
            .send();

        // Heartbeat loop; routerd TTL pruning keeps the registry tidy even if we never unregister.
        while !cancel.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let _ = client
                .post(&heartbeat_url)
                .json(&serde_json::json!({
                    "instance_id": shared.instance_id.to_string(),
                }))
                .send();
        }

        let _ = client
            .post(&unregister_url)
            .json(&serde_json::json!({
                "instance_id": shared.instance_id.to_string(),
            }))
            .send();
    }))
}

impl Drop for EmbeddedMcpServerHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join_handle.take() {
            let _ = join.join();
        }
        if let Some(join) = self.router_join_handle.take() {
            let _ = join.join();
        }
    }
}

pub struct AgentAudioWrapper {
    params: Arc<WrapperParams>,
    shared: SharedState,
    gui_state: Arc<Mutex<GuiState>>,
    mcp_server: Option<EmbeddedMcpServerHandle>,
    input_staging: Vec<Vec<f32>>,
}

impl Default for AgentAudioWrapper {
    fn default() -> Self {
        Self {
            params: Arc::new(WrapperParams::default()),
            shared: SharedState::new(),
            gui_state: Arc::new(Mutex::new(GuiState::default())),
            mcp_server: None,
            input_staging: Vec::new(),
        }
    }
}

impl Plugin for AgentAudioWrapper {
    type SysExMessage = ();
    type BackgroundTask = ();

    const NAME: &'static str = "AgentAudio Wrapper";
    const VENDOR: &'static str = "AgentAudio";
    const URL: &'static str = "https://example.com";
    const EMAIL: &'static str = "support@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: Some(new_nonzero_u32(2)),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let params = self.params.clone();
        let shared = self.shared.clone();
        let gui_state = Arc::clone(&self.gui_state);
        create_egui_editor(
            self.params.editor_state.clone(),
            (),
            |_, _| {},
            move |ctx, _setter, _state| {
                egui::Window::new("AgentAudio Wrapper").show(ctx, |ui| {
                    ui.label(format!("Instance: {}", shared.instance_id));
                    ui.label(format!("MCP Name: {}", shared.mcp_name()));
                    ui.monospace(
                        shared
                            .endpoint()
                            .unwrap_or_else(|| "MCP endpoint starting...".to_string()),
                    );
                    ui.separator();

                    ui.label("VST3 bundle path (e.g. ~/.vst3/MyPlugin.vst3):");
                    {
                        let mut path = gui_state
                            .lock()
                            .ok()
                            .map(|gs| gs.plugin_path.clone())
                            .unwrap_or_default();
                        if ui.text_edit_singleline(&mut path).changed() {
                            if let Ok(mut gs) = gui_state.lock() {
                                gs.plugin_path = path;
                            }
                        }
                    }

                    let path = gui_state.lock().ok().and_then(|gs| {
                        let p = gs.plugin_path.trim();
                        if p.is_empty() {
                            None
                        } else {
                            Some(p.to_string())
                        }
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Load from path").clicked() {
                            let msg = if let Some(p) = path {
                                let expanded = expand_tilde(&p);
                                match shared.load_child_plugin_by_path(&expanded) {
                                    Ok(info) => {
                                        let _ = shared.open_editor();
                                        format!("Loaded {}", info.name)
                                    }
                                    Err(e) => format!("Load failed: {e}"),
                                }
                            } else {
                                "Enter a path to a .vst3 bundle".to_string()
                            };
                            if let Ok(mut gs) = gui_state.lock() {
                                gs.message = msg;
                            }
                        }
                        if ui.button("Unload Child").clicked() {
                            let msg = match shared.unload_child_plugin() {
                                Ok(()) => "Child plugin unloaded".to_string(),
                                Err(e) => format!("Unload failed: {e}"),
                            };
                            if let Ok(mut gs) = gui_state.lock() {
                                gs.message = msg;
                            }
                        }
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Open Child Editor").clicked() {
                            let msg = match shared.open_editor() {
                                Ok(()) => "Editor opened".to_string(),
                                Err(e) => format!("Open editor failed: {e}"),
                            };
                            if let Ok(mut gs) = gui_state.lock() {
                                gs.message = msg;
                            }
                        }
                        if ui.button("Close Child Editor").clicked() {
                            let closed = shared.close_editor();
                            if let Ok(mut gs) = gui_state.lock() {
                                gs.message = if closed {
                                    "Editor closed".to_string()
                                } else {
                                    "Editor was not open".to_string()
                                };
                            }
                        }
                    });

                    if let Some(loaded) = shared.loaded_info.read().ok().and_then(|v| v.clone()) {
                        ui.label(format!("Loaded: {} ({})", loaded.name, loaded.vendor));
                    } else {
                        ui.label("Loaded: none");
                    }

                    let message = gui_state
                        .lock()
                        .ok()
                        .map(|g| g.message.clone())
                        .unwrap_or_default();
                    if !message.is_empty() {
                        ui.separator();
                        ui.label(message);
                    }
                    ui.label(format!("Queue size: {}", shared.param_queue.len()));
                    ui.label(format!(
                        "GUI open: {}",
                        if params.editor_state.is_open() {
                            "yes"
                        } else {
                            "no"
                        }
                    ));
                });
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        if let Ok(mut sr) = self.shared.sample_rate.write() {
            *sr = buffer_config.sample_rate as f64;
        }
        if let Ok(mut bs) = self.shared.max_block_size.write() {
            *bs = buffer_config.max_buffer_size as i32;
        }

        if self.mcp_server.is_none() {
            if let Ok(server) = EmbeddedMcpServerHandle::start(self.shared.clone()) {
                nih_log!(
                    "AgentAudio MCP endpoint [{}]: {}",
                    self.shared.instance_id,
                    server.endpoint
                );
                self.mcp_server = Some(server);
            } else {
                nih_log!("AgentAudio failed to start embedded MCP server");
            }
        }

        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer<'_>,
        _aux: &mut AuxiliaryBuffers<'_>,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        if buffer.is_empty() {
            return ProcessStatus::Normal;
        }

        let mut guard = match self.shared.child_plugin.try_lock() {
            Ok(guard) => guard,
            Err(_) => return ProcessStatus::Normal,
        };
        let Some(child) = guard.as_mut() else {
            return ProcessStatus::Normal;
        };

        for _ in 0..MAX_PARAM_EVENTS_PER_BLOCK {
            let Some(change) = self.shared.param_queue.pop() else {
                break;
            };
            let _latency_ms = now_ms().saturating_sub(change.enqueued_at_ms);
            child.queue_parameter_change(change.id, change.value);
        }

        let num_samples = buffer.samples();
        let channels = buffer.as_slice();
        if self.input_staging.len() < channels.len() {
            self.input_staging
                .resize_with(channels.len(), || vec![0.0; num_samples]);
        }

        for (idx, channel) in channels.iter().enumerate() {
            let staging = &mut self.input_staging[idx];
            if staging.len() != num_samples {
                staging.resize(num_samples, 0.0);
            }
            staging[..num_samples].copy_from_slice(&channel[..num_samples]);
        }

        let input_refs: Vec<&[f32]> = self
            .input_staging
            .iter()
            .take(channels.len())
            .map(|v| v.as_slice())
            .collect();
        let mut output_refs: Vec<&mut [f32]> = channels.iter_mut().map(|c| &mut c[..]).collect();

        if let Err(err) = child.process(&input_refs, &mut output_refs, num_samples as i32) {
            nih_log!("Child plugin processing error: {err}");
        }

        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        let _ = self.shared.unload_child_plugin();
        self.mcp_server = None;
    }
}

impl Vst3Plugin for AgentAudioWrapper {
    const VST3_CLASS_ID: [u8; 16] = *b"AgentAudioWrap01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Tools,
    ];
}

nih_export_vst3!(AgentAudioWrapper);

fn validate_supported_routing(buses: &[BusInfo]) -> Result<(), String> {
    let has_audio_input = buses
        .iter()
        .any(|b| b.bus_type == BusType::Audio && b.direction == BusDirection::Input);
    let has_audio_output = buses
        .iter()
        .any(|b| b.bus_type == BusType::Audio && b.direction == BusDirection::Output);
    let has_event_input = buses
        .iter()
        .any(|b| b.bus_type == BusType::Event && b.direction == BusDirection::Input);

    let effect_like = has_audio_input && has_audio_output;
    let instrument_like = has_audio_output && has_event_input;
    if effect_like || instrument_like {
        Ok(())
    } else {
        Err("Unsupported child routing: plugin must expose audio output and either audio input (effect) or event input (instrument).".to_string())
    }
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, &path[2..]);
        }
    } else if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    path.to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn hex_to_tuid(hex: &str) -> Result<[i8; 16], String> {
    if hex.len() != 32 {
        return Err(format!("UID must be 32 hex characters, got {}", hex.len()));
    }
    let mut tuid = [0i8; 16];
    for i in 0..16 {
        let byte_str = &hex[i * 2..i * 2 + 2];
        let byte = u8::from_str_radix(byte_str, 16)
            .map_err(|e| format!("Invalid hex byte '{}': {}", byte_str, e))?;
        tuid[i] = byte as i8;
    }
    Ok(tuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_to_tuid_rejects_bad_length() {
        assert!(hex_to_tuid("ABC").is_err());
    }

    #[test]
    fn queue_drops_when_full() {
        let queue = ArrayQueue::new(2);
        assert!(
            queue
                .push(QueuedParamChange {
                    id: 1,
                    value: 0.5,
                    enqueued_at_ms: 1,
                })
                .is_ok()
        );
        assert!(
            queue
                .push(QueuedParamChange {
                    id: 2,
                    value: 0.6,
                    enqueued_at_ms: 2,
                })
                .is_ok()
        );
        assert!(
            queue
                .push(QueuedParamChange {
                    id: 3,
                    value: 0.7,
                    enqueued_at_ms: 3,
                })
                .is_err()
        );
    }

    #[test]
    fn supports_effect_and_instrument_layouts() {
        let layouts = AgentAudioWrapper::AUDIO_IO_LAYOUTS;
        let has_effect = layouts
            .iter()
            .any(|l| l.main_input_channels.is_some() && l.main_output_channels.is_some());
        let has_instrument = layouts
            .iter()
            .any(|l| l.main_input_channels.is_none() && l.main_output_channels.is_some());
        assert!(has_effect, "expected at least one effect layout");
        assert!(has_instrument, "expected at least one instrument layout");
    }

    #[test]
    fn routing_validation_accepts_effect_and_instrument() {
        let effect = vec![
            BusInfo {
                name: "In".to_string(),
                channel_count: 2,
                bus_type: BusType::Audio,
                direction: BusDirection::Input,
                is_default_active: true,
            },
            BusInfo {
                name: "Out".to_string(),
                channel_count: 2,
                bus_type: BusType::Audio,
                direction: BusDirection::Output,
                is_default_active: true,
            },
        ];
        assert!(validate_supported_routing(&effect).is_ok());

        let instrument = vec![
            BusInfo {
                name: "Events".to_string(),
                channel_count: 16,
                bus_type: BusType::Event,
                direction: BusDirection::Input,
                is_default_active: true,
            },
            BusInfo {
                name: "Out".to_string(),
                channel_count: 2,
                bus_type: BusType::Audio,
                direction: BusDirection::Output,
                is_default_active: true,
            },
        ];
        assert!(validate_supported_routing(&instrument).is_ok());
    }

    #[test]
    fn routing_validation_rejects_incompatible_plugins() {
        let incompatible = vec![BusInfo {
            name: "Events".to_string(),
            channel_count: 16,
            bus_type: BusType::Event,
            direction: BusDirection::Input,
            is_default_active: true,
        }];
        assert!(validate_supported_routing(&incompatible).is_err());
    }
}
