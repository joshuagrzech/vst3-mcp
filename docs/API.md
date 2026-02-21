# API Reference

This project currently exposes two MCP surfaces:

1. **Wrapper realtime MCP** (recommended): embedded in `AgentAudio Wrapper` plugin instance.
2. **Legacy offline host MCP**: root `vst3-mcp-host` binary.

## Wrapper Realtime MCP (Primary)

Transport is HTTP MCP endpoint per wrapper instance, typically:

`http://127.0.0.1:<dynamic_port>/mcp`

### `wrapper_status`

Returns instance metadata and current load state.

Important fields:
- `instance_id`
- `mcp_name`
- `endpoint`
- `queue_len`
- `status` (`child_loaded` or `no_child_loaded`)

### `scan_plugins`

Scans available child VST3 plugins.

Arguments:
- `path` (optional string)

### `load_child_plugin`

Loads selected child plugin into wrapper instance.

Arguments:
- `uid` (string, 32-char hex)

### `unload_child_plugin`

Unloads current child plugin and clears runtime state.

Arguments: none

### `open_child_editor`

Opens child plugin editor window.

Arguments: none

### `close_child_editor`

Closes child plugin editor window.

Arguments: none

### `list_params`

Lists writable/non-hidden parameters for loaded child plugin.

Arguments: none

### `set_param_realtime`

Enqueues one realtime parameter event.

Arguments:
- `id` (u32)
- `value` (f64 in `[0.0, 1.0]`)

Response includes:
- `status` (`queued` or `dropped_queue_full`)
- `queue_len`
- `timestamp_ms`
- `instance_id`

### `batch_set_realtime`

Enqueues multiple realtime parameter events.

Arguments:
- `changes`: array of `{ id, value }`

Response includes:
- `accepted`
- `dropped`
- `queue_len`
- `timestamp_ms`
- `instance_id`

### `get_param_info`

Returns metadata and a 5-point display range probe for a single parameter. Useful before calling `set_param_realtime` to understand what normalized values map to human-readable labels.

Arguments:
- `id` (u32, required)

Response includes:
- `id`, `name`, `units`
- `default_normalized`, `default_display`
- `step_count`, `is_writable`, `is_bypass`
- `range_probe`: display strings at `{ "0.00", "0.25", "0.50", "0.75", "1.00" }`

### `save_preset`

Saves current plugin state to a `.vstpreset` file. Supports `~/` path expansion.

Arguments:
- `path` (string, required) — e.g. `~/Presets/MyPatch.vstpreset`

Response includes:
- `status` (`saved`)
- `path`
- `timestamp_ms`

### `load_preset`

Restores plugin state from a `.vstpreset` file. Call `load_child_plugin` first to ensure the plugin is loaded.

Arguments:
- `path` (string, required) — e.g. `~/Presets/MyPatch.vstpreset`

Response includes:
- `status` (`loaded`)
- `path`
- `timestamp_ms`

### `set_param_by_name`

Sets a parameter by human-readable name instead of numeric id. Uses case-insensitive exact match, falling back to substring match.

Arguments:
- `name` (string, required) — e.g. `"Filter Cutoff"` or `"cutoff"`
- `value` (f64, required, in `[0.0, 1.0]`)

Response includes:
- `status` (`queued` or `dropped_queue_full`)
- `id` (resolved numeric id)
- `name` (matched parameter name)
- `value`, `immediate_applied`, `queue_len`, `timestamp_ms`, `instance_id`

### High-level alias tools (router + wrapper direct)

These are natural-language aliases to improve tool selection from prompts:

- `load_plugin` (alias of `load_child_plugin`)
- `edit_vst_patch` (alias of `batch_set_realtime`)
- `find_vst_parameter` (NL search over `list_params`)
- `preview_vst_parameter_values` (inspect current values before editing)

### `guard_audio_routing` (router + stdio shim)

Deterministic intent guardrail for orchestrators before WebSearch:

- Input: `user_message`, optional `requested_tool`
- Output: `audio_intent_confidence`, `block_web_search`, `recommended_first_tool`, etc.

Rule:

- If audio intent confidence is above threshold and the user did **not** explicitly ask for docs/news, block web search and route to Audio MCP tools first.
- In audio context, “patch” means preset/sound configuration (not code diff).

## Legacy Offline Host MCP (Secondary)

Available via `vst3-mcp-host` stdio transport:

- `scan_plugins`
- `load_plugin`
- `get_plugin_info`
- `list_params`
- `get_param`
- `set_param`
- `batch_set`
- `process_audio`
- `save_preset`
- `load_preset`
- `open_editor`
- `close_editor`

Use this path for offline file rendering workflows, not live DAW automation.

## Error Semantics

Common errors:
- no plugin loaded
- invalid UID
- invalid parameter ID
- normalized value out of range
- queue full (realtime path)
- unsupported child routing

## See Also

- `USAGE.md` for full wrapper-first setup
- `EXAMPLES.md` for realtime recipe-style usage
