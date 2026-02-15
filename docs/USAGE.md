# AgentAudio Realtime Usage Guide

This guide focuses on the **DAW wrapper + embedded MCP** realtime workflow.

For offline file processing (legacy host), see `../README.md`.

## Realtime Architecture

1. Insert `AgentAudio Wrapper` as a VST3 plugin in your DAW.
2. Wrapper starts an embedded MCP server for that instance.
3. Load a child VST3 in the wrapper UI (scan + select + load).
4. Wrapper opens the child editor.
5. LLM sends realtime parameter events through MCP.
6. Audio callback applies queued events while the plugin is running.

## Build and Test

```bash
cargo build --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
cargo test --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
```

For DAW use, build in release mode:

```bash
cargo build --release --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
```

## Bundle as VST3

`cargo build` produces a shared library (e.g. `target/release/libagentaudio_wrapper_vst3.so`). DAWs expect a **VST3 bundle**: a directory named `AgentAudio Wrapper.vst3` with a specific layout.

**Linux bundle layout** (per VST3 spec):

- `AgentAudio Wrapper.vst3/Contents/x86_64-linux/AgentAudio Wrapper.so` — the plugin binary (same name as the bundle)
- `AgentAudio Wrapper.vst3/Contents/Resources/` — optional (e.g. `moduleinfo.json`)

**Steps:**

1. Build (release recommended):
   ```bash
   cargo build --release --manifest-path crates/agentaudio-wrapper-vst3/Cargo.toml
   ```

2. Create the bundle and copy the binary (from repo root):
   ```bash
   BUNDLE="AgentAudio Wrapper.vst3"
   ARCH="x86_64-linux"
   SO="crates/agentaudio-wrapper-vst3/target/release/libagentaudio_wrapper_vst3.so"
   mkdir -p "$BUNDLE/Contents/$ARCH"
   cp "$SO" "$BUNDLE/Contents/$ARCH/AgentAudio Wrapper.so"
   ```

3. Install so your DAW can see it:
   - **User:** `cp -r "AgentAudio Wrapper.vst3" ~/.vst3/`
   - **System:** `sudo cp -r "AgentAudio Wrapper.vst3" /usr/lib/vst3/`

A script `scripts/bundle-vst3.sh` is provided to do steps 2–3 in one go (see script for usage).

**Single pipeline (build + bundle + optional install):**

```bash
./scripts/build-and-install-vst3.sh                    # build release + bundle only
./scripts/build-and-install-vst3.sh release ~/.vst3   # build + bundle + install to ~/.vst3
```

## DAW Setup Steps

1. Build wrapper crate and bundle as above.
2. Install the bundle into your DAW plugin path (e.g. `~/.vst3/`).
3. Insert `AgentAudio Wrapper` on:
   - audio track for effect workflows, or
   - instrument track for synth workflows.
4. Open wrapper UI and click **Scan Plugins**.
5. Select child plugin and click **Load Child**.
6. Confirm child editor opens.
7. Copy endpoint/instance ID from wrapper UI (or `wrapper_status`).

## MCP Connection Pattern

Each wrapper instance runs its own MCP endpoint:

- endpoint: `http://127.0.0.1:<dynamic_port>/mcp`
- instance identity: unique `instance_id`
- logical name: `AgentAudio - <ChildPluginName>` after load

## Realtime Parameter Workflow

1. Call `list_params` once child plugin is loaded.
2. Map semantic names to IDs.
3. Send `set_param_realtime` or `batch_set_realtime`.
4. Events enqueue immediately and apply on the audio thread.
5. Iterate while audio is running.

### Event semantics

- Values must be normalized `[0.0, 1.0]`.
- Queue is bounded; overload can return drop status.
- Updates are instance-scoped (no cross-instance leakage).

## Recommended Prompting Pattern for LLMs

Use this sequence:

1. `wrapper_status`
2. `scan_plugins`
3. `load_child_plugin`
4. `list_params`
5. Repeated `set_param_realtime` / `batch_set_realtime`
6. Optional: `open_child_editor` / `close_child_editor`

## Troubleshooting

### No endpoint shown

- Reopen wrapper UI.
- Reinsert wrapper plugin instance.
- Check DAW/plugin logs for failed thread or port bind.

### Child plugin fails to load

- Confirm UID from `scan_plugins`.
- Verify child plugin supports:
  - effect-like routing (audio in + audio out), or
  - instrument-like routing (event in + audio out).

### Realtime updates not audible

- Ensure child plugin is loaded in this instance.
- Verify parameter IDs from `list_params`.
- Check queue status from tool response (`queued` vs `dropped_queue_full`).

### Multiple instances conflict

- Use `wrapper_status` for each endpoint.
- Make sure client targets the correct endpoint/instance ID.

## Important Note About Presets

Realtime control does **not** depend on `.vstpreset` files in this wrapper workflow.
If your primary goal is live LLM automation, drive parameters directly through realtime MCP tools.

## See Also

- `API.md` for tool reference
- `EXAMPLES.md` for practical command sequences
- `../crates/agentaudio-wrapper-vst3/README.md` for crate-level details
