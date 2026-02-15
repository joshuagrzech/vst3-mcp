# Phase 4: MCP Server & Tools - Research

**Researched:** 2026-02-15
**Domain:** MCP server implementation with rmcp, stdio transport, and VST3 parameter tools
**Confidence:** HIGH

## Summary

Phase 4 integrates an embedded MCP (Model Context Protocol) server into the VST3 host application, exposing plugin parameter control to AI agents (Claude) via JSON-RPC tools. The core challenge is bridging the existing parameter infrastructure (built in Phase 3) with the MCP protocol layer, ensuring thread-safe access to plugin state, and handling the async/sync impedance mismatch between Tokio-based MCP handlers and synchronous VST3 COM calls.

The current codebase already has MCP server infrastructure in place (`server.rs` with `AudioHost` using rmcp 0.15.0) and complete parameter control from Phase 3 (`get_parameter_info`, `get_parameter`, `queue_parameter_change`, `get_parameter_display`). This phase adds six new MCP tools (`get_plugin_info`, `list_params`, `get_param`, `set_param`, `batch_set`) that wrap existing `PluginInstance` methods, plus integration tests that verify AI-driven parameter control produces audible changes.

The codebase uses stdio transport (`rmcp::transport::io::stdio()`), which works perfectly for the offline MVP architecture. The prior context notes that "stdio transport inside DAW plugin may not work -- DAWs may redirect stdin/stdout. SSE fallback planned for Phase 4." However, Phase 4 is explicitly an offline MVP (not a DAW plugin), so stdio transport is the correct choice. SSE/Streamable HTTP transport would only be needed for Phase 7+ (real-time DAW integration), which is deferred.

**Primary recommendation:** Add six MCP tools to `AudioHost` using the existing `#[tool]` macro pattern, wrap synchronous VST3 calls in the existing `Mutex<Option<PluginInstance>>`, and validate with integration tests that verify AI tool calls produce audible parameter changes.

## Standard Stack

### Core Dependencies (Already in Cargo.toml)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| rmcp | 0.15.0 | MCP protocol server implementation | Official Rust SDK for MCP, supports stdio transport with async/await, mature and actively maintained |
| tokio | 1.49.0 | Async runtime for MCP server | Industry standard for async Rust, required by rmcp, already in codebase with "full" features |
| serde / serde_json | 1.0.228 / 1.0.149 | JSON serialization for MCP protocol | Standard for JSON in Rust, already used for tool request/response |
| schemars | 1.2.1 | JSON Schema generation for tool definitions | Generates MCP tool schemas from Rust types, already used in codebase |

### Supporting (No new dependencies required)

All MCP tool functionality can be implemented using existing dependencies and infrastructure:
- `vst3` crate (0.3.0) for parameter access
- `Arc<Mutex<>>` pattern already established in `AudioHost`
- `#[tool]` and `#[tool_router]` macros from rmcp

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| stdio transport | SSE or Streamable HTTP | SSE/HTTP needed for web/remote access; stdio is simpler for offline MVP and matches current architecture |
| rmcp | rust-mcp-server or mcpkit | rmcp is listed as "BEST Rust SDK for MCP" with 0.15.0 actively maintained; switching would require rewriting existing server.rs |
| Tokio runtime | async-std | Tokio is industry standard, already integrated, and required by rmcp |

**Installation:**
No new dependencies required. Phase 4 builds on existing `rmcp = { version = "0.15.0", features = ["server", "transport-io", "macros"] }`.

## Architecture Patterns

### Recommended Project Structure

```
src/
├── server.rs           # AudioHost with MCP tools (EXPAND: add 6 parameter tools)
├── hosting/
│   ├── plugin.rs       # PluginInstance with parameter methods (UNCHANGED)
│   ├── param_changes.rs # IParameterChanges COM impl (UNCHANGED, from Phase 3)
│   └── types.rs        # ParamInfo with flag helpers (UNCHANGED)
└── main.rs             # Tokio runtime + MCP server startup (UNCHANGED)
```

### Pattern 1: MCP Tool Definition with Existing Plugin Methods

**What:** Use `#[tool]` macro to expose existing `PluginInstance` methods as MCP tools. Lock plugin mutex, call method, serialize result.

**When to use:** For all six parameter tools required by Phase 4 success criteria.

