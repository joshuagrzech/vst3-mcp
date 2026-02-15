# VST3 MCP Host Usage Guide

Complete guide to using the VST3 MCP Host for AI-driven audio processing and parameter control.

## Table of Contents

- [Installation](#installation)
- [Configuration](#configuration)
- [Basic Workflows](#basic-workflows)
- [Parameter Control](#parameter-control)
- [Preset Management](#preset-management)
- [Troubleshooting](#troubleshooting)

## Installation

### Building from Source

```bash
# Clone the repository
git clone <repository-url>
cd vst3-mcp-host

# Build release binary
cargo build --release

# Binary location
ls target/release/vst3-mcp-host
```

### System Requirements

- **Rust**: 1.85 or newer (edition 2024)
- **Operating System**: Linux, macOS, or Windows
- **VST3 Plugins**: At least one VST3 plugin installed

### VST3 Plugin Directories

The host scans these default locations:

**Linux:**
- `~/.vst3/`
- `/usr/lib/vst3/`
- `/usr/local/lib/vst3/`

**macOS:**
- `~/Library/Audio/Plug-Ins/VST3/`
- `/Library/Audio/Plug-Ins/VST3/`

**Windows:**
- `%LOCALAPPDATA%\Programs\Common\VST3\`
- `C:\Program Files\Common Files\VST3\`

## Configuration

### MCP Client Setup

The host communicates via stdio using the MCP protocol. Configure it in your MCP client:

#### Claude Code

Add to `.claude/settings.json` (project-level) or `~/.claude/settings.json` (global):

```json
{
  "mcpServers": {
    "vst3": {
      "command": "/absolute/path/to/vst3-mcp-host",
      "env": {
        "RUST_LOG": "info"
      }
    }
  }
}
```

#### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "vst3": {
      "command": "/absolute/path/to/vst3-mcp-host",
      "env": {
        "RUST_LOG": "info"
      }
    }
  }
}
```

### Environment Variables

- `RUST_LOG`: Set log level (`error`, `warn`, `info`, `debug`, `trace`)
  - Logs output to stderr (stdout reserved for MCP protocol)
- `VST3_PLUGIN_PATH`: Custom plugin search path (overrides defaults)

### Testing Connection

Verify the server starts correctly:

```bash
# Send MCP initialize request
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}' \
  | ./target/release/vst3-mcp-host 2>/dev/null
```

Expected response: JSON with server capabilities and available tools.

## Basic Workflows

### Workflow 1: Discover and Load a Plugin

```
User: "What VST3 plugins do I have?"

Claude will:
1. Call scan_plugins tool
2. Parse and present the list with names, vendors, UIDs
3. Ask which plugin to use
```

**Expected Output:**
```
Found 5 VST3 plugins:
- AGain Sample Accurate (Steinberg) - UID: C18D3C1E719E4E29924D3ECAA5E4DA18
- Vital (Matt Tytel) - UID: 76737476...
- ...
```

**Next Step:**
```
User: "Load the AGain plugin"

Claude will:
1. Extract UID from scan results
2. Call load_plugin with {"uid": "C18D3C1E719E4E29924D3ECAA5E4DA18"}
3. Confirm plugin loaded successfully
```

### Workflow 2: Process Audio Through Plugin

```
User: "Process input.wav through the loaded plugin and save as output.wav"

Claude will:
1. Verify plugin is loaded (call get_plugin_info)
2. Call process_audio with:
   {
     "input_file": "input.wav",
     "output_file": "output.wav"
   }
3. Report processing complete
```

**Processing Details:**
- Input formats: WAV, FLAC, MP3, OGG
- Output format: WAV (preserves sample rate and channels)
- Plugin tail (reverb/delay decay) automatically handled
- Sample rate auto-matched to input file

### Workflow 3: Adjust Parameters

```
User: "Set the gain parameter to 75%"

Claude will:
1. Call list_params to enumerate parameters
2. Find parameter with "gain" in name
3. Call set_param with {"id": <param_id>, "value": 0.75}
4. Confirm parameter set (shows new display value)
```

**Parameter Value Range:**
- All parameter values are normalized: `0.0` (minimum) to `1.0` (maximum)
- Plugin provides human-readable display strings (e.g., "3.5 dB", "100 Hz")

### Workflow 4: Batch Parameter Changes

```
User: "Set gain to 80%, mix to 50%, and output to 90%"

Claude will:
1. Call list_params to get parameter IDs
2. Map names to IDs (gain, mix, output)
3. Call batch_set with:
   {
     "changes": [
       {"id": 0, "value": 0.8},
       {"id": 1, "value": 0.5},
       {"id": 2, "value": 0.9}
     ]
   }
4. All parameters updated atomically
```

**Atomic Validation:**
- All values validated BEFORE any changes applied
- If any value is out of range [0.0, 1.0], entire batch fails
- Prevents partial parameter updates

### Workflow 5: Save and Load Presets

```
User: "Save the current settings as warm-vocal.vstpreset"

Claude will:
1. Call save_preset with {"path": "warm-vocal.vstpreset"}
2. Confirm preset saved

User: "Load that preset back"

Claude will:
1. Call load_preset with {"path": "warm-vocal.vstpreset"}
2. Confirm preset loaded
3. All parameters restored to saved state
```

## Parameter Control

### Understanding Parameters

VST3 plugins expose parameters as normalized values [0.0, 1.0] with metadata:

- **ID**: Unique integer identifier (0, 1, 2, ...)
- **Name**: Human-readable label ("Gain", "Frequency", "Mix")
- **Value**: Normalized 0.0-1.0 (0% to 100%)
- **Display**: Plugin's formatted string ("3.5 dB", "440 Hz", "50%")
- **Flags**: Writable, read-only, hidden, automatable

### Parameter Filtering

The host automatically filters parameters:

**Included in `list_params`:**
- Writable (kCanAutomate flag set)
- Not read-only (kIsReadOnly flag clear)
- Not hidden (kIsHidden flag clear)

**Excluded:**
- Read-only parameters (e.g., output meters)
- Hidden parameters (internal state)
- Bypass parameters (some plugins)

### Parameter Change Timing

**Important:** Parameter changes are **queued** and applied during the next `process_audio` call.

```
1. set_param(id=0, value=0.75)   → Queued
2. set_param(id=1, value=0.5)    → Queued
3. process_audio(...)            → Both changes applied here
4. get_param(id=0)               → Returns 0.75
```

**Why queued?**
- VST3 parameters use `IParameterChanges` during audio processing
- Changes must be sample-accurate (applied at buffer boundaries)
- Offline mode doesn't run continuous processing loop

**Workaround for immediate feedback:**
After setting parameters, call `get_param` to verify (reads from plugin state, not audio output).

### Parameter Discovery Strategy

```
User: "Adjust the compressor to be more aggressive"

Claude should:
1. Call list_params to see available parameters
2. Identify relevant params: "Threshold", "Ratio", "Attack", "Release"
3. Adjust parameters based on music production knowledge:
   - Lower threshold (e.g., 0.3 → 0.2)
   - Higher ratio (e.g., 0.4 → 0.6)
   - Faster attack (e.g., 0.5 → 0.3)
4. Process audio to hear result
5. Iterate if needed
```

## Preset Management

### Preset File Format

VST3 presets use the `.vstpreset` binary format:

- Header: VST3 magic bytes + version
- Chunk: Plugin class ID
- Component state (IComponent)
- Controller state (IEditController)
- Optional metadata

### Preset Compatibility

**Same Plugin Required:**
- Presets only work with the plugin that created them
- Class ID must match
- Plugin version may affect compatibility

**State Sync:**
- Component state: DSP parameters (filter coefficients, delay lines)
- Controller state: UI parameters (knob positions, switches)
- Both must be saved and loaded for full restoration

### Preset Workflows

#### Save Current State
```
User: "Save this as my-settings.vstpreset"

Result: File contains:
- All parameter values
- Plugin state (buffers, internal state)
- Class ID for validation
```

#### Load Saved State
```
User: "Load my-settings.vstpreset"

Claude will:
1. Verify plugin is loaded
2. Call load_preset
3. Plugin state fully restored
4. Confirm success
```

#### Create Preset Library
```
User: "Create 5 variations of this EQ and save each as a preset"

Claude will:
1. Load EQ plugin
2. Adjust parameters for variation 1
3. Save as eq-bright.vstpreset
4. Adjust for variation 2
5. Save as eq-warm.vstpreset
6. ... repeat for all variations
```

## Troubleshooting

### Plugin Not Found

**Symptom:** `scan_plugins` returns empty array or missing expected plugin

**Solutions:**
1. Verify plugin installed in standard directory
2. Check file permissions (plugin must be readable)
3. Try custom path: `scan_plugins({"path": "/custom/vst3/path"})`
4. Check logs: `RUST_LOG=debug cargo run 2>debug.log`

### Load Plugin Fails

**Symptom:** `load_plugin` returns error

**Common Causes:**
- Invalid UID (must be 32-character hex from scan results)
- Plugin requires dependencies not found
- Plugin initialization failure (check plugin compatibility)

**Debug Steps:**
```bash
# Enable debug logging
RUST_LOG=debug ./vst3-mcp-host 2>debug.log

# Check logs for specific error
grep "ERROR" debug.log
```

### Audio Processing Produces Silence

**Possible Causes:**
1. **Parameters not set:** Some plugins default to "off" state
   - Call `list_params` and verify bypass/enable parameters
2. **Sample rate mismatch:** Plugin rejects unsupported sample rates
   - Try different input file or explicit sample_rate in load_plugin
3. **Mono/stereo mismatch:** Plugin expects specific channel count
   - Verify input file channels match plugin expectations

**Diagnostic:**
```
1. Process file through bypass/transparent plugin (e.g., AGain at 0 dB)
2. If output matches input → plugin working
3. If output silent → check plugin parameters
```

### Parameter Changes Not Applied

**Symptom:** `set_param` succeeds but audio unchanged

**Remember:** Parameters are **queued** until next `process_audio` call.

**Correct Sequence:**
```
1. load_plugin
2. set_param (queued)
3. process_audio (changes applied here)
4. Output reflects parameter change
```

**Incorrect Sequence:**
```
1. load_plugin
2. process_audio (no parameters set yet)
3. set_param (queued but no subsequent processing)
4. Output doesn't reflect change
```

### Preset Load Fails

**Symptom:** `load_preset` returns error or restores incorrect state

**Common Issues:**
1. **Wrong plugin loaded:** Preset class ID doesn't match loaded plugin
   - Load the correct plugin first
2. **Corrupted preset file:** File damaged or incomplete
   - Try re-saving from plugin directly
3. **Plugin version mismatch:** Newer plugin, older preset format
   - May partially work or fail

### Performance Issues

**Symptom:** Processing takes very long

**Optimization:**
1. **Reduce buffer size:** Not configurable in v1 (uses 512 samples)
2. **Disable unused effects:** Load minimal plugin chains
3. **Check plugin tail:** Some reverbs have very long tails
   - Tail is auto-calculated, may add significant processing time

### MCP Connection Issues

**Symptom:** Claude can't see the tools

**Checklist:**
1. Verify binary path in settings.json is absolute and correct
2. Test binary runs: `./vst3-mcp-host` should wait for input (not crash)
3. Check MCP protocol version: Must be "2024-11-05" or newer
4. Restart MCP client after config changes
5. Check client logs for connection errors

## Advanced Usage

### Custom Plugin Paths

Scan non-standard directories:

```json
{
  "method": "tools/call",
  "params": {
    "name": "scan_plugins",
    "arguments": {
      "path": "/home/user/my-plugins/vst3"
    }
  }
}
```

### Sample Rate Specification

Force specific sample rate during plugin load:

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

Note: Input files must match this sample rate, or resampling may occur.

### Batch Processing

Process multiple files with same plugin settings:

```
User: "Process all WAV files in ./tracks/ with the current plugin settings"

Claude will:
1. Verify plugin loaded
2. Set any requested parameters
3. Loop through each .wav file:
   - process_audio(input=file, output=file_processed.wav)
4. Report completion stats
```

### Parameter Sweeps

Create parameter animations:

```
User: "Create a filter sweep from 100 Hz to 10 kHz over 10 seconds"

Claude will:
1. Load EQ/filter plugin
2. Generate 10 seconds of audio (or use input file)
3. Find frequency parameter
4. Create multiple passes:
   - set_param(freq_id, 0.1) → process → save
   - set_param(freq_id, 0.3) → process → save
   - set_param(freq_id, 0.5) → process → save
   - ... etc
5. Optionally concatenate outputs
```

Note: True sample-accurate sweeps not supported in v1 (parameter changes apply per-buffer, not per-sample).

## See Also

- [API.md](API.md) - Complete MCP tool reference
- [EXAMPLES.md](EXAMPLES.md) - Practical recipes and use cases
- [../README.md](../README.md) - Project overview and quick start
