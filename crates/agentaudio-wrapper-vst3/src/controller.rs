//! Dedicated controller thread for IEditController calls.
//!
//! VST3 requires IEditController (getParameterInfo, getParamNormalized, etc.) to be
//! called from a single designated thread. The MCP server runs on Tokio worker threads,
//! so we route all IEditController operations through this controller thread to avoid
//! threading violations that cause hangs, data races, or wrong behavior in plugins like Vital.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread;

use vst3_mcp_host::hosting::plugin::PluginInstance;
use vst3_mcp_host::preset;

use crate::param_scoring::{param_group_prefix, query_terms_for_scoring, score_param};

/// Commands that touch IEditController. Processed on the controller thread.
#[derive(Debug)]
pub enum ControllerCmd {
    ListParams { prefix: Option<String> },
    ListParamGroups,
    SearchParams { query: String },
    GetParamsByName { names: Vec<String> },
    GetPatchState { diff_only: bool },
    GetParamInfo { id: u32 },
    PreviewParams { ids: Option<Vec<u32>>, limit: usize },
    SavePreset { path: PathBuf },
    LoadPreset { path: PathBuf },
}

/// Spawn the controller thread. Returns a sender; the thread runs until the sender is dropped.
pub fn spawn(shared: crate::SharedState) -> SyncSender<(ControllerCmd, SyncSender<Result<String, String>>)> {
    let (tx, rx) = std::sync::mpsc::sync_channel(0);

    thread::Builder::new()
        .name("agentaudio-controller".into())
        .spawn(move || controller_loop(shared, rx))
        .expect("Failed to spawn controller thread");

    tx
}

fn controller_loop(
    shared: crate::SharedState,
    rx: Receiver<(ControllerCmd, SyncSender<Result<String, String>>)>,
) {
    while let Ok((cmd, resp_tx)) = rx.recv() {
        let result = execute_cmd(&shared, cmd);
        let _ = resp_tx.send(result);
    }
}

fn execute_cmd(shared: &crate::SharedState, cmd: ControllerCmd) -> Result<String, String> {
    let mut guard = shared
        .child_plugin
        .lock()
        .map_err(|e| format!("Lock error: {e}"))?;
    let plugin = guard
        .as_mut()
        .ok_or_else(|| "No child plugin loaded".to_string())?;

    match cmd {
        ControllerCmd::ListParams { prefix } => cmd_list_params(plugin, prefix),
        ControllerCmd::ListParamGroups => cmd_list_param_groups(plugin),
        ControllerCmd::SearchParams { query } => cmd_search_params(plugin, &query),
        ControllerCmd::GetParamsByName { names } => cmd_get_params_by_name(plugin, names),
        ControllerCmd::GetPatchState { diff_only } => cmd_get_patch_state(plugin, diff_only),
        ControllerCmd::GetParamInfo { id } => cmd_get_param_info(plugin, id),
        ControllerCmd::PreviewParams { ids, limit } => cmd_preview_params(plugin, ids, limit),
        ControllerCmd::SavePreset { path } => cmd_save_preset(plugin, path),
        ControllerCmd::LoadPreset { path } => cmd_load_preset(plugin, path),
    }
}