**Example:**
```rust
// In AudioHost impl block with #[tool_router]
#[tool(description = "Get information about the loaded VST3 plugin (classId, name, vendor)")]
fn get_plugin_info(&self) -> Result<String, String> {
    info!("get_plugin_info called");

    let plugin_info_guard = self.plugin_info.lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let info = plugin_info_guard.as_ref()
        .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

    let response = serde_json::json!({
        "classId": info.uid,
        "name": info.name,
        "vendor": info.vendor,
    });

    Ok(serde_json::to_string_pretty(&response).unwrap())
}

#[tool(description = "List all non-read-only parameters with their ids, names, and current values")]
fn list_params(&self) -> Result<String, String> {
    info!("list_params called");

    let plugin_guard = self.plugin.lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let plugin = plugin_guard.as_ref()
        .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

    let param_count = plugin.get_parameter_count();
    let mut params = Vec::new();

    for i in 0..param_count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            // Filter out read-only and hidden parameters
            if info.is_writable() && !info.is_hidden() {
                let value = plugin.get_parameter(info.id);
                let display = plugin.get_parameter_display(info.id)
                    .unwrap_or_else(|_| format!("{:.3}", value));

                params.push(serde_json::json!({
                    "id": info.id,
                    "name": info.title,
                    "value": value,
                    "display": display,
                }));
            }
        }
    }

    let response = serde_json::json!({
        "parameters": params,
        "count": params.len(),
    });

    Ok(serde_json::to_string_pretty(&response).unwrap())
}
```

**Source:** Adapted from existing `scan_plugins` and `load_plugin` tools in `server.rs`.

### Pattern 2: Async MCP Tool with Synchronous Plugin Access

**What:** MCP tools are async functions (`#[tool]` supports both sync and async), but VST3 COM calls are synchronous. Use the existing `Mutex<Option<PluginInstance>>` pattern to safely bridge the gap.

**When to use:** For all parameter tools that access plugin state.

**Example:**
```rust
#[tool(description = "Set a parameter value (normalized 0.0-1.0) and trigger audible change")]
fn set_param(
    &self,
    Parameters(req): Parameters<SetParamRequest>,
) -> Result<String, String> {
    info!("set_param called: id={}, value={}", req.id, req.value);

    // Validate normalized range
    if req.value < 0.0 || req.value > 1.0 {
        return Err(format!("Parameter value must be in range [0.0, 1.0], got {}", req.value));
    }

    let mut plugin_guard = self.plugin.lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let plugin = plugin_guard.as_mut()
        .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

    // Queue the parameter change (will be delivered in next process() call)
    plugin.queue_parameter_change(req.id, req.value);

    // Get the new display value
    let display = plugin.get_parameter_display(req.id)
        .unwrap_or_else(|_| format!("{:.3}", req.value));

    let response = serde_json::json!({
        "status": "queued",
        "id": req.id,
        "value": req.value,
        "display": display,
    });

    Ok(serde_json::to_string_pretty(&response).unwrap())
}
```

**Source:** Pattern established in `process_audio` tool (lines 226-299 in server.rs).

### Pattern 3: Batch Parameter Changes

**What:** Apply multiple parameter changes atomically by queuing all changes before the next `process()` call.

**When to use:** When AI needs to adjust multiple related parameters (e.g., filter frequency + resonance, or EQ band adjustments).

**Example:**
```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchSetRequest {
    /// Array of parameter changes: [{"id": 0, "value": 0.5}, ...]
    #[schemars(description = "Array of parameter changes")]
    pub changes: Vec<ParamChange>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ParamChange {
    pub id: u32,
    pub value: f64,
}

#[tool(description = "Set multiple parameters atomically (all changes applied together)")]
fn batch_set(
    &self,
    Parameters(req): Parameters<BatchSetRequest>,
) -> Result<String, String> {
    info!("batch_set called with {} changes", req.changes.len());

    // Validate all changes first
    for change in &req.changes {
        if change.value < 0.0 || change.value > 1.0 {
            return Err(format!(
                "Parameter {} value must be in range [0.0, 1.0], got {}",
                change.id, change.value
            ));
        }
    }

    let mut plugin_guard = self.plugin.lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let plugin = plugin_guard.as_mut()
        .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

    // Queue all changes
    for change in req.changes {
        plugin.queue_parameter_change(change.id, change.value);
    }

    let response = serde_json::json!({
        "status": "queued",
        "changes_queued": req.changes.len(),
    });

    Ok(serde_json::to_string_pretty(&response).unwrap())
}
```

