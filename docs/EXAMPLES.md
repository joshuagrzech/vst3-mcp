# Realtime Wrapper Examples

These examples assume you are using the **embedded MCP endpoint from `AgentAudio Wrapper`** in a DAW.

## 1) Bootstrap a wrapper instance

```json
{
  "method": "tools/call",
  "params": {
    "name": "wrapper_status",
    "arguments": {}
  }
}
```

Use response fields:
- `endpoint`
- `instance_id`
- `mcp_name`

## 2) Scan and load child plugin

```json
{
  "method": "tools/call",
  "params": {
    "name": "scan_plugins",
    "arguments": {}
  }
}
```

Then:

```json
{
  "method": "tools/call",
  "params": {
    "name": "load_child_plugin",
    "arguments": {
      "uid": "YOUR_PLUGIN_UID"
    }
  }
}
```

Optionally manage child editor:
- `open_child_editor`
- `close_child_editor`

## 3) List parameters and map IDs

```json
{
  "method": "tools/call",
  "params": {
    "name": "list_params",
    "arguments": {}
  }
}
```

Pick IDs for meaningful controls (e.g. cutoff, resonance, mix).

## 4) Single realtime update

```json
{
  "method": "tools/call",
  "params": {
    "name": "set_param_realtime",
    "arguments": {
      "id": 42,
      "value": 0.73
    }
  }
}
```

## 5) Batch realtime updates

```json
{
  "method": "tools/call",
  "params": {
    "name": "batch_set_realtime",
    "arguments": {
      "changes": [
        { "id": 42, "value": 0.70 },
        { "id": 43, "value": 0.35 },
        { "id": 44, "value": 0.80 }
      ]
    }
  }
}
```

Check response:
- `accepted`
- `dropped`
- `queue_len`

## 6) Continuous control loop example

Prompt style for an LLM:

1. Load plugin and list params once.
2. Keep a local map: semantic name -> param ID.
3. Send `set_param_realtime` every control tick (10-30 Hz is a good start).
4. Use `batch_set_realtime` for coordinated moves.
5. If queue drops appear, reduce update rate.

## 7) Multi-instance safety example

When multiple wrapper instances exist:

1. Query `wrapper_status` on each endpoint.
2. Verify `instance_id` and plugin name before writing.
3. Route updates only to the intended endpoint.

## 8) Unload/reset child plugin

```json
{
  "method": "tools/call",
  "params": {
    "name": "unload_child_plugin",
    "arguments": {}
  }
}
```

This clears current child state and parameter queue for that wrapper instance.

## Notes

- Realtime control in this flow does not require `.vstpreset`.
- Values must stay in normalized range `[0.0, 1.0]`.
- Wrapper currently uses queue-based event application on audio callback boundaries.
