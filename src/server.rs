//! MCP server with tool definitions for VST3 audio processing.
//!
//! Exposes plugin scanning, loading, audio processing, and preset management
//! as MCP tools over stdio transport.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use tracing::{debug, info};

use vst3_mcp_host::audio;
use vst3_mcp_host::doc_search;
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
pub struct GetParamsByNameRequest {
    /// List of parameter names to fuzzy match.
    #[schemars(description = "List of parameter names to fuzzy match")]
    pub names: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetPatchStateRequest {
    /// Optional: Only return parameters that differ from default.
    #[schemars(
        description = "Optional: Only return parameters that differ from default (default: true)"
    )]
    pub diff_only: Option<bool>,
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
pub struct ProbeParamRequest {
    /// Parameter ID to probe.
    #[schemars(description = "Parameter ID to probe")]
    pub id: u32,
    /// List of normalized values [0.0, 1.0] to test.
    #[schemars(description = "List of normalized values [0.0, 1.0] to test")]
    pub values: Vec<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchParamsRequest {
    /// Query string to fuzzy match against parameter names.
    #[schemars(description = "Query string to fuzzy match against parameter names")]
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchPluginDocsRequest {
    /// Plugin name or UID to search (e.g., "Serum").
    #[schemars(description = "Plugin name or UID to search (e.g., Serum)")]
    pub plugin_name: String,
    /// Targeted feature/parameter question (e.g., "LFO routing").
    #[schemars(description = "Targeted feature/parameter question (e.g., LFO routing)")]
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchSoundDesignGuideRequest {
    /// Broad sound design topic or target outcome (e.g., "vocal compression").
    #[schemars(
        description = "Broad sound design topic or target outcome (e.g., vocal compression)"
    )]
    pub topic: String,
    /// Optional deeper query to refine the recipe search.
    #[schemars(description = "Optional deeper query to refine the recipe search")]
    pub query: Option<String>,
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
pub struct ListParamsRequest {
    /// Optional name prefix filter (case-insensitive). Returns only params whose names start with this prefix. Use list_param_groups to discover valid prefixes.
    #[schemars(
        description = "Optional name prefix filter (case-insensitive). Use list_param_groups to discover valid prefixes."
    )]
    pub prefix: Option<String>,
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
    editor_close_signal: Arc<AtomicBool>,
    /// Whether the editor window is currently open (set by persistent loop).
    editor_is_open: Arc<AtomicBool>,
    /// Plugin name for the editor window title (shared with GUI thread).
    editor_plugin_name: Arc<RwLock<String>>,
    /// Handle to the persistent GUI thread. Never joined — thread runs for process lifetime.
    editor_thread: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    /// Cached param metadata (id, name, units, step_count). Populated on first find_vst_parameter/list_param_groups call.
    /// Invalidated on load_plugin/unload_plugin. Avoids re-fetching static metadata on every search.
    param_cache: Arc<Mutex<Option<Vec<serde_json::Value>>>>,
    tool_router: ToolRouter<Self>,
}