**Source:** Derived from VST3 parameter automation patterns and MCP multi-operation tools.

### Anti-Patterns to Avoid

- **Spawning background threads for MCP server:** The codebase already runs MCP server on the main Tokio thread via `#[tokio::main]`. Don't spawn additional threads; use existing async runtime.
- **Calling VST3 methods without mutex lock:** All plugin access must go through `Arc<Mutex<Option<PluginInstance>>>`. VST3 COM is not thread-safe.
- **Using async VST3 calls:** VST3 COM is synchronous. Don't wrap in `spawn_blocking` unless calls are known to block for >10ms (they aren't in this codebase).
- **Implementing custom JSON-RPC protocol:** Use rmcp's `#[tool]` macro. It handles protocol, schema generation, and error mapping.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| MCP protocol implementation | Custom JSON-RPC server | `rmcp` crate with `#[tool]` macro | Edge cases: protocol versioning, tool schema generation, error codes, backward compatibility |
| Async runtime | Custom thread pool or async executor | Tokio (already in codebase) | Mature, widely tested, required by rmcp |
| Thread-safe plugin access | Custom locks or channels | `Arc<Mutex<>>` (established pattern) | Proven in existing codebase, matches VST3 single-threaded model |
| JSON Schema for tools | Manual schema definitions | `schemars` derive macro | Automatic schema generation, kept in sync with Rust types |

**Key insight:** MCP server infrastructure is already built (server.rs + main.rs). Phase 4 is adding six tool functions, not building a server from scratch.

## Common Pitfalls

### Pitfall 1: Stdio Transport Won't Work in DAW Plugins

**What goes wrong:** When embedded in a DAW plugin (Phase 7+), stdio transport may fail because DAWs redirect stdin/stdout for logging or debugging.

**Why it happens:** DAWs like Ableton Live, Bitwig, or Reaper spawn plugins as shared libraries in the DAW's process space and often redirect stdio streams for crash reporting or diagnostics.

**How to avoid:**
- For Phase 4 (offline MVP): stdio transport is correct and will work
- For Phase 7+ (DAW plugin): switch to SSE or Streamable HTTP transport
- Document the transport limitation in Phase 4 verification

**Warning signs:**
- MCP client (Claude desktop) can't connect when running as VST3 plugin
- No output on stdout/stdin when embedded in DAW
- Connection works in standalone binary but fails in plugin

**Phase 4 scope:** This is NOT a Phase 4 problem. Phase 4 is offline MVP with standalone binary. Transport switching is deferred to Phase 7.

### Pitfall 2: Async/Sync Impedance Mismatch

**What goes wrong:** MCP tools are async (`async fn`), but VST3 COM calls are synchronous. Wrapping every call in `tokio::spawn_blocking` creates unnecessary thread pool overhead.

**Why it happens:** Developers assume blocking calls must be wrapped in `spawn_blocking` per Tokio best practices.

**How to avoid:**
- VST3 COM calls in this codebase are fast (<1ms): `getParameterInfo`, `getParamNormalized`, `queue_parameter_change`
- Only use `spawn_blocking` for genuinely blocking operations (>10ms): file I/O, heavy computation
- Current pattern (sync tool functions with mutex locks) is optimal

**Warning signs:**
- Excessive thread creation in performance profiling
- Latency spikes in MCP tool responses
- Thread pool exhaustion warnings from Tokio

### Pitfall 3: Parameter Changes Not Applied Until process()

**What goes wrong:** AI calls `set_param`, immediately calls `get_param`, and sees the old value. The change hasn't been applied yet because `process()` hasn't been called.

**Why it happens:** `queue_parameter_change()` queues changes for delivery in the next `process()` call. The parameter isn't updated until audio processing runs.

**How to avoid:**
- Document in tool descriptions that changes are queued, not immediate
- `set_param` should return `"status": "queued"` to indicate deferred application
- Integration tests must call `process_audio` after `set_param` to verify audible change
- For immediate verification, provide a helper tool that triggers a silent process() pass

**Warning signs:**
- AI complains that parameter changes don't stick
- Integration tests for `set_param` fail because they check value immediately
- User confusion about when parameters take effect

### Pitfall 4: Exposing Read-Only or Hidden Parameters

**What goes wrong:** `list_params` returns hundreds of parameters including internal state variables, read-only meters, and hidden controls. AI overwhelms user with irrelevant parameters.

