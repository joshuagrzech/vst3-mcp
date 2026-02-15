# VST3 MCP Host

A headless Rust VST3 host that AI agents control through the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/). Scan, load, and process audio through VST3 plugins — all via tool calls from Claude or any MCP client.

## Features

- **Plugin scanning** — discovers installed VST3 plugins with metadata (name, vendor, UID, category)
- **Plugin loading** — full VST3 lifecycle management (Created → SetupDone → Active → Processing)
- **Audio processing** — offline block-based rendering through any loaded VST3 plugin
- **Multi-format input** — WAV, FLAC, MP3, OGG (via symphonia)
- **Preset management** — save/load `.vstpreset` files with correct component+controller state sync
- **Parameter control** — enumerate, read, and write plugin parameters with AI-accessible tools
- **MCP integration** — 10 tools exposed over stdio JSON-RPC

## Requirements

- **Rust** 1.85+ (edition 2024)
- **VST3 plugins** installed on your system
- An **MCP client** (Claude Code, Claude Desktop, or any MCP-compatible client)

## Building

```bash
git clone <repo-url>
cd vst3-mcp-host
cargo build --release
```

The binary is at `target/release/vst3-mcp-host`.

## Running

### As an MCP server (recommended)

The host communicates over stdio using JSON-RPC, so you configure it in your MCP client rather than running it directly.

**Claude Code** — add to `.claude/settings.json` (project) or `~/.claude/settings.json` (global):

```json
{
  "mcpServers": {
    "vst3": {
      "command": "/path/to/vst3-mcp-host"
    }
  }
}
```

**Claude Desktop** — add to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "vst3": {
      "command": "/path/to/vst3-mcp-host"
    }
  }
}
```

### Quick test

Verify the server starts and responds:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}' \
  | cargo run 2>/dev/null
```

You should get a JSON response listing server capabilities and available tools.

### Debug logging

Logs go to stderr (stdout is reserved for MCP protocol):

```bash
RUST_LOG=debug cargo run 2>vst3-mcp.log
```

## MCP Tools

### `scan_plugins`

Discover installed VST3 plugins on your system.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | no | Custom directory to scan instead of default OS locations |

Default scan paths:
- **Linux**: `~/.vst3/`, `/usr/lib/vst3/`, `/usr/local/lib/vst3/`
- **macOS**: `~/Library/Audio/Plug-Ins/VST3/`, `/Library/Audio/Plug-Ins/VST3/`
- **Windows**: `%LOCALAPPDATA%/Programs/Common/VST3/`, `C:/Program Files/Common Files/VST3/`

Returns a JSON array with each plugin's `uid`, `name`, `vendor`, `category`, `version`, and `path`.

### `load_plugin`

Load a VST3 plugin by UID and prepare it for processing.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `uid` | string | yes | 32-character hex UID from scan results |
| `sample_rate` | integer | no | Sample rate in Hz (default: 44100, auto-matched to input file during processing) |

Only one plugin can be loaded at a time. Loading a new plugin replaces the current one.

### `process_audio`

Process an audio file through the loaded plugin and write the output as WAV.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `input_file` | string | yes | Path to input audio file (WAV, FLAC, MP3, OGG) |
| `output_file` | string | yes | Path for the output WAV file |

The output preserves the input's sample rate and channel count. Plugin tail (reverb/delay fade-out) is automatically appended.

### `save_preset`

Save the current plugin state to a `.vstpreset` file.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Output path for the .vstpreset file |

### `load_preset`

Load a `.vstpreset` file into the currently loaded plugin.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | Path to the .vstpreset file |

### `get_plugin_info`

Get the loaded plugin's identity (classId, name, vendor).

No parameters required. Returns JSON with plugin metadata.

### `list_params`

List all writable parameters with their current values.

No parameters required. Returns an array of parameters with `id`, `name`, `value` (normalized 0-1), and `display` (human-readable string like "3.5 dB").

Filters out read-only and hidden parameters automatically.

### `get_param`

Get a single parameter's current value and display string.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | integer | yes | Parameter ID from list_params |

Returns the parameter's normalized value [0.0, 1.0] and display string.

### `set_param`

Set a single parameter value. Changes are queued and applied in the next `process_audio` call.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | integer | yes | Parameter ID from list_params |
| `value` | number | yes | Normalized value in range [0.0, 1.0] |

Returns confirmation with the new display string.

### `batch_set`

Set multiple parameters atomically. All changes are validated before any are applied.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `changes` | array | yes | Array of objects with `id` and `value` fields |

Changes are queued and applied together in the next `process_audio` call.

## Example Conversation

Once configured as an MCP server, you can interact with it naturally:

> **You:** What VST3 plugins do I have installed?
>
> *Claude calls `scan_plugins` and lists your plugins*
>
> **You:** Load the reverb plugin and process my vocal take
>
> *Claude calls `load_plugin` with the reverb's UID, then `process_audio` with your file*
>
> **You:** Save that as a preset so we can reuse it
>
> *Claude calls `save_preset` to save the current state*

## Architecture

```
src/
├── main.rs              # Entry point, starts MCP server over stdio
├── server.rs            # MCP tool definitions (AudioHost)
├── lib.rs               # Library root
├── hosting/
│   ├── module.rs        # VstModule: dlopen .vst3 bundles
│   ├── scanner.rs       # Plugin discovery (moduleinfo.json fast path + binary fallback)
│   ├── plugin.rs        # PluginInstance: lifecycle state machine, COM RAII
│   ├── host_app.rs      # IHostApplication + IComponentHandler implementations
│   └── types.rs         # Shared types (PluginInfo, PluginState, HostError, etc.)
├── audio/
│   ├── decode.rs        # Multi-format decoding to f32 (symphonia)
│   ├── encode.rs        # WAV encoding from f32 (hound)
│   ├── buffers.rs       # Interleaved ↔ planar conversion
│   └── process.rs       # Block-based offline rendering with tail handling
└── preset/
    ├── vstpreset.rs     # .vstpreset binary format read/write
    └── state.rs         # Plugin state save/restore bridge
```

## Tests

```bash
cargo test
```

Runs unit tests for buffer conversion, scanner path detection, preset round-trips, and more.

## License

TBD
