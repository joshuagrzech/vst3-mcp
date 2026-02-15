# VST3 MCP Host API Reference

Complete reference for all MCP tools exposed by the VST3 MCP Host.

## Protocol Information

- **Protocol**: Model Context Protocol (MCP)
- **Transport**: stdio (JSON-RPC over stdin/stdout)
- **Protocol Version**: 2024-11-05
- **Message Format**: JSON-RPC 2.0

## Tool Invocation

All tools are invoked using the MCP `tools/call` method:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "tool_name",
    "arguments": {
      "param1": "value1",
      "param2": "value2"
    }
  }
}
```

Responses follow MCP content array format:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"result\": \"data\"}"
      }
    ]
  }
}
```

Error responses include `isError: true`:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "Error message description"
      }
    ],
    "isError": true
  }
}
```

## Plugin Discovery & Loading

### scan_plugins

Scan for installed VST3 plugins and return a list with metadata.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `path` | string | No | System default | Custom directory to scan |

**Default Scan Paths:**

- **Linux**: `~/.vst3/`, `/usr/lib/vst3/`, `/usr/local/lib/vst3/`
- **macOS**: `~/Library/Audio/Plug-Ins/VST3/`, `/Library/Audio/Plug-Ins/VST3/`
- **Windows**: `%LOCALAPPDATA%\Programs\Common\VST3\`, `C:\Program Files\Common Files\VST3\`

**Response:**

```json
{
  "plugins": [
    {
      "uid": "C18D3C1E719E4E29924D3ECAA5E4DA18",
      "name": "AGain Sample Accurate",
      "vendor": "Steinberg Media Technologies",
      "category": "Fx",
      "version": "1.0.0",
      "path": "/usr/lib/vst3/AGain.vst3"
    },
    ...
  ],
  "count": 5
}
```

**Response Fields:**

- `uid`: 32-character hexadecimal class ID (unique identifier)
- `name`: Plugin display name
- `vendor`: Plugin manufacturer/developer
- `category`: VST3 category (Fx, Instrument, Analyzer, etc.)
- `version`: Plugin version string
- `path`: Absolute path to .vst3 bundle
- `count`: Total number of plugins found

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "scan_plugins",
    "arguments": {}
  }
}
```

**Custom Path Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "scan_plugins",
    "arguments": {
      "path": "/home/user/custom-plugins/vst3"
    }
  }
}
```

**Notes:**

- Scan is performed out-of-process for safety (plugin crashes don't affect host)
- Fast path: reads `moduleinfo.json` if present
- Fallback: loads plugin binary to query factory
- Discovered plugins cached until next scan

---

### load_plugin

Load a VST3 plugin by UID and prepare it for processing.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `uid` | string | Yes | — | 32-character hex UID from scan results |
| `sample_rate` | integer | No | 44100 | Sample rate in Hz |

**Response:**

```json
{
  "status": "loaded",
  "plugin": {
    "classId": "C18D3C1E719E4E29924D3ECAA5E4DA18",
    "name": "AGain Sample Accurate",
    "vendor": "Steinberg Media Technologies"
  }
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "load_plugin",
    "arguments": {
      "uid": "C18D3C1E719E4E29924D3ECAA5E4DA18",
      "sample_rate": 48000
    }
  }
}
```

**Lifecycle:**

The plugin transitions through these states:

1. **Created**: Plugin factory instantiated
2. **SetupDone**: Audio I/O and sample rate configured
3. **Active**: Plugin activated (internal state initialized)
4. **Processing**: Ready to process audio (setProcessing called if supported)

**Notes:**

- Only one plugin can be loaded at a time
- Loading a new plugin unloads the previous one
- Sample rate can be overridden during `process_audio` (auto-matched to input file)
- Some plugins don't implement `setProcessing` (optional per VST3 spec)

**Errors:**

- `"Plugin not found"`: UID doesn't match any scanned plugin
- `"Failed to initialize"`: Plugin rejected setup (check sample rate, channel config)
- `"Failed to activate"`: Plugin internal error during activation

---

### get_plugin_info

Get the loaded plugin's identity (classId, name, vendor).

**Parameters:** None

**Response:**

```json
{
  "classId": "C18D3C1E719E4E29924D3ECAA5E4DA18",
  "name": "AGain Sample Accurate",
  "vendor": "Steinberg Media Technologies"
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "get_plugin_info",
    "arguments": {}
  }
}
```

**Errors:**

- `"No plugin loaded"`: Must call `load_plugin` first

**Use Cases:**

- Verify correct plugin loaded before processing
- Display current plugin to user
- Log plugin identity for debugging

---

## Audio Processing

### process_audio

Process an audio file through the loaded VST3 plugin.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `input_file` | string | Yes | — | Path to input audio file |
| `output_file` | string | Yes | — | Path for output WAV file |

**Supported Input Formats:**

- WAV (PCM, float)
- FLAC (lossless)
- MP3 (lossy)
- OGG Vorbis (lossy)

**Output Format:** WAV (32-bit float, preserves input sample rate and channels)

**Response:**

```json
{
  "status": "completed",
  "input": {
    "path": "input.wav",
    "sample_rate": 44100,
    "channels": 2,
    "samples": 220500
  },
  "output": {
    "path": "output.wav",
    "sample_rate": 44100,
    "channels": 2,
    "samples": 264600
  },
  "tail_samples": 44100
}
```

**Response Fields:**

- `input.samples`: Total input samples processed
- `output.samples`: Total output samples (input + tail)
- `tail_samples`: Additional samples for plugin tail (reverb/delay decay)

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "process_audio",
    "arguments": {
      "input_file": "/home/user/vocal.wav",
      "output_file": "/home/user/vocal-processed.wav"
    }
  }
}
```