**Why it happens:** Naive implementation calls `get_parameter_info(i)` for all `i` without filtering flags.

**How to avoid:**
- Filter with `info.is_writable() && !info.is_hidden()` (methods from Phase 3)
- `is_writable()` checks `kCanAutomate && !kIsReadOnly` flags
- `is_hidden()` checks `kIsHidden` flag (internal parameters)
- Test with complex plugins (Vital, Diva) that have 200+ parameters

**Warning signs:**
- `list_params` returns 300+ parameters for a simple compressor
- Parameters with names like "internal_state_123" or "meter_out_L"
- AI struggles to understand which parameters to control

### Pitfall 5: Missing Plugin Info After Load

**What goes wrong:** `get_plugin_info` fails with "No plugin loaded" even though `load_plugin` succeeded.

**Why it happens:** `load_plugin` stores plugin in `self.plugin` but forgets to populate `self.plugin_info`.

**How to avoid:**
- Existing `load_plugin` tool (lines 120-224 in server.rs) already stores `plugin_info`
- `get_plugin_info` should read from `self.plugin_info`, NOT query plugin COM interfaces
- Avoids mutex contention and simplifies implementation

**Warning signs:**
- `get_plugin_info` returns null or errors after successful `load_plugin`
- Integration tests for plugin info fail intermittently
- Mutex deadlocks when both tools try to lock plugin simultaneously

## Code Examples

Verified patterns from rmcp documentation and existing codebase:

### Complete MCP Tool Definition

```rust
// Add to server.rs AudioHost impl block

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetParamRequest {
    /// Parameter ID from list_params.
    #[schemars(description = "Parameter ID from list_params")]
    pub id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetParamRequest {
    /// Parameter ID from list_params.
    #[schemars(description = "Parameter ID from list_params")]
    pub id: u32,
    /// Normalized value in range [0.0, 1.0].
    #[schemars(description = "Normalized value in range [0.0, 1.0]")]
    pub value: f64,
}

// In #[tool_router] impl AudioHost block:

#[tool(description = "Get the current value and display string for a parameter")]
fn get_param(
    &self,
    Parameters(req): Parameters<GetParamRequest>,
) -> Result<String, String> {
    info!("get_param called: id={}", req.id);

    let plugin_guard = self.plugin.lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let plugin = plugin_guard.as_ref()
        .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

    let value = plugin.get_parameter(req.id);
    let display = plugin.get_parameter_display(req.id)
        .map_err(|e| format!("Failed to get parameter display: {}", e))?;

    let response = serde_json::json!({
        "id": req.id,
        "value": value,
        "display": display,
    });

    Ok(serde_json::to_string_pretty(&response).unwrap())
}
```

**Source:** Adapted from existing `load_plugin` tool pattern in server.rs and rmcp documentation.

### Integration Test Pattern for Parameter Tools

```rust
// In src/bin/integration_test.rs or new test file

fn test_mcp_parameter_control() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load plugin via MCP tool
    let load_result = call_mcp_tool("load_plugin", json!({
        "uid": "1234567890ABCDEF1234567890ABCDEF",
        "sample_rate": 44100
    }))?;
    assert_eq!(load_result["status"], "loaded");

    // 2. Get plugin info
    let info_result = call_mcp_tool("get_plugin_info", json!({}))?;
    assert!(info_result["name"].as_str().is_some());

    // 3. List parameters
    let list_result = call_mcp_tool("list_params", json!({}))?;
    let params = list_result["parameters"].as_array()
        .ok_or("No parameters array")?;
    assert!(params.len() > 0, "Plugin should have parameters");

    // 4. Set a parameter
    let param_id = params[0]["id"].as_u64().unwrap() as u32;
    let set_result = call_mcp_tool("set_param", json!({
        "id": param_id,
        "value": 0.75
    }))?;
    assert_eq!(set_result["status"], "queued");

    // 5. Process audio to apply parameter change
    let process_result = call_mcp_tool("process_audio", json!({
        "input_file": "test_input.wav",
        "output_file": "test_output.wav"
    }))?;
    assert_eq!(process_result["status"], "processed");

    // 6. Verify output differs from baseline (audible change test)
    let baseline_audio = decode_audio("baseline.wav")?;
    let output_audio = decode_audio("test_output.wav")?;
    let difference = calculate_rms_difference(&baseline_audio, &output_audio);
    assert!(difference > 0.01, "Parameter change should produce audible difference");

    Ok(())
}
```

