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

## Optional: Global MCP Router (stable endpoint)

Each wrapper instance binds to an **ephemeral port** (`127.0.0.1:0`), so the MCP endpoint changes every time you insert the plugin.

If you want to configure **one** MCP server in your coding agent (Claude/Gemini/Cursor) and have it automatically route to whichever wrapper instances are running, use the router daemon:

- Router daemon: `agentaudio-mcp-routerd` (Streamable HTTP MCP at `/mcp`)
- Cursor shim: `agentaudio-mcp-stdio` (stdio MCP → forwards to router daemon)
- Installer: `agentaudio-mcp` (add/remove the MCP server entry in common configs)

### Start the router daemon

```bash
cargo run -p agentaudio-mcp-router --bin agentaudio-mcp-routerd
```

The router listens on `http://127.0.0.1:38765` by default. MCP endpoint is:

`http://127.0.0.1:38765/mcp`

### Wrapper auto-registration

When the wrapper starts its embedded MCP server, it will (best-effort) register itself with the router daemon:
- `POST /register` once at startup
- `POST /heartbeat` every ~3s
- `POST /unregister` on teardown

Override router base URL via:
- `AGENTAUDIO_MCP_ROUTERD=http://127.0.0.1:38765`

### Install into MCP clients (add + remove)

Install (idempotent):

```bash
cargo run --bin agentaudio-mcp -- install
```

Remove (idempotent):

```bash
cargo run --bin agentaudio-mcp -- uninstall
```

Status:

```bash
cargo run --bin agentaudio-mcp -- status
```

On Linux, this patches (with backups + atomic writes):
- Claude Code: `~/.claude.json` (adds `mcpServers.agentaudio-router` pointing at router HTTP `/mcp`)
- Gemini CLI: `~/.gemini/settings.json` (adds `mcpServers.agentaudio-router.httpUrl`)
- Cursor: `~/.config/cursor/mcp.json` (adds `mcpServers.agentaudio-router` as a stdio server using `agentaudio-mcp-stdio`)

### systemd user service example

Create `~/.config/systemd/user/agentaudio-mcp-routerd.service`:

```ini
[Unit]
Description=AgentAudio MCP Router Daemon
After=network.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/agentaudio-mcp-routerd
Restart=on-failure
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
```

Then:

```bash
systemctl --user daemon-reload
systemctl --user enable --now agentaudio-mcp-routerd
systemctl --user status agentaudio-mcp-routerd
```

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

## Master release installer bundle (precompiled artifacts)

For shipping a ready-to-run installer package (no compile step on install machine), run:

```bash
./scripts/master-release-installer.sh
```

This produces a release directory + tarball under `./dist/` containing:
- `agentaudio-installer` (GUI installer)
- `run-installer.sh` launcher
- `precompiled-target/release/` with all required artifacts:
  - `libagentaudio_wrapper_vst3.so`
  - `agent-audio-scanner`
  - `agentaudio-mcp-routerd`
  - `agentaudio-mcp-stdio`
  - `agentaudio-mcp`

When launched from that package, the installer auto-detects `precompiled-target` and defaults to "Skip build", so installation focuses on file placement, service setup, and MCP client config patching.

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

Use this hard routing rule in your client/system prompt:

> If user mentions VST/plugin/preset/patch/sound/tone/parameter/knob/automation, use Audio MCP tools first. Do not use web search unless user explicitly asks for docs/news. In audio context, patch = preset/sound configuration, not code diff.

If your orchestrator supports it, call `guard_audio_routing` before any web search call and follow `block_web_search` + `recommended_first_tool`.

Use this sequence:

1. `wrapper_status`
2. `scan_plugins`
3. `load_plugin` (alias of `load_child_plugin`)
4. `find_vst_parameter` (alias for natural-language parameter search)
5. `preview_vst_parameter_values`
6. Repeated `set_param_realtime` / `batch_set_realtime` (or `edit_vst_patch`)
7. Optional: `open_child_editor` / `close_child_editor`

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