**Processing Details:**

1. Decode input file to f32 planar buffers
2. Convert to VST3 buffer format (AudioBusBuffers)
3. Apply queued parameter changes (from `set_param`/`batch_set`)
4. Process in 512-sample blocks
5. Append plugin tail (from `getTailSamples`)
6. Encode output as WAV

**Parameter Application:**

Any parameters set via `set_param` or `batch_set` are applied during this call. Parameter changes are **queued** and **consumed** here.

**Notes:**

- Input sample rate overrides `load_plugin` sample_rate if different
- Stereo files process with 2 channels, mono with 1 channel
- Plugin tail automatically calculated (reverb/delay decay)
- Output always WAV format (high quality, lossless)

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first
- `"File not found"`: Input file doesn't exist
- `"Unsupported format"`: Input format not recognized
- `"Processing failed"`: Plugin returned error during processing

---

## Parameter Control

### list_params

List all writable parameters with their current values.

**Parameters:** None

**Response:**

```json
{
  "parameters": [
    {
      "id": 0,
      "name": "Bypass",
      "value": 0.0,
      "display": "Off"
    },
    {
      "id": 1,
      "name": "Gain",
      "value": 0.5,
      "display": "0.0 dB"
    }
  ],
  "count": 2
}
```

**Response Fields:**

- `id`: Parameter ID (use in `get_param`, `set_param`, `batch_set`)
- `name`: Human-readable parameter name
- `value`: Normalized value [0.0, 1.0]
- `display`: Plugin's formatted string (e.g., "3.5 dB", "440 Hz", "50%")
- `count`: Total writable parameters

**Filtering:**

Automatically excludes:
- Read-only parameters (output meters, analyzers)
- Hidden parameters (internal state)
- Parameters with `kIsReadOnly` flag set
- Parameters with `kIsHidden` flag set

Only includes parameters with `kCanAutomate` flag (writable).

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "list_params",
    "arguments": {}
  }
}
```

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first

**Use Cases:**

- Discover available parameters before automation
- Show current plugin state to user
- Find parameter IDs for `set_param` calls

---

### get_param

Get a single parameter's current value and display string.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `id` | integer | Yes | — | Parameter ID from `list_params` |

**Response:**

```json
{
  "id": 1,
  "value": 0.75,
  "display": "+3.5 dB"
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "get_param",
    "arguments": {
      "id": 1
    }
  }
}
```

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first
- `"Invalid parameter ID"`: ID doesn't exist or is out of range

**Use Cases:**

- Verify parameter value after `set_param`
- Read current state before making adjustments
- Display current value to user

---

### set_param

Set a single parameter value. Changes are queued and applied in next `process_audio` call.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `id` | integer | Yes | — | Parameter ID from `list_params` |
| `value` | number | Yes | — | Normalized value [0.0, 1.0] |

**Response:**

```json
{
  "status": "queued",
  "id": 1,
  "value": 0.75,
  "display": "+3.5 dB"
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "set_param",
    "arguments": {
      "id": 1,
      "value": 0.75
    }
  }
}
```

**Value Range:**

- Must be between `0.0` (minimum) and `1.0` (maximum)
- Out-of-range values return error
- Plugin converts normalized value to internal representation

**Timing:**

Parameter changes are **queued** and applied during next `process_audio` call via `IParameterChanges`.

```
set_param(id=1, value=0.75)  →  Queued
process_audio(...)           →  Applied here
```

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first
- `"Value out of range"`: Value must be [0.0, 1.0]
- `"Invalid parameter ID"`: ID doesn't exist

**Use Cases:**

- Adjust individual parameter before processing
- Iterate on parameter values with user feedback
- Automate single parameter changes

---

### batch_set

Set multiple parameters atomically. All changes validated before any are applied.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `changes` | array | Yes | — | Array of `{id, value}` objects |

**Changes Object:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | integer | Yes | Parameter ID |
| `value` | number | Yes | Normalized value [0.0, 1.0] |

**Response:**

```json
{
  "status": "queued",
  "changes_queued": 3
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "batch_set",
    "arguments": {
      "changes": [
        {"id": 0, "value": 0.0},
        {"id": 1, "value": 0.8},
        {"id": 2, "value": 0.5}
      ]
    }
  }
}
```

**Atomic Validation:**

All values validated **before** any changes queued:

1. Check ALL values in range [0.0, 1.0]
2. If ANY value invalid → reject entire batch
3. If all valid → queue ALL changes

This prevents partial parameter updates.

**Timing:**

Like `set_param`, changes are queued and applied during next `process_audio` call.

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first
- `"Value out of range"`: At least one value invalid (reports which one)
- `"Invalid parameter ID"`: At least one ID doesn't exist

**Use Cases:**

- Apply multiple parameter changes together
- Restore plugin state from saved configuration
- Create parameter snapshots

---

## Preset Management

### save_preset

Save the current plugin state as a `.vstpreset` file.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `path` | string | Yes | — | Output path for .vstpreset file |

**Response:**

```json
{
  "status": "saved",
  "path": "/home/user/presets/my-preset.vstpreset",
  "size_bytes": 2048
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "save_preset",
    "arguments": {
      "path": "/home/user/presets/warm-vocal.vstpreset"
    }
  }
}
```

**Preset Contents:**

- VST3 header (magic bytes, version)
- Plugin class ID
- Component state (IComponent::getState)
- Controller state (IEditController::getState)

**Notes:**

- Captures all parameter values
- Includes internal plugin state (buffers, filters, etc.)
- File format is binary (not human-readable)
- Compatible with DAWs and other VST3 hosts

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first
- `"Failed to save"`: File write error (check permissions, disk space)

---

### load_preset

Load a `.vstpreset` file into the currently loaded plugin.

**Parameters:**

| Name | Type | Required | Default | Description |
|------|------|----------|---------|-------------|
| `path` | string | Yes | — | Path to .vstpreset file |

**Response:**

```json
{
  "status": "loaded",
  "path": "/home/user/presets/warm-vocal.vstpreset",
  "class_id": "C18D3C1E719E4E29924D3ECAA5E4DA18"
}
```

**Example:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "load_preset",
    "arguments": {
      "path": "/home/user/presets/warm-vocal.vstpreset"
    }
  }
}
```