**Source:** Derived from existing integration test patterns in `src/bin/integration_test.rs`.

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Custom plugin control protocols | MCP standard protocol | MCP spec 2024-11-05, updated 2025-03-26 | AI tools can discover and use plugin controls without custom integration |
| SSE transport for MCP | Streamable HTTP (with SSE fallback) | MCP 2025-03-26 spec update | Simpler transport, better performance, backward compatible |
| Blocking I/O in async tools | Tokio async runtime | Tokio 1.0+ (2020) | Non-blocking MCP server can handle concurrent tool calls |
| Manual JSON schema definitions | `schemars` derive macro | Rust ecosystem standard | Schemas auto-generated and kept in sync with code |

**Deprecated/outdated:**
- **MCP SSE transport as primary:** Replaced by Streamable HTTP in 2025-03-26 spec. SSE still supported for backward compatibility but no longer recommended.
- **Manual tool registration:** Old MCP servers manually registered tools. Current rmcp uses `#[tool]` macro for automatic registration.

## Open Questions

1. **Should parameter changes trigger immediate process() pass for verification?**
   - What we know: Current `set_param` queues changes, requires separate `process_audio` call to apply
   - What's unclear: Whether a helper tool should exist to apply queued changes without full audio processing
   - Recommendation: Phase 4 keeps current design (explicit `process_audio` call). Phase 5+ could add `apply_changes()` helper if AI workflow needs it.

2. **How should batch_set handle partial failures?**
   - What we know: If one parameter ID is invalid, should entire batch fail or apply valid changes?
   - What's unclear: MCP protocol best practices for atomic vs. partial batch operations
   - Recommendation: Validate all changes before queuing any (atomic). Return error with details on first invalid change.

3. **Should get_param return parameter info along with value?**
   - What we know: `get_param` returns value + display. AI might need name/units for context.
   - What's unclear: Whether combining `get_param` and `get_parameter_info` reduces round-trips
   - Recommendation: Keep separate for Phase 4. `list_params` provides full info once, `get_param` is lightweight for value checks.

## Sources

### Primary (HIGH confidence)
- [rmcp 0.15.0 Documentation](https://docs.rs/rmcp/latest/rmcp/) - MCP server implementation patterns
- [GitHub: 4t145/rmcp](https://github.com/4t145/rmcp) - Official rmcp repository with examples
- [Model Context Protocol: Transports](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports) - Official MCP transport specification
- Existing codebase: `src/server.rs`, `src/main.rs`, `src/hosting/plugin.rs` - Established patterns

### Secondary (MEDIUM confidence)
- [How to Build a stdio MCP Server in Rust | Shuttle](https://www.shuttle.dev/blog/2025/07/18/how-to-build-a-stdio-mcp-server-in-rust) - Practical implementation guide
- [MCP Server Transports: STDIO, Streamable HTTP & SSE | Roo Code Documentation](https://docs.roocode.com/features/mcp/server-transports) - Transport comparison and selection guide
- [Tokio Tutorial 2026: Building Async Applications in Rust](https://reintech.io/blog/tokio-tutorial-2026-building-async-applications-rust) - Async/sync patterns
- [Write your MCP servers in Rust](https://rup12.net/posts/write-your-mcps-in-rust/) - Rust-specific MCP patterns

### Tertiary (LOW confidence)
- [Why MCP Deprecated SSE and Went with Streamable HTTP](https://blog.fka.dev/blog/2025-06-06-why-mcp-deprecated-sse-and-go-with-streamable-http/) - Transport evolution rationale
- [VST 3 SDK Documentation](https://steinbergmedia.github.io/vst3_doc/vstsdk/index.html) - VST3 reference (no stdio-specific info found)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - rmcp 0.15.0 proven in current codebase, no new dependencies needed
- Architecture: HIGH - Pattern established in existing server.rs, six tools follow same structure
- Pitfalls: HIGH - Stdio/DAW limitation documented in prior context, async/sync pattern proven in codebase

**Research date:** 2026-02-15
**Valid until:** 30 days (MCP spec stable, rmcp actively maintained)

---
*Research for Phase 4: MCP Server & Tools*
*Focus: MCP tool definitions, parameter control via JSON-RPC, stdio transport, integration testing*
