//! MCP server with tool definitions for VST3 audio processing.
//!
//! Exposes plugin scanning, loading, audio processing, and preset management
//! as MCP tools over stdio transport.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler};
use tracing::{debug, info};

use vst3_mcp_host::audio;
use vst3_mcp_host::gui;
use vst3_mcp_host::hosting::host_app::{ComponentHandler, HostApp};
use vst3_mcp_host::hosting::module::VstModule;
use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::hosting::scanner;
use vst3_mcp_host::hosting::types::PluginInfo;
use vst3_mcp_host::preset::state;

// #region agent log
fn agent_log(
    run_id: &str,
    hypothesis_id: &str,
    location: &str,
    message: &str,
    data: serde_json::Value,
) {
    use std::io::Write;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let payload = serde_json::json!({
        "id": format!("log_{}_{}", std::process::id(), ts),
        "timestamp": ts,
        "location": location,
        "message": message,
        "data": data,
        "runId": run_id,
        "hypothesisId": hypothesis_id,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/home/josh/Developer/vst3-mcp/.cursor/debug.log")
    {
        let _ = writeln!(f, "{}", payload);
    }
}
// #endregion

// -- Tool input structs --

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScanPluginsRequest {
    /// Optional directory path to scan instead of default OS locations.
    #[schemars(description = "Optional directory path to scan instead of default OS locations")]
    pub path: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LoadPluginRequest {
    /// Plugin UID (hex string) from scan results.
    #[schemars(description = "Plugin UID (hex string) from scan results")]
    pub uid: String,
    /// Sample rate for processing (default: 44100, overridden by input file during process_audio).
    #[schemars(description = "Sample rate for processing (default: 44100)")]
    pub sample_rate: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProcessAudioRequest {
    /// Path to the input audio file (WAV, FLAC, MP3, OGG, etc).
    #[schemars(description = "Path to the input audio file (WAV, FLAC, MP3, OGG, etc)")]
    pub input_file: String,
    /// Path for the output WAV file.
    #[schemars(description = "Path for the output WAV file")]
    pub output_file: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SavePresetRequest {
    /// Output path for the .vstpreset file.
    #[schemars(description = "Output path for the .vstpreset file")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LoadPresetRequest {
    /// Path to the .vstpreset file to load.
    #[schemars(description = "Path to the .vstpreset file to load")]
    pub path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetParamRequest {
    /// Parameter ID from list_params.
    #[schemars(description = "Parameter ID from list_params")]
    pub id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetParamRequest {
    /// Parameter ID from list_params.
    #[schemars(description = "Parameter ID from list_params")]
    pub id: u32,
    /// Normalized parameter value (must be in range [0.0, 1.0]).
    #[schemars(description = "Normalized parameter value (must be in range [0.0, 1.0])")]
    pub value: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ParamChange {
    /// Parameter ID.
    #[schemars(description = "Parameter ID")]
    pub id: u32,
    /// Normalized parameter value (must be in range [0.0, 1.0]).
    #[schemars(description = "Normalized parameter value (must be in range [0.0, 1.0])")]
    pub value: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchSetRequest {
    /// List of parameter changes to apply atomically.
    #[schemars(description = "List of parameter changes to apply atomically")]
    pub changes: Vec<ParamChange>,
}

// -- AudioHost MCP server --

/// MCP server that hosts VST3 plugins for audio processing.
///
/// Supports one plugin loaded at a time (Phase 1 scope).
/// All plugin operations are protected by a Mutex and run on blocking threads.
pub struct AudioHost {
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    plugin_info: Arc<Mutex<Option<PluginInfo>>>,
    /// Keep a reference to the VST3 module while the plugin is loaded.
    /// PluginInstance also holds an Arc<VstModule>, ensuring the module
    /// outlives the plugin instance even if this field is cleared first.
    module: Arc<Mutex<Option<Arc<VstModule>>>>,
    /// Last scan results cached for load_plugin lookup.
    scan_cache: Arc<Mutex<Vec<PluginInfo>>>,
    tool_router: ToolRouter<Self>,
}

impl AudioHost {
    pub fn new() -> Self {
        Self {
            plugin: Arc::new(Mutex::new(None)),
            plugin_info: Arc::new(Mutex::new(None)),
            module: Arc::new(Mutex::new(None)),
            scan_cache: Arc::new(Mutex::new(Vec::new())),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl AudioHost {
    #[tool(description = "Scan for installed VST3 plugins and return a list with UIDs, names, vendors, and categories")]
    fn scan_plugins(
        &self,
        Parameters(req): Parameters<ScanPluginsRequest>,
    ) -> Result<String, String> {
        info!("scan_plugins called with path: {:?}", req.path);

        let plugins = scanner::scan_plugins(req.path.as_deref())
            .map_err(|e| format!("Scan failed: {}", e))?;

        // Cache scan results for load_plugin
        if let Ok(mut cache) = self.scan_cache.lock() {
            *cache = plugins.clone();
        }

        let json = serde_json::to_string_pretty(&plugins)
            .map_err(|e| format!("Failed to serialize plugin list: {}", e))?;

        info!("scan_plugins found {} plugins", plugins.len());
        Ok(json)
    }

    #[tool(description = "Load a VST3 plugin by its UID from scan results. Call scan_plugins first to discover available plugins.")]
    fn load_plugin(
        &self,
        Parameters(req): Parameters<LoadPluginRequest>,
    ) -> Result<String, String> {
        info!("load_plugin called with uid: {}", req.uid);

        let sample_rate = req.sample_rate.unwrap_or(44100);
        let uid_upper = req.uid.to_uppercase();

        // Find the plugin in scan cache
        let plugin_info = {
            let cache = self.scan_cache.lock().map_err(|e| format!("Lock error: {}", e))?;
            cache
                .iter()
                .find(|p| p.uid.to_uppercase() == uid_upper)
                .cloned()
        };

        let info = match plugin_info {
            Some(info) => info,
            None => {
                // Try a fresh scan
                let plugins = scanner::scan_plugins(None)
                    .map_err(|e| format!("Scan failed: {}", e))?;

                if let Ok(mut cache) = self.scan_cache.lock() {
                    *cache = plugins.clone();
                }

                plugins
                    .into_iter()
                    .find(|p| p.uid.to_uppercase() == uid_upper)
                    .ok_or_else(|| {
                        format!(
                            "Plugin with UID '{}' not found. Run scan_plugins first.",
                            req.uid
                        )
                    })?
            }
        };

        // Load the VST3 module, wrapped in Arc so PluginInstance can hold a reference
        let module = Arc::new(
            VstModule::load(&info.path)
                .map_err(|e| format!("Failed to load module {}: {}", info.path.display(), e))?,
        );

        // Parse the UID hex string to TUID bytes
        let class_id = hex_to_tuid(&uid_upper)
            .map_err(|e| format!("Invalid UID format: {}", e))?;

        // Create host objects (HostApp::new() and ComponentHandler::new()
        // already return ComWrapper<T>)
        let host_app = HostApp::new();
        let handler = ComponentHandler::new();

        // Create plugin instance from factory (takes Arc<VstModule>)
        let mut instance = PluginInstance::from_factory(Arc::clone(&module), &class_id, host_app, handler)
            .map_err(|e| format!("Failed to create plugin instance: {}", e))?;

        // Setup, activate, start processing
        instance
            .setup(sample_rate as f64, 4096)
            .map_err(|e| format!("Plugin setup failed: {}", e))?;

        instance
            .activate()
            .map_err(|e| format!("Plugin activation failed: {}", e))?;

        instance
            .start_processing()
            .map_err(|e| format!("Start processing failed: {}", e))?;

        // Get info before storing
        let param_count = instance.get_parameter_count();
        let bus_info = instance.get_bus_info();

        // Store the module and plugin
        {
            let mut m = self.module.lock().map_err(|e| format!("Lock error: {}", e))?;
            *m = Some(module);
        }
        {
            let mut p = self.plugin.lock().map_err(|e| format!("Lock error: {}", e))?;
            *p = Some(instance);
        }
        {
            let mut pi = self.plugin_info.lock().map_err(|e| format!("Lock error: {}", e))?;
            *pi = Some(info.clone());
        }

        let response = serde_json::json!({
            "status": "loaded",
            "name": info.name,
            "vendor": info.vendor,
            "uid": info.uid,
            "sample_rate": sample_rate,
            "parameters": param_count,
            "buses": bus_info.len(),
        });

        info!("Plugin loaded: {} ({})", info.name, info.uid);
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Process an audio file through the loaded VST3 plugin. Outputs a WAV file. Call load_plugin first.")]
    fn process_audio(
        &self,
        Parameters(req): Parameters<ProcessAudioRequest>,
    ) -> Result<String, String> {
        info!(
            "process_audio called: {} -> {}",
            req.input_file, req.output_file
        );

        let mut plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_mut()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        // Decode input file
        let input_path = Path::new(&req.input_file);
        let decoded = audio::decode::decode_audio_file(input_path)
            .map_err(|e| format!("Failed to decode input file: {}", e))?;

        debug!(
            "Decoded {} frames, {} channels, {} Hz",
            decoded.total_frames, decoded.channels, decoded.sample_rate
        );

        // Re-setup plugin if sample rate differs from current setup
        // (This handles the case where plugin was set up at default 44100
        // but input file is 48000, etc.)
        // Sample rate mismatch is a hard error -- processing at the wrong rate
        // produces incorrect output (pitch shift, wrong time-based effects).
        plugin.re_setup(decoded.sample_rate as f64, 4096)
            .map_err(|e| format!(
                "Plugin does not support sample rate {} Hz: {}",
                decoded.sample_rate, e
            ))?;

        // Render through plugin
        let output_samples = audio::process::render_offline(plugin, &decoded)
            .map_err(|e| format!("Processing failed: {}", e))?;

        // Write output WAV
        let output_path = Path::new(&req.output_file);
        audio::encode::write_wav(
            output_path,
            &output_samples,
            decoded.channels as u16,
            decoded.sample_rate,
        )
        .map_err(|e| format!("Failed to write output file: {}", e))?;

        let output_frames = output_samples.len() / decoded.channels;
        let duration_secs = output_frames as f64 / decoded.sample_rate as f64;

        let response = serde_json::json!({
            "status": "processed",
            "input_file": req.input_file,
            "output_file": req.output_file,
            "sample_rate": decoded.sample_rate,
            "channels": decoded.channels,
            "input_frames": decoded.total_frames,
            "output_frames": output_frames,
            "duration_seconds": format!("{:.2}", duration_secs),
        });

        info!(
            "Audio processed: {} -> {} ({} frames)",
            req.input_file, req.output_file, output_frames
        );
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Save the current plugin state as a .vstpreset file. Call load_plugin first.")]
    fn save_preset(
        &self,
        Parameters(req): Parameters<SavePresetRequest>,
    ) -> Result<String, String> {
        info!("save_preset called: {}", req.path);

        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let preset_path = PathBuf::from(&req.path);
        state::save_plugin_state(plugin, &preset_path)
            .map_err(|e| format!("Failed to save preset: {}", e))?;

        let response = serde_json::json!({
            "status": "saved",
            "path": req.path,
        });

        info!("Preset saved: {}", req.path);
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Load a .vstpreset file into the currently loaded plugin. Call load_plugin first.")]
    fn load_preset(
        &self,
        Parameters(req): Parameters<LoadPresetRequest>,
    ) -> Result<String, String> {
        info!("load_preset called: {}", req.path);

        let mut plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_mut()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let preset_path = PathBuf::from(&req.path);
        state::restore_plugin_state(plugin, &preset_path)
            .map_err(|e| format!("Failed to load preset: {}", e))?;

        let response = serde_json::json!({
            "status": "loaded",
            "path": req.path,
        });

        info!("Preset loaded: {}", req.path);
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Get the loaded plugin's identity (classId, name, vendor). Call load_plugin first.")]
    fn get_plugin_info(&self) -> Result<String, String> {
        info!("get_plugin_info called");

        let info_guard = self
            .plugin_info
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let info = info_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let response = serde_json::json!({
            "classId": info.uid,
            "name": info.name,
            "vendor": info.vendor,
        });

        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "List all writable parameters with current values. Returns parameter ID, name, value, and display string for each. Call load_plugin first.")]
    fn list_params(&self) -> Result<String, String> {
        info!("list_params called");

        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let count = plugin.get_parameter_count();
        let mut parameters = Vec::new();

        for i in 0..count {
            if let Ok(info) = plugin.get_parameter_info(i) {
                // Only include writable, non-hidden parameters
                if info.is_writable() && !info.is_hidden() {
                    let value = plugin.get_parameter(info.id);
                    let display = plugin
                        .get_parameter_display(info.id)
                        .unwrap_or_else(|_| format!("{:.3}", value));

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
            "parameters": parameters,
            "count": parameters.len(),
        });

        info!("list_params found {} writable parameters", parameters.len());
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Get a single parameter's current value and display string. Call load_plugin first.")]
    fn get_param(
        &self,
        Parameters(req): Parameters<GetParamRequest>,
    ) -> Result<String, String> {
        info!("get_param called for id: {}", req.id);

        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let value = plugin.get_parameter(req.id);
        let display = plugin
            .get_parameter_display(req.id)
            .unwrap_or_else(|_| format!("{:.3}", value));

        let response = serde_json::json!({
            "id": req.id,
            "value": value,
            "display": display,
        });

        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Set a parameter value. Changes are queued and applied in next process_audio call. Value must be in range [0.0, 1.0]. Call load_plugin first.")]
    fn set_param(
        &self,
        Parameters(req): Parameters<SetParamRequest>,
    ) -> Result<String, String> {
        info!("set_param called: id={}, value={}", req.id, req.value);

        // Validate value range
        if req.value < 0.0 || req.value > 1.0 {
            return Err(format!(
                "Invalid parameter value: {}. Must be in range [0.0, 1.0]",
                req.value
            ));
        }

        let mut plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_mut()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        plugin.queue_parameter_change(req.id, req.value);

        let display_str = plugin
            .get_parameter_display(req.id)
            .unwrap_or_else(|_| format!("{:.3}", req.value));

        let response = serde_json::json!({
            "status": "queued",
            "id": req.id,
            "value": req.value,
            "display": display_str,
        });

        info!("Parameter {} queued: {} ({})", req.id, req.value, display_str);
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Set multiple parameters atomically. All values must be in range [0.0, 1.0]. Changes are queued and applied in next process_audio call. Call load_plugin first.")]
    fn batch_set(
        &self,
        Parameters(req): Parameters<BatchSetRequest>,
    ) -> Result<String, String> {
        info!("batch_set called with {} changes", req.changes.len());

        // Validate all changes first
        for change in &req.changes {
            if change.value < 0.0 || change.value > 1.0 {
                return Err(format!(
                    "Invalid parameter value for id {}: {}. Must be in range [0.0, 1.0]",
                    change.id, change.value
                ));
            }
        }

        let mut plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_mut()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        // Queue all changes
        for change in &req.changes {
            plugin.queue_parameter_change(change.id, change.value);
        }

        let response = serde_json::json!({
            "status": "queued",
            "changes_queued": req.changes.len(),
        });

        info!("Batch queued {} parameter changes", req.changes.len());
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(description = "Open the plugin's graphical editor window. Blocks until the window is closed by the user. Call load_plugin first.")]
    fn open_editor(&self) -> Result<String, String> {
        info!("open_editor called");

        // Verify a plugin is loaded and get the name
        let plugin_name = {
            let plugin_guard = self
                .plugin
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;

            if plugin_guard.is_none() {
                return Err("No plugin loaded. Call load_plugin first.".to_string());
            }

            // Get plugin name for window title
            let info_guard = self
                .plugin_info
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            info_guard
                .as_ref()
                .map(|i| i.name.clone())
                .unwrap_or_else(|| "Unknown Plugin".to_string())
        };

        // #region agent log
        agent_log(
            "pre-fix",
            "H0",
            "src/server.rs:open_editor",
            "open_editor spawning GUI thread",
            serde_json::json!({
                "plugin_name": plugin_name,
                "wayland_display_set": std::env::var("WAYLAND_DISPLAY").is_ok(),
                "x11_display": std::env::var("DISPLAY").ok(),
                "winit_backend": std::env::var("WINIT_UNIX_BACKEND").ok(),
            }),
        );
        // #endregion

        // Clone Arc references for the GUI thread
        let plugin_arc = Arc::clone(&self.plugin);

        // Channel to signal window close
        let (tx, rx) = std::sync::mpsc::channel();

        // Spawn dedicated GUI thread (winit event loop is blocking
        // and incompatible with Tokio's async runtime)
        std::thread::spawn(move || {
            // #region agent log
            agent_log(
                "pre-fix",
                "H0",
                "src/server.rs:open_editor",
                "GUI thread started; calling open_editor_window",
                serde_json::json!({}),
            );
            // #endregion
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                gui::open_editor_window(plugin_arc, plugin_name)
            }));

            match result {
                Ok(Ok(())) => {
                    info!("Editor window closed normally");
                    let _ = tx.send(Ok(()));
                }
                Ok(Err(e)) => {
                    tracing::error!("Editor window failed: {}", e);
                    // #region agent log
                    agent_log(
                        "pre-fix",
                        "H0",
                        "src/server.rs:open_editor",
                        "open_editor_window returned Err",
                        serde_json::json!({"error": e}),
                    );
                    // #endregion
                    let _ = tx.send(Err(format!("Editor error: {}", e)));
                }
                Err(panic_payload) => {
                    // #region agent log
                    agent_log(
                        "pre-fix",
                        "H0",
                        "src/server.rs:open_editor",
                        "GUI thread panicked while running open_editor_window",
                        serde_json::json!({
                            "panic": if let Some(s) = panic_payload.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "non-string panic payload".to_string()
                            }
                        }),
                    );
                    // #endregion
                    let _ = tx.send(Err("GUI thread panicked".to_string()));
                }
            };
        });

        // Block until window closes
        let result = rx
            .recv()
            .map_err(|_| "GUI thread panicked or disconnected".to_string())?;

        // #region agent log
        agent_log(
            "pre-fix",
            "H0",
            "src/server.rs:open_editor",
            "GUI thread returned to open_editor",
            serde_json::json!({
                "result_is_ok": result.is_ok(),
                "result_err": result.as_ref().err().cloned(),
            }),
        );
        // #endregion

        result.map(|()| {
            let response = serde_json::json!({
                "status": "closed",
                "message": "Editor window closed",
            });
            serde_json::to_string_pretty(&response).unwrap()
        })
    }
}

#[tool_handler]
impl ServerHandler for AudioHost {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "VST3 audio processing host. Scan for plugins, load one, \
                 process audio files through it, and manage presets."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

/// Convert a 32-character hex string to a 16-byte TUID.
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
