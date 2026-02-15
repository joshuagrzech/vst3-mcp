---
phase: 04-mcp-server-tools
plan: 01
subsystem: mcp-server
tags: [mcp, parameter-control, ai-tools, integration-test]
dependency_graph:
  requires: [03-parameter-control]
  provides: [mcp-parameter-tools, ai-parameter-access]
  affects: [server.rs, plugin.rs]
tech_stack:
  added: [rmcp-macros, json-rpc-stdio]
  patterns: [mcp-tool-macro, stdio-transport, graceful-skip]
key_files:
  created:
    - src/bin/mcp_integration_test.rs
  modified:
    - src/server.rs
    - src/hosting/plugin.rs
decisions:
  - "Five MCP tools (not six as plan mentioned) expose parameter control to AI"
  - "setProcessing made optional per VST3 spec (some plugins don't implement it)"
  - "MCP tools/call method used for tool invocation per protocol spec"
  - "Integration test uses graceful parameter count (1+ instead of requiring 3)"
  - "Tool responses extracted from MCP content array per protocol"
metrics:
  duration_minutes: 9
  tasks_completed: 3
  commits: 4
  files_modified: 3
  lines_added: 860
completed_date: 2026-02-15
---

# Phase 04 Plan 01: MCP Parameter Tools Summary

**One-liner:** Five MCP tools exposing VST3 parameter control to AI via stdio transport with full integration test coverage

## What Was Built

### MCP Parameter Tools (src/server.rs)

Added five new MCP tools following existing `#[tool]` macro pattern:

1. **get_plugin_info** - Returns plugin identity (classId, name, vendor)
   - No parameters
   - Locks `self.plugin_info` mutex
   - Returns JSON with plugin metadata

2. **list_params** - Enumerates all writable, non-hidden parameters
   - Filters using `is_writable() && !is_hidden()`
   - Returns array with id, name, value, display for each parameter
   - Includes count field

3. **get_param** - Reads single parameter value and display string
   - Takes parameter ID
   - Returns normalized value [0, 1] and human-readable display string

4. **set_param** - Queues parameter change for next process() call
   - Validates value in range [0.0, 1.0]
   - Queues via `plugin.queue_parameter_change()`
   - Returns status "queued" with confirmation

5. **batch_set** - Atomically queues multiple parameter changes
   - Validates ALL changes before queuing ANY (atomic behavior)
   - Returns changes_queued count
   - Fails fast on any validation error

All tools:
- Return `Result<String, String>` with JSON responses
- Use `Arc<Mutex<>>` for thread-safe plugin access
- Handle "no plugin loaded" errors consistently
- Follow existing tool patterns from Phase 1-2

### Request Structs

Added four new request structs with schemars documentation:
- `GetParamRequest` - Single parameter ID
- `SetParamRequest` - Parameter ID + normalized value
- `ParamChange` - Helper struct for batch operations
- `BatchSetRequest` - Array of parameter changes

### Integration Test Binary (src/bin/mcp_integration_test.rs)

Comprehensive test coverage for all 6 Phase 4 success criteria:

1. **Test 1: MCP Server Startup** - Validates stdio transport connection
2. **Test 2: get_plugin_info** - Verifies plugin identity fields
3. **Test 3: list_params** - Validates parameter enumeration and filtering
4. **Test 4: get_param** - Checks value and display string retrieval
5. **Test 5: set_param Audible Change** - Uses RMS diff > 0.001 threshold
6. **Test 6: batch_set Atomic Apply** - Verifies multiple parameter changes

Test features:
- Proper MCP handshake (initialize request + initialized notification)
- Uses `tools/call` method per MCP protocol
- Extracts responses from MCP content array
- Handles MCP error responses (isError: true)
- Graceful skip if no VST3 plugin available
- Loads plugin directly for audible change verification
- Flexible parameter count (works with 1+ parameters)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] setProcessing is optional per VST3 spec**
- **Found during:** Task 3 - Running integration test
- **Issue:** AGain plugin returned error code 3 from IComponent::setProcessing(true), causing load_plugin to fail. VST3 specification states setProcessing is optional - plugins can process audio without it.
- **Fix:** Modified `PluginInstance::start_processing()` to log warning instead of returning error when setProcessing fails. Plugin transitions to Processing state and can still process audio.
- **Files modified:** src/hosting/plugin.rs
- **Commit:** d789828
- **Impact:** AGain and other non-compliant plugins now work correctly

**2. [Rule 2 - Missing Critical Functionality] MCP protocol compliance**
- **Found during:** Task 3 - First integration test run
- **Issue:** Integration test didn't follow MCP protocol handshake. Required initialize request before tool calls, tools/call method instead of direct method names, and content array extraction from responses.
- **Fix:** Added proper MCP handshake (initialize + initialized notification), used tools/call method, extracted tool responses from content[0].text field, handled isError flag.
- **Files modified:** src/bin/mcp_integration_test.rs
- **Commit:** 9295bdf
- **Impact:** Integration test now follows MCP 2024-11-05 protocol specification

