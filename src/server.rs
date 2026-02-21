//! MCP server with tool definitions for VST3 audio processing.
//!
//! Exposes plugin scanning, loading, audio processing, and preset management
//! as MCP tools over stdio transport.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use tracing::{debug, info, warn};

use vst3_mcp_host::audio;
use vst3_mcp_host::gui;
use vst3_mcp_host::hosting::host_app::{ComponentHandler, HostApp};
use vst3_mcp_host::hosting::module::VstModule;
use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::hosting::scanner;
use vst3_mcp_host::hosting::types::PluginInfo;
use vst3_mcp_host::preset::state;

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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindVstParameterRequest {
    /// Natural-language parameter query (e.g. "attack", "make brighter", "reduce reverb").
    #[schemars(description = "Natural-language parameter query")]
    pub query: String,
    /// Maximum number of matches to return (default 20).
    #[schemars(description = "Maximum number of matches to return (default 20)")]
    pub limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewVstParameterValuesRequest {
    /// Optional list of parameter IDs to inspect. If omitted, returns the first `limit` params.
    #[schemars(description = "Optional list of parameter IDs to inspect")]
    pub ids: Option<Vec<u32>>,
    /// Maximum number of values to return (default 20).
    #[schemars(description = "Maximum number of values to return (default 20)")]
    pub limit: Option<usize>,
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
    /// Close signal for the editor window (shared with the GUI thread).
    editor_close_signal: Arc<std::sync::atomic::AtomicBool>,
    /// Handle to the GUI thread so we can join/detect if it's still alive.
    editor_thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    tool_router: ToolRouter<Self>,
}

impl AudioHost {
    pub fn new() -> Self {
        Self {
            plugin: Arc::new(Mutex::new(None)),
            plugin_info: Arc::new(Mutex::new(None)),
            module: Arc::new(Mutex::new(None)),
            scan_cache: Arc::new(Mutex::new(Vec::new())),
            editor_close_signal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            editor_thread: Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl AudioHost {
    #[tool(
        description = "Unload the currently loaded plugin, closing its editor. This is called automatically by load_plugin."
    )]
    fn unload_plugin(&self) -> Result<String, String> {
        info!("unload_plugin called");
        let was_loaded = self.unload_plugin_inner()?;
        let response = serde_json::json!({
            "status": if was_loaded { "unloaded" } else { "not_loaded" },
            "message": if was_loaded {
                "Plugin has been unloaded."
            } else {
                "No plugin was loaded."
            },
        });
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "Scan installed VST3 plugins. Use first when user says plugin/VST/synth/preset/patch/sound/tone."
    )]
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

    #[tool(
        description = "Load a VST3 plugin by UID from scan results. Use for requests like 'load plugin', 'open Serum', or 'edit this preset'."
    )]
    fn load_plugin(
        &self,
        Parameters(req): Parameters<LoadPluginRequest>,
    ) -> Result<String, String> {
        info!("load_plugin called with uid: {}", req.uid);

        // --- UNLOAD PREVIOUS PLUGIN ---
        // This is critical to ensure the old plugin and its editor are
        // completely torn down before loading a new one.
        self.unload_plugin_inner()?;
        // --- END UNLOAD ---

        let sample_rate = req.sample_rate.unwrap_or(44100);
        let uid_upper = req.uid.to_uppercase();

        // Find the plugin in scan cache
        let plugin_info = {
            let cache = self
                .scan_cache
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            cache
                .iter()
                .find(|p| p.uid.to_uppercase() == uid_upper)
                .cloned()
        };

        let info = match plugin_info {
            Some(info) => info,
            None => {
                // Try a fresh scan
                let plugins =
                    scanner::scan_plugins(None).map_err(|e| format!("Scan failed: {}", e))?;

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
        let class_id = hex_to_tuid(&uid_upper).map_err(|e| format!("Invalid UID format: {}", e))?;

        // Create host objects (HostApp::new() and ComponentHandler::new()
        // already return ComWrapper<T>)
        let host_app = HostApp::new();
        let handler = ComponentHandler::new();

        // Create plugin instance from factory (takes Arc<VstModule>)
        let mut instance =
            PluginInstance::from_factory(Arc::clone(&module), &class_id, host_app, handler)
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
            let mut m = self
                .module
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            *m = Some(module);
        }
        {
            let mut p = self
                .plugin
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            *p = Some(instance);
        }
        {
            let mut pi = self
                .plugin_info
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
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

    #[tool(
        description = "Process an audio file through the loaded VST3 plugin. Outputs a WAV file. Call load_plugin first."
    )]
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
        plugin
            .re_setup(decoded.sample_rate as f64, 4096)
            .map_err(|e| {
                format!(
                    "Plugin does not support sample rate {} Hz: {}",
                    decoded.sample_rate, e
                )
            })?;

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

    #[tool(
        description = "Save the current plugin state as a .vstpreset file. Use after patch/preset/tone edits."
    )]
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

    #[tool(
        description = "Load a .vstpreset file into the currently loaded plugin. Call load_plugin first."
    )]
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

    #[tool(
        description = "Get the loaded plugin's identity (classId, name, vendor). Call load_plugin first."
    )]
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

    #[tool(
        description = "List all writable parameters/knobs with current values. Use when user says parameter/knob/automation/make brighter/reduce reverb."
    )]
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

    #[tool(
        description = "Get a single parameter's current value and display string. Call load_plugin first."
    )]
    fn get_param(&self, Parameters(req): Parameters<GetParamRequest>) -> Result<String, String> {
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

    #[tool(
        description = "Set one parameter value (single knob tweak). Value must be in [0.0, 1.0]."
    )]
    fn set_param(&self, Parameters(req): Parameters<SetParamRequest>) -> Result<String, String> {
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

        info!(
            "Parameter {} queued: {} ({})",
            req.id, req.value, display_str
        );
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "Set multiple parameters atomically for coordinated tone/patch/preset edits. Values must be in [0.0, 1.0]."
    )]
    fn batch_set(&self, Parameters(req): Parameters<BatchSetRequest>) -> Result<String, String> {
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

    #[tool(description = "Alias for batch_set. Edit VST patch/preset/sound via parameter changes.")]
    fn edit_vst_patch(
        &self,
        Parameters(req): Parameters<BatchSetRequest>,
    ) -> Result<String, String> {
        self.batch_set(Parameters(req))
    }

    #[tool(
        description = "Search plugin parameters by natural language (e.g. 'attack', 'release', 'make brighter', 'reduce reverb')."
    )]
    fn find_vst_parameter(
        &self,
        Parameters(req): Parameters<FindVstParameterRequest>,
    ) -> Result<String, String> {
        let raw = self.list_params()?;
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
            "next_step": "Use preview_vst_parameter_values, then set_param/batch_set (or edit_vst_patch).",
        });
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Preview current values for selected parameter IDs before editing a patch/preset/tone. If ids are omitted, returns first N parameters."
    )]
    fn preview_vst_parameter_values(
        &self,
        Parameters(req): Parameters<PreviewVstParameterValuesRequest>,
    ) -> Result<String, String> {
        let raw = self.list_params()?;
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
        description = "Open the plugin's graphical editor window. Returns immediately once the editor is visible; the window stays open in the background. If an editor is already open it will be closed first. Call load_plugin first."
    )]
    fn open_editor(&self) -> Result<String, String> {
        info!("open_editor called");

        // Close any existing editor first
        self.close_editor_inner();

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

        // Reset close signal for the new editor session
        self.editor_close_signal
            .store(false, std::sync::atomic::Ordering::Relaxed);

        // Clone Arc references for the GUI thread
        let plugin_arc = Arc::clone(&self.plugin);
        let close_signal = Arc::clone(&self.editor_close_signal);

        // Channel: GUI thread signals when editor is open (or failed).
        let (opened_tx, opened_rx) = std::sync::mpsc::channel();

        // Spawn dedicated GUI thread
        let opened_tx_clone = opened_tx.clone();
        let handle = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                gui::open_editor_window(plugin_arc, plugin_name, opened_tx_clone, close_signal)
            }));

            match result {
                Ok(Ok(())) => {
                    info!("Editor window closed normally");
                }
                Ok(Err(e)) => {
                    tracing::error!("Editor window failed: {}", e);
                    let _ = opened_tx.send(Err(format!("Editor error: {}", e)));
                }
                Err(panic_payload) => {
                    let msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        (*s).to_string()
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "non-string panic payload".to_string()
                    };
                    tracing::error!("GUI thread panicked: {}", msg);
                    let _ = opened_tx.send(Err(format!("GUI thread panicked: {}", msg)));
                }
            };
        });

        // Store the thread handle
        if let Ok(mut thread_guard) = self.editor_thread.lock() {
            *thread_guard = Some(handle);
        }

        // Wait only until the editor window is created (or fails), then return.
        let opened_result = opened_rx
            .recv()
            .map_err(|_| "GUI thread disconnected before signalling editor state".to_string())?;

        match opened_result {
            Ok(()) => {
                let response = serde_json::json!({
                    "status": "opened",
                    "message": "Editor window is now open. You may continue — the window stays open in the background.",
                });
                Ok(serde_json::to_string_pretty(&response).unwrap())
            }
            Err(e) => Err(e),
        }
    }

    #[tool(
        description = "Close the plugin's graphical editor window. Safe to call even if no editor is open."
    )]
    fn close_editor(&self) -> Result<String, String> {
        info!("close_editor called");
        let was_open = self.close_editor_inner();
        let response = serde_json::json!({
            "status": if was_open { "closed" } else { "not_open" },
            "message": if was_open {
                "Editor window has been closed."
            } else {
                "No editor window was open."
            },
        });
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }
}