fn cmd_list_params(
    plugin: &mut PluginInstance,
    prefix: Option<String>,
) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let prefix_lower = prefix
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase());
    let mut parameters = Vec::new();
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if info.is_writable() && !info.is_hidden() {
                if let Some(ref pre) = prefix_lower {
                    if !info.title.to_lowercase().starts_with(pre) {
                        continue;
                    }
                }
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
    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_list_param_groups(plugin: &mut PluginInstance) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let mut group_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if info.is_writable() && !info.is_hidden() {
                let group = param_group_prefix(&info.title);
                *group_counts.entry(group).or_default() += 1;
            }
        }
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
    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_search_params(plugin: &mut PluginInstance, query: &str) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if info.is_writable() && !info.is_hidden()
                && info.title.to_lowercase().contains(&query_lower)
            {
                let value = plugin.get_parameter(info.id);
                let display = plugin
                    .get_parameter_display(info.id)
                    .unwrap_or_else(|_| format!("{value:.3}"));
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
        "query": query,
        "matches": matches,
        "count": matches.len()
    });
    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_get_params_by_name(
    plugin: &mut PluginInstance,
    names: Vec<String>,
) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let mut params = Vec::new();
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if info.is_writable() && !info.is_hidden() {
                let value = plugin.get_parameter(info.id);
                let display = plugin
                    .get_parameter_display(info.id)
                    .unwrap_or_else(|_| format!("{value:.3}"));
                params.push(serde_json::json!({
                    "id": info.id,
                    "name": info.title,
                    "value": value,
                    "display": display,
                }));
            }
        }
    }
    let mut results = Vec::new();
    for name in names {
        let (primary, aliases) = query_terms_for_scoring(&name);
        let mut scored: Vec<(u32, serde_json::Value)> = params
            .iter()
            .filter_map(|p| {
                let s = score_param(p, &primary, &aliases);
                if s > 0 {
                    Some((s, p.clone()))
                } else {
                    None
                }
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
                "match": serde_json::Value::Null,
                "score": 0
            }));
        }
    }
    let response = serde_json::json!({
        "results": results,
        "count": results.len()
    });
    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_get_patch_state(plugin: &mut PluginInstance, diff_only: bool) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let mut result = Vec::new();
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if !info.is_writable() || info.is_hidden() {
                continue;
            }
            let value = plugin.get_parameter(info.id);
            if diff_only && (value - info.default_normalized).abs() < 1e-4 {
                continue;
            }
            let display = plugin
                .get_parameter_display(info.id)
                .unwrap_or_else(|_| format!("{value:.3}"));
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
    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_get_param_info(plugin: &mut PluginInstance, id: u32) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let mut info_opt = None;
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if info.id == id {
                info_opt = Some(info);
                break;
            }
        }
    }
    let info = info_opt.ok_or_else(|| format!("Parameter id {} not found", id))?;

    let default_display = plugin
        .get_parameter_display_for_value(info.id, info.default_normalized)
        .map_err(|e| format!("Failed to get default display: {e}"))?;

    let probe_vals = [0.0, 0.25, 0.5, 0.75, 1.0];
    let mut range_probe = serde_json::Map::new();
    for v in probe_vals {
        let key = format!("{:.2}", v);
        let display = plugin
            .get_parameter_display_for_value(info.id, v)
            .unwrap_or_else(|_| format!("{v:.3}"));
        range_probe.insert(key, serde_json::Value::String(display));
    }

    let response = serde_json::json!({
        "id": info.id,
        "name": info.title,
        "units": info.units,
        "default_normalized": info.default_normalized,
        "default_display": default_display,
        "step_count": info.step_count,
        "is_writable": info.is_writable(),
        "is_bypass": info.is_bypass(),
        "range_probe": range_probe,
    });
    serde_json::to_string_pretty(&response)
        .map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_preview_params(
    plugin: &mut PluginInstance,
    ids: Option<Vec<u32>>,
    limit: usize,
) -> Result<String, String> {
    let count = plugin.get_parameter_count();
    let mut params = Vec::new();
    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            if info.is_writable() && !info.is_hidden() {
                let value = plugin.get_parameter(info.id);
                let display = plugin
                    .get_parameter_display(info.id)
                    .unwrap_or_else(|_| format!("{value:.3}"));
                params.push(serde_json::json!({
                    "id": info.id,
                    "name": info.title,
                    "value": value,
                    "display": display,
                }));
            }
        }
    }
    let selected: Vec<serde_json::Value> = if let Some(filter_ids) = ids {
        params
            .iter()
            .filter(|p| {
                p.get("id")
                    .and_then(|v| v.as_u64())
                    .map(|id| filter_ids.contains(&(id as u32)))
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
    serde_json::to_string_pretty(&response)
        .map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_save_preset(plugin: &mut PluginInstance, path: PathBuf) -> Result<String, String> {
    preset::state::save_plugin_state(plugin, &path)
        .map_err(|e| format!("Failed to save preset: {e}"))?;
    let response = serde_json::json!({
        "status": "saved",
        "path": path.to_string_lossy(),
        "timestamp_ms": crate::now_ms(),
    });
    serde_json::to_string_pretty(&response)
        .map_err(|e| format!("Serialization failed: {e}"))
}

fn cmd_load_preset(plugin: &mut PluginInstance, path: PathBuf) -> Result<String, String> {
    preset::state::restore_plugin_state(plugin, &path)
        .map_err(|e| format!("Failed to load preset: {e}"))?;
    let response = serde_json::json!({
        "status": "loaded",
        "path": path.to_string_lossy(),
        "timestamp_ms": crate::now_ms(),
    });
    serde_json::to_string_pretty(&response)
        .map_err(|e| format!("Serialization failed: {e}"))
}
