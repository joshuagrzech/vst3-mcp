# VST3 MCP Host Examples

Practical examples and recipes for common audio processing tasks using AI-driven parameter control.

## Table of Contents

- [Basic Examples](#basic-examples)
- [Parameter Automation](#parameter-automation)
- [Batch Processing](#batch-processing)
- [Preset Workflows](#preset-workflows)
- [Advanced Recipes](#advanced-recipes)

## Basic Examples

### Example 1: Find and Load a Reverb Plugin

**Goal:** Discover available plugins and load a reverb effect.

**Conversation:**

```
User: "What reverb plugins do I have?"

Claude: Let me scan for VST3 plugins and look for reverb effects.
```

**MCP Calls:**

```json
// 1. Scan plugins
{
  "method": "tools/call",
  "params": {
    "name": "scan_plugins",
    "arguments": {}
  }
}

// Response shows plugins including:
// - "ValhallaRoom" (reverb)
// - "TAL-Reverb-4" (reverb)
// - etc.

// 2. Load specific reverb
{
  "method": "tools/call",
  "params": {
    "name": "load_plugin",
    "arguments": {
      "uid": "76616C68616C6C61526F6F6D00000000"  // ValhallaRoom UID
    }
  }
}
```

**Result:** ValhallaRoom loaded and ready for processing.

---

### Example 2: Process Audio with Default Settings

**Goal:** Apply a loaded plugin to an audio file.

**Conversation:**

```
User: "Process vocal.wav through the reverb and save as vocal-wet.wav"
```

**MCP Call:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "process_audio",
    "arguments": {
      "input_file": "/home/user/vocal.wav",
      "output_file": "/home/user/vocal-wet.wav"
    }
  }
}
```

**Result:** vocal-wet.wav contains the processed audio with reverb applied.

---

### Example 3: Inspect Plugin Parameters

**Goal:** See what parameters are available to adjust.

**Conversation:**

```
User: "What parameters does this reverb have?"
```

**MCP Call:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "list_params",
    "arguments": {}
  }
}
```

**Response:**

```json
{
  "parameters": [
    {"id": 0, "name": "Mix", "value": 0.5, "display": "50%"},
    {"id": 1, "name": "Decay", "value": 0.6, "display": "2.5s"},
    {"id": 2, "name": "Pre-Delay", "value": 0.2, "display": "20ms"},
    {"id": 3, "name": "Size", "value": 0.7, "display": "Large"}
  ],
  "count": 4
}
```

**Result:** Claude can now report and adjust these 4 parameters.

---

## Parameter Automation

### Example 4: Adjust Single Parameter

**Goal:** Make the reverb more subtle by reducing mix.

**Conversation:**

```
User: "Reduce the reverb mix to 30%"
```

**MCP Calls:**

```json
// 1. Set mix parameter (id 0 from list_params)
{
  "method": "tools/call",
  "params": {
    "name": "set_param",
    "arguments": {
      "id": 0,
      "value": 0.3
    }
  }
}

// 2. Process audio to apply change
{
  "method": "tools/call",
  "params": {
    "name": "process_audio",
    "arguments": {
      "input_file": "vocal.wav",
      "output_file": "vocal-subtle.wav"
    }
  }
}
```

**Result:** vocal-subtle.wav has 30% reverb mix (was 50%).

---

### Example 5: Multiple Parameter Changes

**Goal:** Create a large, spacious reverb sound.

**Conversation:**

```
User: "Make the reverb huge and spacious - long decay, big size, more mix"
```

**MCP Call:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "batch_set",
    "arguments": {
      "changes": [
        {"id": 0, "value": 0.7},   // Mix: 70%
        {"id": 1, "value": 0.9},   // Decay: very long
        {"id": 3, "value": 1.0}    // Size: maximum
      ]
    }
  }
}
```

**Result:** All three parameters set atomically, ready for next process_audio.

---

### Example 6: Iterative Parameter Tuning

**Goal:** Find the right compressor settings through iteration.

**Workflow:**

```
User: "Dial in a vocal compressor - medium compression"

Claude:
1. Load compressor plugin
2. Set initial values: threshold=0.4, ratio=0.5, attack=0.3, release=0.5
3. Process audio → vocal-v1.wav

User: "More aggressive"

Claude:
1. Adjust: threshold=0.3, ratio=0.7
2. Process audio → vocal-v2.wav

User: "Perfect! Save these settings"

Claude:
1. save_preset → vocal-compressor.vstpreset
```

**MCP Calls:**

```json
// Initial settings
{
  "method": "tools/call",
  "params": {
    "name": "batch_set",
    "arguments": {
      "changes": [
        {"id": 0, "value": 0.4},  // Threshold
        {"id": 1, "value": 0.5},  // Ratio
        {"id": 2, "value": 0.3},  // Attack
        {"id": 3, "value": 0.5}   // Release
      ]
    }
  }
}

// Process
{"method": "tools/call", "params": {"name": "process_audio", "arguments": {...}}}

// Adjust (more aggressive)
{
  "method": "tools/call",
  "params": {
    "name": "batch_set",
    "arguments": {
      "changes": [
        {"id": 0, "value": 0.3},
        {"id": 1, "value": 0.7}
      ]
    }
  }
}

// Process again
{"method": "tools/call", "params": {"name": "process_audio", "arguments": {...}}}

// Save
{"method": "tools/call", "params": {"name": "save_preset", "arguments": {"path": "vocal-compressor.vstpreset"}}}
```

---

## Batch Processing

### Example 7: Process Multiple Files

**Goal:** Apply same plugin settings to multiple audio files.

**Conversation:**

```
User: "Apply this EQ to all 10 drum tracks in ./drums/"
```

**Workflow:**

```
1. List files: ls drums/*.wav
2. For each file:
   - process_audio(input=file, output=file_processed.wav)
3. Report completion
```

**Pseudo-code:**

```python
files = ["kick.wav", "snare.wav", "hihat.wav", ...]

for file in files:
    process_audio(
        input_file=f"drums/{file}",
        output_file=f"drums/{file.replace('.wav', '_processed.wav')}"
    )
```

**Result:** 10 processed files in ./drums/ with EQ applied.

---

### Example 8: Parameter Variations

**Goal:** Create multiple versions with different parameter settings.

**Conversation:**

```
User: "Create 3 versions: light, medium, heavy compression"
```

**Workflow:**

```json
// Version 1: Light (threshold=0.6, ratio=0.3)
batch_set([{"id": 0, "value": 0.6}, {"id": 1, "value": 0.3}])
process_audio(input="vocal.wav", output="vocal-light.wav")

// Version 2: Medium (threshold=0.4, ratio=0.5)
batch_set([{"id": 0, "value": 0.4}, {"id": 1, "value": 0.5}])
process_audio(input="vocal.wav", output="vocal-medium.wav")

// Version 3: Heavy (threshold=0.2, ratio=0.8)
batch_set([{"id": 0, "value": 0.2}, {"id": 1, "value": 0.8}])
process_audio(input="vocal.wav", output="vocal-heavy.wav")
```

**Result:** 3 files with different compression intensities for comparison.

---

## Preset Workflows

### Example 9: Save and Recall Settings

**Goal:** Save current plugin state for later use.

**Conversation:**

```
User: "Save this as my default vocal chain"

Claude: I'll save the current plugin state.
```

**MCP Call:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "save_preset",
    "arguments": {
      "path": "/home/user/presets/vocal-chain-default.vstpreset"
    }
  }
}
```

**Later:**

```
User: "Load my default vocal chain"

Claude: Loading your saved preset.
```

**MCP Call:**

```json
{
  "method": "tools/call",
  "params": {
    "name": "load_preset",
    "arguments": {
      "path": "/home/user/presets/vocal-chain-default.vstpreset"
    }
  }
}
```

**Result:** All parameter values restored exactly as saved.

---

### Example 10: Create Preset Library

**Goal:** Build a collection of presets for different use cases.

**Conversation:**

```
User: "Create 5 EQ presets: bright, warm, scooped, telephone, radio"
```

**Workflow:**

```
For each preset:
1. Adjust EQ parameters for desired tone
2. Save to preset file with descriptive name

Bright:
- High shelf: +6dB at 8kHz
- save_preset("eq-bright.vstpreset")

Warm:
- Low shelf: +3dB at 200Hz
- High shelf: -2dB at 10kHz
- save_preset("eq-warm.vstpreset")

Scooped:
- Low shelf: +4dB at 100Hz
- Mid cut: -6dB at 1kHz
- High shelf: +4dB at 8kHz
- save_preset("eq-scooped.vstpreset")

... etc
```

**Result:** 5 reusable presets for different EQ applications.

---

## Advanced Recipes

### Example 11: Intelligent Parameter Search

**Goal:** AI systematically explores parameter space to achieve desired sound.

**Conversation:**

```
User: "Find compressor settings that make this vocal sound polished but natural"
```

**AI Strategy:**

```
1. Define parameter ranges:
   - Threshold: [0.3, 0.6] (gentle to medium)
   - Ratio: [0.3, 0.6] (low to medium ratios)
   - Attack: [0.2, 0.5] (preserve transients)
   - Release: [0.4, 0.7] (smooth to auto)

2. Test combinations:
   - Start with middle values
   - Process audio
   - Evaluate against "polished but natural" criteria:
     * Dynamic range preserved (not over-compressed)
     * Consistent volume level
     * No pumping/breathing artifacts

3. Iterate:
   - If too aggressive: increase threshold, reduce ratio
   - If too gentle: decrease threshold, increase ratio
   - Fine-tune attack/release for smoothness

4. Present best result to user
```

**MCP Calls (abbreviated):**

```json
// Test 1: Middle values
batch_set([{"id": 0, "value": 0.45}, {"id": 1, "value": 0.45}, ...])
process_audio(input="vocal.wav", output="test1.wav")

// Test 2: Gentler
batch_set([{"id": 0, "value": 0.5}, {"id": 1, "value": 0.4}, ...])
process_audio(input="vocal.wav", output="test2.wav")

// ... continue testing

// Final best settings
batch_set([{"id": 0, "value": 0.48}, {"id": 1, "value": 0.42}, ...])
process_audio(input="vocal.wav", output="vocal-final.wav")
save_preset("polished-vocal-comp.vstpreset")
```

---

### Example 12: Match Reference Audio

**Goal:** Adjust EQ to match spectral characteristics of reference track.

**Conversation:**

```
User: "Make my vocal sound like the reference (bright and airy)"
```

**AI Strategy:**

```
1. Analyze reference description: "bright and airy"
   - Bright → boost high frequencies (5-10kHz)
   - Airy → presence boost (2-4kHz), subtle high shelf

2. Load EQ plugin and enumerate parameters
   - Find high shelf frequency and gain
   - Find presence band (mid-high EQ)

3. Apply boost based on description:
   - Presence: +3dB at 3kHz
   - Air: +4dB at 10kHz shelf

4. Process and evaluate
5. Iterate if needed
```

**MCP Calls:**

```json
// Get EQ parameters
list_params()  // Find band IDs

// Apply boosts
batch_set([
  {"id": 3, "value": 0.65},  // Band 3 gain: +3dB
  {"id": 4, "value": 0.7},   // Band 4 gain: +4dB
  {"id": 5, "value": 0.8}    // High shelf frequency
])

process_audio(input="vocal.wav", output="vocal-bright.wav")
```

---

### Example 13: Parallel Processing Emulation

**Goal:** Create wet/dry blend using parameter control.

**Conversation:**

```
User: "Apply heavy compression but keep it subtle with parallel blend"
```

**Workflow:**

```
1. Process with heavy compression (mix=100%)
   - batch_set([threshold=0.2, ratio=0.9, mix=1.0])
   - process_audio → vocal-heavy.wav

2. Process with dry signal (mix=0% or bypass)
   - batch_set([mix=0.0])
   - process_audio → vocal-dry.wav

3. Instruct user to blend in DAW, OR:
4. Use plugin's built-in mix parameter:
   - batch_set([threshold=0.2, ratio=0.9, mix=0.3])
   - process_audio → vocal-parallel.wav (30% wet, 70% dry)
```

**MCP Calls:**

```json
// Heavy settings, 30% mix
{
  "method": "tools/call",
  "params": {
    "name": "batch_set",
    "arguments": {
      "changes": [
        {"id": 0, "value": 0.2},   // Threshold: low
        {"id": 1, "value": 0.9},   // Ratio: high
        {"id": 4, "value": 0.3}    // Mix: 30%
      ]
    }
  }
}

{
  "method": "tools/call",
  "params": {
    "name": "process_audio",
    "arguments": {
      "input_file": "vocal.wav",
      "output_file": "vocal-parallel.wav"
    }
  }
}
```

---

### Example 14: A/B Comparison Workflow

**Goal:** Create multiple processing variations for user comparison.

**Conversation:**

```
User: "Create 3 different reverb sounds so I can choose the best one"
```

**Workflow:**

```
Version A: Small room (short decay, tight)
- batch_set([decay=0.3, size=0.2, mix=0.4])
- process_audio → vocal-room-small.wav

Version B: Medium hall (balanced)
- batch_set([decay=0.6, size=0.6, mix=0.5])
- process_audio → vocal-hall-medium.wav

Version C: Large cathedral (long decay, spacious)
- batch_set([decay=0.9, size=1.0, mix=0.6])
- process_audio → vocal-cathedral-large.wav

Save each preset for recall:
- save_preset("reverb-small-room.vstpreset")
- save_preset("reverb-medium-hall.vstpreset")
- save_preset("reverb-large-cathedral.vstpreset")
```

**Result:** 3 processed files + 3 presets for instant comparison and recall.

---

### Example 15: Chain Multiple Plugins

**Goal:** Apply sequential processing with different plugins.

**Note:** Current version (v1.0) supports one plugin at a time. Chain by processing sequentially:

**Workflow:**

```
User: "Apply EQ, then compressor, then reverb"

Step 1: EQ
- load_plugin(eq_uid)
- batch_set([eq_params])
- process_audio(input="vocal.wav", output="vocal-eq.wav")

Step 2: Compressor
- load_plugin(compressor_uid)
- batch_set([comp_params])
- process_audio(input="vocal-eq.wav", output="vocal-eq-comp.wav")

Step 3: Reverb
- load_plugin(reverb_uid)
- batch_set([reverb_params])
- process_audio(input="vocal-eq-comp.wav", output="vocal-final.wav")
```

**Result:** vocal-final.wav has all 3 effects applied in sequence.

---

## Python Client Example

Complete example using Python to interact with the MCP server:

```python
#!/usr/bin/env python3
import subprocess
import json
import sys

class VST3Host:
    def __init__(self, binary_path="./target/release/vst3-mcp-host"):
        self.process = subprocess.Popen(
            [binary_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True
        )
        self._request_id = 0

    def call_tool(self, tool_name, arguments=None):
        """Call an MCP tool and return the result."""
        self._request_id += 1
        request = {
            "jsonrpc": "2.0",
            "id": self._request_id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments or {}
            }
        }

        # Send request
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()

        # Read response
        response_line = self.process.stdout.readline()
        response = json.loads(response_line)

        # Extract content from MCP format
        if "result" in response and "content" in response["result"]:
            content = response["result"]["content"][0]["text"]
            is_error = response["result"].get("isError", False)

            if is_error:
                raise Exception(f"Tool error: {content}")

            return json.loads(content)

        return response

# Usage example
host = VST3Host()

# Scan for plugins
plugins = host.call_tool("scan_plugins")
print(f"Found {plugins['count']} plugins")

# Load first plugin
uid = plugins['plugins'][0]['uid']
host.call_tool("load_plugin", {"uid": uid})

# List parameters
params = host.call_tool("list_params")
print(f"Plugin has {params['count']} parameters")

# Set first parameter to 75%
host.call_tool("set_param", {"id": 0, "value": 0.75})

# Process audio
host.call_tool("process_audio", {
    "input_file": "input.wav",
    "output_file": "output.wav"
})

print("Processing complete!")
```

---

## See Also

- [USAGE.md](USAGE.md) - Detailed usage guide
- [API.md](API.md) - Complete API reference
- [../README.md](../README.md) - Project overview