**State Restoration:**

1. Parse .vstpreset file
2. Verify class ID matches loaded plugin
3. Call `IComponent::setState` (DSP state)
4. Call `IEditController::setState` (UI state)
5. All parameters restored to saved values

**Notes:**

- Preset must match loaded plugin's class ID
- Restores both component and controller state
- Previous parameter changes discarded
- Plugin version mismatch may cause issues

**Errors:**

- `"No plugin loaded"`: Call `load_plugin` first
- `"File not found"`: Preset file doesn't exist
- `"Class ID mismatch"`: Preset is for different plugin
- `"Failed to load"`: Corrupted preset or plugin rejected state

---

## Error Handling

All tools return errors in MCP content array format with `isError: true`:

```json
{
  "result": {
    "content": [
      {
        "type": "text",
        "text": "No plugin loaded. Call load_plugin first."
      }
    ],
    "isError": true
  }
}
```

### Common Error Messages

| Error | Cause | Solution |
|-------|-------|----------|
| `"No plugin loaded"` | Tool requires loaded plugin | Call `load_plugin` first |
| `"Plugin not found"` | Invalid UID in `load_plugin` | Use UID from `scan_plugins` |
| `"File not found"` | Input file doesn't exist | Check path, use absolute paths |
| `"Value out of range"` | Parameter value not [0.0, 1.0] | Clamp to valid range |
| `"Invalid parameter ID"` | Parameter ID doesn't exist | Use ID from `list_params` |
| `"Class ID mismatch"` | Preset for different plugin | Load correct plugin first |

### Debug Logging

Enable detailed logging with `RUST_LOG` environment variable:

```bash
RUST_LOG=debug ./vst3-mcp-host 2>debug.log
```

Log levels:
- `error`: Critical errors only
- `warn`: Warnings and errors
- `info`: Normal operation messages
- `debug`: Detailed execution trace
- `trace`: Maximum verbosity

Logs output to **stderr** (stdout reserved for MCP protocol).

---

## Tool Dependencies

Some tools require others to be called first:

```
scan_plugins (optional)
  ↓
load_plugin (required for all below)
  ↓
  ├→ get_plugin_info
  ├→ list_params
  ├→ get_param
  ├→ set_param
  ├→ batch_set
  ├→ process_audio (applies queued parameter changes)
  ├→ save_preset
  └→ load_preset
```

**Typical Workflow:**

1. `scan_plugins` - Discover available plugins
2. `load_plugin` - Load specific plugin by UID
3. `list_params` - See available parameters
4. `set_param` / `batch_set` - Adjust parameters (queued)
5. `process_audio` - Process file (applies parameters)
6. Optional: `save_preset` - Save configuration

---

## See Also

- [USAGE.md](USAGE.md) - Detailed usage guide and workflows
- [EXAMPLES.md](EXAMPLES.md) - Practical recipes and examples
- [../README.md](../README.md) - Project overview