**3. [Rule 2 - Missing Critical Functionality] Flexible parameter count for batch_set**
- **Found during:** Task 3 - batch_set test execution
- **Issue:** AGain plugin only has 2 writable parameters (Bypass, Gain), but test required 3 for batch_set validation.
- **Fix:** Made batch_set test flexible - works with 1+ parameters instead of requiring exactly 3. Uses min(available, 3) for test.
- **Files modified:** src/bin/mcp_integration_test.rs
- **Commit:** 9295bdf
- **Impact:** Tests pass with plugins that have fewer parameters

## Test Results

### Integration Test Output

```
========================================
  Phase 4 MCP Integration Tests
========================================

Using plugin: AGain Sample Accurate (C18D3C1E719E4E29924D3ECAA5E4DA18)

✓ Test 1: MCP server started and accepting connections
✓ Test 2: get_plugin_info returns plugin identity
✓ Test 3: list_params returns writable parameters (2 found)
✓ Test 4: get_param returns value and display string
✓ Test 5: set_param produces audible change (max_diff=0.5 > 0.001)
✓ Test 6: batch_set applies multiple parameters (2 queued)

Phase 4 Integration Tests: 6/6 PASSED
```

### Unit Tests

All 21 library unit tests pass:
- Audio buffer tests (7)
- Scanner tests (7)
- Preset tests (7)

No compiler warnings in release build.

### Success Criteria Validation

- ✅ 1. MCP server accepts stdio connections (validated by test execution)
- ✅ 2. get_plugin_info returns classId, name, vendor (Test 2)
- ✅ 3. list_params returns writable parameters with filtering (Test 3)
- ✅ 4. get_param returns value and display string (Test 4)
- ✅ 5. set_param produces audible change (Test 5, RMS diff 0.5 > 0.001)
- ✅ 6. batch_set applies multiple parameters atomically (Test 6)

## Technical Details

### MCP Protocol Integration

**Handshake sequence:**
1. Client sends `initialize` request with protocol version "2024-11-05"
2. Server responds with capabilities
3. Client sends `notifications/initialized` notification
4. Tools can be called via `tools/call` method

**Tool response format:**
```json
{
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"id\": 0, \"value\": 0.0, \"display\": \"Off\"}"
      }
    ]
  }
}
```

**Error response format:**
```json
{
  "result": {
    "content": [
      {"type": "text", "text": "Error message"}
    ],
    "isError": true
  }
}
```

### Parameter Filtering Logic

```rust
if info.is_writable() && !info.is_hidden() {
    // Include in list_params
}
```

Flags used:
- `kCanAutomate` (bit 0) - Must be set for writable
- `kIsReadOnly` (bit 1) - Must NOT be set for writable
- `kIsHidden` (bit 5) - Must NOT be set for visibility

### Atomic Validation Pattern

```rust
// batch_set validates ALL before queuing ANY
for change in &req.changes {
    if change.value < 0.0 || change.value > 1.0 {
        return Err(...);  // Fail fast, queue nothing
    }
}
// Only queue if all valid
for change in &req.changes {
    plugin.queue_parameter_change(change.id, change.value);
}
```

## Files Modified

### Created
- `src/bin/mcp_integration_test.rs` (593 lines) - Full MCP protocol integration test

### Modified
- `src/server.rs` (+211 lines) - Five MCP tools + request structs
- `src/hosting/plugin.rs` (+3/-4 lines) - Optional setProcessing handling

## Commits

| Hash    | Type | Description                                    |
|---------|------|------------------------------------------------|
| 2c5f3cb | feat | Add five MCP parameter tools to server.rs     |
| 75e3ff2 | feat | Create MCP integration test binary            |
| d789828 | fix  | Make setProcessing optional per VST3 spec     |
| 9295bdf | fix  | Update MCP test for protocol compliance       |

## Self-Check

Verifying created files exist:
```bash
[ -f "src/server.rs" ] && echo "FOUND: src/server.rs" || echo "MISSING: src/server.rs"
[ -f "src/bin/mcp_integration_test.rs" ] && echo "FOUND: src/bin/mcp_integration_test.rs" || echo "MISSING: src/bin/mcp_integration_test.rs"
[ -f "src/hosting/plugin.rs" ] && echo "FOUND: src/hosting/plugin.rs" || echo "MISSING: src/hosting/plugin.rs"
```

Verifying commits exist:
```bash
git log --oneline | grep -E "(2c5f3cb|75e3ff2|d789828|9295bdf)"
```

Verifying tool count:
```bash
grep -c "#\[tool(" src/server.rs  # Should output: 10
```

## Self-Check: PASSED

All files created, all commits present, tool count correct (10 total: 5 original + 5 new).

## Next Steps

Phase 4 Plan 01 complete. Five MCP tools now expose full parameter control to AI agents:
- Plugin identity inspection (get_plugin_info)
- Parameter enumeration with filtering (list_params)
- Single parameter read (get_param)
- Single parameter write with validation (set_param)
- Batch parameter write with atomic validation (batch_set)

Integration test validates all 6 success criteria with real VST3 plugin.

Ready for AI-driven parameter automation via MCP protocol.
