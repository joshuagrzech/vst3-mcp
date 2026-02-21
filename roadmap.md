
Here are focused recommendations based on the codebase review.

---

## MCP Server Recommendations, Enhancements & Optimizations

### 1. **Tool parity**

The stdio shim (`agentaudio-mcp-stdio`) does not expose all tools the router offers:

- `list_params` – shim omits `prefix` (router uses `ProxyListParamsRequest` with `prefix`)
- `list_param_groups`
- `search_params`
- `get_param_info`
- `get_params_by_name`
- `get_current_patch_state`
- `save_preset` / `load_preset`
- `set_param_by_name`

The stdio shim does expose `scan_plugins` and `load_child_plugin`, but the router does not implement them, so those calls will fail when forwarded.

**Recommendation:** Make the stdio shim a thin passthrough that forwards all router tools and arguments. Either forward every router tool 1:1, or switch to dynamic tool discovery from the router instead of a hard-coded subset.

---

### 2. **Wrapper missing tools**

The router expects tools that the wrapper does not implement:

- `list_param_groups` – router forwards this; wrapper has no implementation
- `search_params` – same
- `get_params_by_name` – same
- `get_patch_state` – router calls this for `get_current_patch_state`; wrapper has no implementation

**Recommendation:** Implement these in the wrapper (mirroring `vst3_mcp_host::server` in `src/server.rs`), or move their logic into the router. For example, the router could call `list_params`, parse the result, and derive groups, search results, and patch state itself, like it does for `find_vst_parameter`.

---

### 3. **Connection reuse and timeouts**

In `call_wrapper_tool`, the router creates a new HTTP connection per call:

```rust
let transport = rmcp::transport::StreamableHttpClientTransport::from_uri(endpoint);
let service = ().serve(transport).await...
```

**Recommendation:** Reuse connections per instance (connection pool or cached client), or use a keep-alive client, to reduce latency and overhead when making many calls to the same wrapper endpoint.

Add explicit timeouts (and possibly retries) for wrapper calls so the router does not hang indefinitely on a slow or unresponsive wrapper.

---

### 4. **Caching in `find_vst_parameter`**

Both the router and wrapper call `list_params` for each `find_vst_parameter` request. For plugins with hundreds of parameters, this can be slow.

**Recommendation:** Cache the last `list_params` result per instance (with a TTL or invalidation on preset/param changes) so `find_vst_parameter` can work from the cache. Apply similar caching for `preview_vst_parameter_values` and other tools that depend on `list_params`.

---

### 5. **Document search paths**

`docs_base_dir()` resolves to paths relative to the build manifest:

```rust
const DEFAULT_PLUGIN_DOCS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/plugins");
```

This assumes a specific workspace layout and can fail in different builds or installs.

**Recommendation:** Prefer installation-relative paths (e.g. `$AGENTAUDIO_DOCS` or a well-defined install prefix) and provide clear fallbacks. Document `AGENTAUDIO_PLUGIN_DOCS_DIR` and `AGENTAUDIO_SOUND_DESIGN_DIR` and how they override defaults.

---

### 6. **Doc search performance**

`collect_md_files` and `score_files_for_excerpts` do full directory traversals and scoring on every request.

**Recommendation:** Load and index docs at startup (or lazily on first request) and keep an in-memory index. Refresh periodically or when the docs directory changes. This reduces I/O and CPU for repeated queries.

---

### 7. **Audio intent routing**

`audio_intent_analysis` uses fixed terms and simple scoring, which can misroute in mixed or ambiguous contexts.

**Recommendation:** Extend `AUDIO_INTENT_TERMS` and `HARD_AUDIO_ROUTE_TERMS` with more plugin/synth names and terms (e.g. “tipper”, “squelch”, “psytrance”). Consider using a small ML classifier or embeddings for intent if you need higher accuracy without overfitting.

---

### 8. **Error handling**

Wrapper errors are often surfaced as strings like `"Wrapper tool call failed: {e}"`, without structured error codes or types.

**Recommendation:** Introduce structured error types (e.g. `UnknownInstance`, `NoPluginLoaded`, `ParameterNotFound`, `Timeout`) and include them in JSON responses so clients can handle failures more reliably.

---

### 9. **Health checks**

The router prunes stale instances based on TTL but does not expose a dedicated health endpoint.

**Recommendation:** Add an HTTP health route (e.g. `/health`) that returns 200 and basic status (router up, instance count). This supports load balancers, monitoring, and service discovery.

---

### 10. **Observability**

Logging uses `tracing` but tool calls and wrapper interactions are not clearly instrumented.

**Recommendation:** Add spans and events for:

- Tool name and instance
- Wrapper call latency
- Cache hits/misses
- Audio intent and routing decisions

This will make debugging and tuning easier.

---

### 11. **Parameter queue behavior**

When the param queue is full, `queue_param_change` drops events with little visibility to the caller.

**Recommendation:** Return queue utilization (e.g. length and capacity) in responses (as in `set_param_realtime`), and optionally add a `param_queue_status` tool. Consider backpressure or clearer “queue full” errors so clients know to slow down.

---

### 12. **Shared types and consistency**

Parameter request types (e.g. `ProxyListParamsRequest`, `ProxyFindVstParameterRequest`) are duplicated between the router and stdio shim, and query/alias logic is split across router and wrapper.

**Recommendation:** Extract shared types and query logic into a library crate used by both router and stdio shim to keep behavior and schemas consistent.

---

### Priority overview

| Priority | Item | Impact |
|----------|------|--------|
| High | Add missing wrapper tools (`list_param_groups`, `search_params`, `get_params_by_name`, `get_patch_state`) | Fixes “tool not found” and enables full MCP workflows |
| High | Stdio shim tool parity with router | Ensures Cursor/VSCode clients get the same tools as direct router clients |
| Medium | Connection pooling or reuse for wrapper calls | Lower latency and better scalability |
| Medium | Param list caching in router | Faster `find_vst_parameter` and related tools |
| Medium | Doc search startup indexing | Faster doc search for plugin and sound design queries |
| Low | Structured error types and health endpoint | Better integration and debugging |