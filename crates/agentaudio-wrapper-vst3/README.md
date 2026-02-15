# AgentAudio Wrapper VST3

This crate contains the DAW-loadable `AgentAudio Wrapper` VST3 plugin.

## What it does

- Hosts a child VST3 plugin (effect or instrument-style routing).
- Runs an embedded MCP server per wrapper instance.
- Exposes realtime MCP tools for parameter enqueue:
  - `set_param_realtime`
  - `batch_set_realtime`
- Applies parameter events from a lock-free queue on the audio callback.
- Provides a wrapper editor UI for:
  - scanning plugins,
  - selecting/loading/unloading child plugin,
  - opening/closing the child editor,
  - viewing MCP endpoint + instance ID.

## Wrapper MCP tools

- `wrapper_status`
- `scan_plugins`
- `load_child_plugin`
- `unload_child_plugin`
- `open_child_editor`
- `close_child_editor`
- `list_params`
- `set_param_realtime`
- `batch_set_realtime`

Typical realtime endpoint format per wrapper instance:

`http://127.0.0.1:<dynamic_port>/mcp`

## Build

```bash
cargo build --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
```

## Test

```bash
cargo test --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
```

## Manual DAW validation checklist

1. Build the plugin and install/bundle it into your VST3 path.
2. Insert `AgentAudio Wrapper` on an audio track (effect case).
3. Open wrapper editor, scan, and load a child effect plugin.
4. Verify child editor opens and audio passes through.
5. Insert wrapper on an instrument track (instrument case).
6. Load a child synth and verify MIDI-driven audio output.
7. Use MCP `wrapper_status` to get endpoint and instance metadata.
8. Send repeated `set_param_realtime`/`batch_set_realtime` calls and confirm audible realtime changes.
9. Load multiple wrapper instances and verify instance IDs/endpoints are unique and updates do not cross instances.