impl AudioHost {
    /// Internal helper to close any running editor. Returns true if an editor was running.
    fn close_editor_inner(&self) -> bool {
        // Signal the event loop to exit
        self.editor_close_signal
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // Wait for the GUI thread to finish (with timeout)
        let handle = {
            let mut thread_guard = match self.editor_thread.lock() {
                Ok(g) => g,
                Err(_) => return false,
            };
            thread_guard.take()
        };

        if let Some(h) = handle {
            // The editor thread MUST be joined to ensure all resources are released
            // before we proceed to drop the plugin instance. A timeout is dangerous here,
            // as it could leave the GUI thread running with a dangling pointer.
            if let Err(e) = h.join() {
                warn!("Editor thread panicked on close: {:?}", e);
            }
            true
        } else {
            false
        }
    }

    /// Internal helper to unload the current plugin and all its resources.
    fn unload_plugin_inner(&self) -> Result<bool, String> {
        // 1. Close the editor window and wait for its thread to terminate.
        // This is the most critical step to prevent dangling pointers.
        self.close_editor_inner();

        // 2. Lock and take the plugin instance, causing it to be dropped.
        let mut plugin_guard = self.plugin.lock().map_err(|e| format!("Lock error: {e}"))?;
        let was_loaded = if plugin_guard.is_some() {
            info!("Unloading existing plugin instance.");
            *plugin_guard = None;
            true
        } else {
            false
        };

        // 3. Clear other related state.
        if was_loaded {
            if let Ok(mut info_guard) = self.plugin_info.lock() {
                *info_guard = None;
            }
            if let Ok(mut module_guard) = self.module.lock() {
                *module_guard = None;
            }
            info!("Plugin unloaded and resources released.");
        }

        Ok(was_loaded)
    }
}

#[tool_handler]
impl ServerHandler for AudioHost {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "VST3 audio processing host.\n\
Hard routing rule: If user mentions VST/plugin/preset/patch/sound/tone/parameter/knob/automation, use Audio MCP tools first. Do not use web search unless user explicitly asks for docs/news.\n\
Disambiguation: In audio context, patch = preset/sound configuration, not code diff.\n\
Workflow: scan_plugins -> load_plugin -> find_vst_parameter (search params) -> preview_vst_parameter_values (probe param) -> set_param/batch_set (or edit_vst_patch) -> save_preset.\n\
Use this workflow before web search."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
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

    terms.iter().any(|term| haystack.contains(term))
}

fn parse_params_from_list_result(raw: &str) -> Result<Vec<serde_json::Value>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Failed to parse list_params JSON: {e}"))?;
    let params = parsed
        .get("parameters")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "list_params response is missing a 'parameters' array".to_string())?;
    Ok(params.clone())
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