impl AudioHost {
    pub fn new() -> Self {
        Self {
            plugin: Arc::new(Mutex::new(None)),
            plugin_info: Arc::new(Mutex::new(None)),
            module: Arc::new(Mutex::new(None)),
            scan_cache: Arc::new(Mutex::new(Vec::new())),
            editor_close_signal: Arc::new(AtomicBool::new(true)),
            editor_is_open: Arc::new(AtomicBool::new(false)),
            editor_plugin_name: Arc::new(RwLock::new(String::new())),
            editor_thread: Arc::new(Mutex::new(None)),
            param_cache: Arc::new(Mutex::new(None)),
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
        description = "List all writable parameters/knobs with current values (value, display, units, steps). Supports optional prefix filter to narrow results (e.g. prefix='Reverb'). Use list_param_groups to discover valid prefixes. Call load_plugin first."
    )]
    fn list_params(
        &self,
        Parameters(req): Parameters<ListParamsRequest>,
    ) -> Result<String, String> {
        info!("list_params called (prefix={:?})", req.prefix);

        let mut parameters = self.get_live_params()?;

        // Update metadata cache as a side effect (metadata-only, no live values)
        {
            let metadata: Vec<serde_json::Value> = parameters
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p["id"],
                        "name": p["name"],
                        "units": p["units"],
                        "step_count": p["step_count"],
                    })
                })
                .collect();
            if let Ok(mut cache) = self.param_cache.lock() {
                *cache = Some(metadata);
            }
        }

        // Apply optional prefix filter
        if let Some(ref pfx) = req.prefix {
            let pfx_lower = pfx.to_lowercase();
            parameters.retain(|p| {
                p.get("name")
                    .and_then(|v| v.as_str())
                    .map(|n| n.to_lowercase().starts_with(&pfx_lower))
                    .unwrap_or(false)
            });
        }

        let response = serde_json::json!({
            "parameters": parameters,
            "count": parameters.len(),
        });

        info!("list_params returning {} parameters", parameters.len());
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "List logical parameter groups (e.g. 'Reverb', 'Envelope 1', 'Filter 1') with counts. Use before list_params or find_vst_parameter to discover available sections. Use a group name as the prefix in list_params to narrow results. Call load_plugin first."
    )]
    fn list_param_groups(&self) -> Result<String, String> {
        info!("list_param_groups called");

        let metadata = self.get_cached_param_metadata()?;
        let mut group_counts: HashMap<String, usize> = HashMap::new();

        for param in &metadata {
            let name = param
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let group = param_group_prefix(name);
            *group_counts.entry(group).or_insert(0) += 1;
        }

        let mut groups: Vec<serde_json::Value> = group_counts
            .into_iter()
            .map(|(group, count)| serde_json::json!({ "group": group, "count": count }))
            .collect();
        groups.sort_by(|a, b| {
            a.get("group")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .cmp(b.get("group").and_then(|v| v.as_str()).unwrap_or_default())
        });
        let group_count = groups.len();

        let response = serde_json::json!({
            "groups": groups,
            "count": group_count,
        });
        info!("list_param_groups found {} groups", group_count);
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Simulate parameter values to see their display strings (e.g., '100 Hz', 'On') without changing the plugin state."
    )]
    fn probe_param(
        &self,
        Parameters(req): Parameters<ProbeParamRequest>,
    ) -> Result<String, String> {
        info!("probe_param called: id={}, values={:?}", req.id, req.values);

        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let mut results = Vec::new();

        for &val in &req.values {
            if val < 0.0 || val > 1.0 {
                return Err(format!("Invalid value {}. Must be [0.0, 1.0]", val));
            }

            let display = plugin
                .get_parameter_display_for_value(req.id, val)
                .unwrap_or_else(|_| format!("{:.3}", val));

            results.push(serde_json::json!({
                "value": val,
                "display": display
            }));
        }

        let response = serde_json::json!({
            "id": req.id,
            "probes": results
        });

        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "Search for parameters by name. Useful for plugins with many parameters where list_params is too large."
    )]
    fn search_params(
        &self,
        Parameters(req): Parameters<SearchParamsRequest>,
    ) -> Result<String, String> {
        info!("search_params called: query='{}'", req.query);

        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let count = plugin.get_parameter_count();
        let query_lower = req.query.to_lowercase();
        let mut matches = Vec::new();

        for i in 0..count {
            if let Ok(info) = plugin.get_parameter_info(i) {
                // Filter: must match query
                if info.title.to_lowercase().contains(&query_lower) {
                    let value = plugin.get_parameter(info.id);
                    let display = plugin
                        .get_parameter_display(info.id)
                        .unwrap_or_else(|_| format!("{:.3}", value));

                    matches.push(serde_json::json!({
                        "id": info.id,
                        "name": info.title,
                        "value": value,
                        "display": display,
                        "units": info.units,
                        "step_count": info.step_count
                    }));
                }
            }
        }

        let response = serde_json::json!({
            "query": req.query,
            "matches": matches,
            "count": matches.len()
        });

        info!("search_params found {} matches", matches.len());
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "Search local plugin documentation by plugin_name + query and return only top excerpts (no full file dumps)."
    )]
    fn search_plugin_docs(
        &self,
        Parameters(req): Parameters<SearchPluginDocsRequest>,
    ) -> Result<String, String> {
        info!(
            "search_plugin_docs called: plugin='{}', query='{}'",
            req.plugin_name, req.query
        );
        let response = doc_search::search_plugin_docs(&req.plugin_name, &req.query)?;
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
    }

    #[tool(
        description = "Search local sound-design guides by topic (and optional query) and return top recipe excerpts."
    )]
    fn search_sound_design_guide(
        &self,
        Parameters(req): Parameters<SearchSoundDesignGuideRequest>,
    ) -> Result<String, String> {
        info!(
            "search_sound_design_guide called: topic='{}', query='{:?}'",
            req.topic, req.query
        );
        let response = doc_search::search_sound_design_guide(&req.topic, req.query.as_deref())?;
        serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
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
        description = "Get detailed parameter metadata including step labels for enumerated parameters. Call load_plugin first."
    )]
    fn get_param_info(
        &self,
        Parameters(req): Parameters<GetParamRequest>,
    ) -> Result<String, String> {
        info!("get_param_info called for id: {}", req.id);

        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        let info = plugin
            .get_parameter_info(req.id as i32)
            .map_err(|e| format!("Failed to get info for {}: {}", req.id, e))?;

        let value = plugin.get_parameter(req.id);
        let display = plugin
            .get_parameter_display(req.id)
            .unwrap_or_else(|_| format!("{:.3}", value));

        // Fetch step labels if step_count > 0 (e.g. "Sine", "Square")
        let step_labels = if info.step_count > 0 {
            plugin
                .get_parameter_step_labels(req.id, info.step_count)
                .ok()
        } else {
            None
        };

        let response = serde_json::json!({
            "id": req.id,
            "name": info.title,
            "value": value,
            "display": display,
            "units": info.units,
            "step_count": info.step_count,
            "default_normalized": info.default_normalized,
            "flags": info.flags,
            "step_labels": step_labels,
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

        let mut results = Vec::new();
        let mut plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let plugin = plugin_guard
            .as_mut()
            .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

        // Queue all changes
        for change in &req.changes {
            let clamped = change.value.clamp(0.0, 1.0);
            plugin.queue_parameter_change(change.id, clamped);

            // Try to get display for confirmation
            let display = plugin
                .get_parameter_display_for_value(change.id, clamped)
                .unwrap_or_else(|_| format!("{:.3}", clamped));

            results.push(serde_json::json!({
                "id": change.id,
                "requested": change.value,
                "applied": clamped,
                "clamped": (change.value - clamped).abs() > 1e-6,
                "display": display,
            }));
        }

        let response = serde_json::json!({
            "status": "queued",
            "changes_queued": req.changes.len(),
            "results": results,
        });

        info!("Batch queued {} parameter changes", req.changes.len());
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "Batch lookup of parameter IDs by name (fuzzy match). Returns best match for each query."
    )]
    fn get_params_by_name(
        &self,
        Parameters(req): Parameters<GetParamsByNameRequest>,
    ) -> Result<String, String> {
        let params = self.get_cached_param_metadata()?;
        let mut results = Vec::new();

        for name in req.names {
            let (primary, aliases) = query_terms(&name);
            let mut scored: Vec<(u32, serde_json::Value)> = params
                .iter()
                .filter_map(|p| {
                    let s = score_param(p, &primary, &aliases);
                    if s > 0 { Some((s, p.clone())) } else { None }
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));

            if let Some((score, best_match)) = scored.first() {
                results.push(serde_json::json!({
                    "query": name,
                    "match": best_match,
                    "score": score
                }));
            } else {
                results.push(serde_json::json!({
                    "query": name,
                    "match": null,
                    "score": 0
                }));
            }
        }

        let response = serde_json::json!({
            "results": results,
            "count": results.len()
        });
        Ok(serde_json::to_string_pretty(&response).unwrap())
    }

    #[tool(
        description = "Get current patch state (all non-default parameters). Useful for verifying changes or saving partial presets."
    )]
    fn get_patch_state(
        &self,
        Parameters(req): Parameters<GetPatchStateRequest>,
    ) -> Result<String, String> {
        let diff_only = req.diff_only.unwrap_or(true);
        let plugin_guard = self
            .plugin
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let plugin = plugin_guard
            .as_ref()
            .ok_or_else(|| "No plugin loaded.".to_string())?;

        let count = plugin.get_parameter_count();
        let mut result = Vec::new();

        for i in 0..count {
            if let Ok(info) = plugin.get_parameter_info(i) {
                let value = plugin.get_parameter(info.id);

                if diff_only && (value - info.default_normalized).abs() < 1e-4 {
                    continue;
                }

                let display = plugin
                    .get_parameter_display(info.id)
                    .unwrap_or_else(|_| format!("{:.3}", value));

                result.push(serde_json::json!({
                    "id": info.id,
                    "name": info.title,
                    "value": value,
                    "display": display,
                    "default": info.default_normalized
                }));
            }
        }

        let response = serde_json::json!({
            "parameters": result,
            "count": result.len(),
        });
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
        description = "Search plugin parameters by natural language (e.g. 'attack', 'release', 'make brighter', 'reduce reverb'). Results are ranked by relevance. Uses cached metadata for speed."
    )]
    fn find_vst_parameter(
        &self,
        Parameters(req): Parameters<FindVstParameterRequest>,
    ) -> Result<String, String> {
        let params = self.get_cached_param_metadata()?;
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
        let params = self.get_live_params()?;
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

        // Close any existing editor first (does not join — thread stays alive)
        self.close_editor_inner();

        // If the persistent editor thread is already running, re-open using the existing loop.
        // The thread must never exit (winit allows only one EventLoop per process).
        let has_thread = {
            let thread_guard = self.editor_thread.lock().map_err(|e| format!("Lock error: {}", e))?;
            if let Some(handle) = thread_guard.as_ref() {
                if handle.is_finished() {
                    return Err("Editor thread exited unexpectedly; restart the MCP server to recover.".to_string());
                }
                if let Ok(mut name) = self.editor_plugin_name.write() {
                    *name = plugin_name.clone();
                }
                self.editor_close_signal.store(false, Ordering::Relaxed);
                let was_open = self.editor_is_open.load(Ordering::Relaxed);
                if was_open {
                    let response = serde_json::json!({
                        "status": "opened",
                        "message": "Editor window is already open.",
                    });
                    return Ok(serde_json::to_string_pretty(&response).unwrap());
                }
                true
            } else {
                false
            }
        };

        if !has_thread {
            return self.start_editor_thread(plugin_name);
        }

        // Wait for the persistent loop to create the window
        for _ in 0..500 {
            if self.editor_is_open.load(Ordering::Relaxed) {
                let response = serde_json::json!({
                    "status": "opened",
                    "message": "Editor window is now open. You may continue — the window stays open in the background.",
                });
                return Ok(serde_json::to_string_pretty(&response).unwrap());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Timed out waiting for editor window to open".to_string())
    }

    fn start_editor_thread(&self, plugin_name: String) -> Result<String, String> {
        let (opened_tx, opened_rx) = std::sync::mpsc::channel();

        let handle = {
            let thread_guard = self.editor_thread.lock().map_err(|e| format!("Lock error: {}", e))?;
            if thread_guard.is_some() {
                // Another caller beat us to spawning; reuse the existing thread
                drop(thread_guard);
                return self.open_editor();
            }
            if let Ok(mut name) = self.editor_plugin_name.write() {
                *name = plugin_name.clone();
            }
            self.editor_close_signal.store(false, Ordering::Relaxed);
            self.editor_is_open.store(false, Ordering::Relaxed);

            let plugin_arc = Arc::clone(&self.plugin);
            let name_arc = Arc::clone(&self.editor_plugin_name);
            let close_signal = Arc::clone(&self.editor_close_signal);
            let is_open = Arc::clone(&self.editor_is_open);
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    gui::open_editor_window_persistent(
                        plugin_arc,
                        name_arc,
                        opened_tx,
                        close_signal,
                        is_open,
                    )
                }));

                if let Ok(Err(e)) = result {
                    tracing::error!("Persistent editor loop error: {}", e);
                } else if let Err(payload) = result {
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        (*s).to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "non-string panic payload".to_string()
                    };
                    tracing::error!("Editor thread panicked: {}", msg);
                }
            })
        };

        {
            let mut thread_guard = self.editor_thread.lock().map_err(|e| format!("Lock error: {}", e))?;
            *thread_guard = Some(handle);
        }

        match opened_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => {
                let response = serde_json::json!({
                    "status": "opened",
                    "message": "Editor window is now open. You may continue — the window stays open in the background.",
                });
                Ok(serde_json::to_string_pretty(&response).unwrap())
            }
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err("Timed out waiting for editor window to open".to_string())
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("Editor thread exited before reporting open state".to_string())
            }
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
    /// Does NOT join the thread — it must keep running so we can reopen (winit allows only
    /// one EventLoop per process).
    fn close_editor_inner(&self) -> bool {
        let has_thread = self
            .editor_thread
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false);

        if !has_thread {
            return false;
        }

        let was_open = self.editor_is_open.load(Ordering::Relaxed);
        self.editor_close_signal.store(true, Ordering::Relaxed);

        // Wait for the persistent loop to cleanup and set is_open=false (up to 2s)
        for _ in 0..200 {
            if !self.editor_is_open.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        was_open
    }

    /// Fetch all writable, visible parameters with live values from the plugin.
    fn get_live_params(&self) -> Result<Vec<serde_json::Value>, String> {
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
                let value = plugin.get_parameter(info.id);
                let display = plugin
                    .get_parameter_display(info.id)
                    .unwrap_or_else(|_| format!("{:.3}", value));

                parameters.push(serde_json::json!({
                    "id": info.id,
                    "name": info.title,
                    "value": value,
                    "display": display,
                    "units": info.units,
                    "step_count": info.step_count
                }));
            }
        }
        Ok(parameters)
    }

    /// Get parameter metadata (id, name, units, step_count) from cache, or fetch and cache if miss.
    /// Does not include live values — use get_live_params() when current values are needed.
    fn get_cached_param_metadata(&self) -> Result<Vec<serde_json::Value>, String> {
        // Fast path: return from cache
        {
            let cache = self
                .param_cache
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            if let Some(ref cached) = *cache {
                return Ok(cached.clone());
            }
        }

        // Cache miss: fetch from plugin (metadata only, no live values)
        let metadata = {
            let plugin_guard = self
                .plugin
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            let plugin = plugin_guard
                .as_ref()
                .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

            let count = plugin.get_parameter_count();
            let mut meta = Vec::new();
            for i in 0..count {
                if let Ok(info) = plugin.get_parameter_info(i) {
                    meta.push(serde_json::json!({
                        "id": info.id,
                        "name": info.title,
                        "units": info.units,
                        "step_count": info.step_count,
                    }));
                }
            }
            meta
        };

        // Store in cache
        if let Ok(mut cache) = self.param_cache.lock() {
            *cache = Some(metadata.clone());
        }
        Ok(metadata)
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
            if let Ok(mut cache) = self.param_cache.lock() {
                *cache = None;
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
When editing an unfamiliar plugin, call search_plugin_docs first to retrieve targeted plugin-specific mapping/quirk excerpts before changing parameters.\n\
When asked for a desired audio outcome (for example a reese bass or vocal compression chain), call search_sound_design_guide first for a recipe, then map to plugin parameters.\n\
Workflow: scan_plugins -> load_plugin -> find_vst_parameter (search params) -> preview_vst_parameter_values (probe param) -> set_param/batch_set (or edit_vst_patch) -> save_preset.\n\
Use this workflow before web search."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
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
        if name == *term {
            score += 2000;
        }
        // exact full name match
        else if name_words.iter().any(|w| *w == term.as_str()) {
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

/// Extract the group prefix from a parameter name.
/// Rule: if second word is a number → group = "Word1 N"; else group = "Word1".
fn param_group_prefix(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(3, ' ').collect();
    if parts.len() >= 2 && parts[1].parse::<u32>().is_ok() {
        format!("{} {}", parts[0], parts[1])
    } else {
        parts[0].to_string()
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
