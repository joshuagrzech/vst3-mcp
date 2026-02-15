# Phase 5: Focus Mode - Research

**Researched:** 2026-02-15
**Domain:** Configuration-driven parameter filtering with JSON whitelisting and dynamic reload
**Confidence:** HIGH

## Summary

Phase 5 implements Focus Mode, a configuration-driven parameter filtering system that exposes only user-selected VST3 parameters to AI agents. The core challenge is loading a JSON configuration file that maps plugin classId (TUID) to a set of exposed parameter IDs, filtering the `list_params` MCP tool output based on this config, and supporting hot-reload so changes take effect without restarting the application.

The current codebase already implements complete parameter enumeration and filtering in `list_params` (src/server.rs:416-456), which currently filters by `is_writable() && !is_hidden()`. Phase 5 adds a third filter dimension: "is this parameter ID in the Focus Mode config for the current plugin?" The filtering logic extends the existing pattern with minimal changes: load config into `Arc<Mutex<FocusConfig>>`, check membership in a `HashSet<u32>` during the parameter enumeration loop, and fall back to "all writable parameters" when no config exists.

Hot-reloading is explicitly deferred in the success criteria ("Modifying the Focus Mode config and reloading changes which parameters appear"). The success criteria says "reloading" not "automatic hot-reload", meaning the user must trigger a reload action (e.g., re-calling `load_plugin` or a new `reload_focus_config` MCP tool). Full file-watching with the `notify` crate would add complexity for v1 and is better suited for Phase 7+ (real-time DAW integration).

**Primary recommendation:** Define FocusConfig struct with `HashMap<String, HashSet<u32>>` (classId -> param IDs), load from JSON file using `serde_json::from_str` with proper error handling, store in `Arc<Mutex<Option<FocusConfig>>>` on AudioHost, filter parameters in `list_params` by checking HashSet membership, and default to "all writable parameters" when config is None or plugin classId not found.

## Standard Stack

### Core Dependencies (Already in Cargo.toml)

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| serde / serde_json | 1.0.228 / 1.0.149 | JSON config file parsing | Standard for JSON in Rust, already used throughout codebase |
| std::collections::HashMap | stdlib | Map plugin classId to parameter set | Standard library, O(1) lookup for classId |
| std::collections::HashSet | stdlib | Store exposed parameter IDs | O(1) membership test for filtering |
| std::fs::read_to_string | stdlib | Read JSON config from disk | Standard file I/O, no dependencies needed |

### Supporting (Optional - Phase 7+)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| notify | 8.2.0 | File watching for hot-reload | Phase 7+ real-time DAW integration; deferred for v1 |
| directories | Latest | Cross-platform config directory resolution | If moving config from project-local to ~/.config/agent-audio/ |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| HashMap<String, HashSet<u32>> | BTreeMap for sorted iteration | HashMap is faster O(1) vs O(log n), sorting not needed |
| HashSet<u32> | Vec<u32> with linear search | HashSet O(1) membership vs Vec O(n), critical for 100+ param plugins |
| Manual reload | notify crate auto-reload | Auto-reload adds file watcher thread, overkill for offline MVP |
| Project-local config | XDG_CONFIG_HOME (~/.config) | Project-local simpler for v1, XDG for multi-project workflows later |

**Installation:**
No new dependencies required. Phase 5 builds entirely on existing `serde = { version = "1.0.228", features = ["derive"] }` and `serde_json = "1.0.149"`.

## Architecture Patterns

### Recommended Project Structure

```
src/
├── server.rs           # AudioHost with focus_config field (EXPAND)
├── hosting/
│   ├── plugin.rs       # PluginInstance (UNCHANGED)
│   └── types.rs        # FocusConfig struct (NEW)
└── main.rs             # MCP server startup (UNCHANGED)
```

**Alternative:** FocusConfig could live in `server.rs` if it's only used there. Putting it in `types.rs` follows existing pattern (PluginInfo lives there).

### Pattern 1: JSON Config Schema with Serde

**What:** Define Rust struct matching JSON schema, derive Deserialize, load with `serde_json::from_str`.

**When to use:** Always for config files. Type-safe deserialization catches schema errors at load time.

**JSON Schema:**
```json
{
  "focus_mode": {
    "PLUGIN_CLASSID_HEX_1": [1, 5, 12, 24],
    "PLUGIN_CLASSID_HEX_2": [0, 3, 7]
  }
}
```

**Rust struct:**
```rust
// In src/hosting/types.rs
use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};

/// Focus Mode configuration mapping plugin classId to exposed parameter IDs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FocusConfig {
    /// Map of plugin classId (hex string) to set of exposed parameter IDs.
    pub focus_mode: HashMap<String, HashSet<u32>>,
}

impl FocusConfig {
    /// Load Focus Mode config from JSON file path.
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {}", path, e))?;

        let config: FocusConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config JSON: {}", e))?;

        Ok(config)
    }

    /// Get exposed parameter IDs for a given plugin classId (case-insensitive).
    pub fn get_exposed_params(&self, class_id: &str) -> Option<&HashSet<u32>> {
        // Normalize to uppercase for case-insensitive lookup
        let normalized = class_id.to_uppercase();
        self.focus_mode.get(&normalized)
    }
}
```

**Source:** Pattern from [Serde field attributes](https://serde.rs/field-attrs.html) and [Default values](https://serde.rs/attr-default.html).

### Pattern 2: Optional Config with Sensible Defaults

**What:** Use `Option<FocusConfig>` so missing config = "expose all writable params". Load config if file exists, otherwise None.

**When to use:** When config is optional and system should work without it (Phase 5 success criterion #3).

**Implementation:**
```rust
// In AudioHost struct
pub struct AudioHost {
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    plugin_info: Arc<Mutex<Option<PluginInfo>>>,
    module: Arc<Mutex<Option<Arc<VstModule>>>>,
    scan_cache: Arc<Mutex<Vec<PluginInfo>>>,

    // NEW: Focus Mode config (None = expose all writable params)
    focus_config: Arc<Mutex<Option<FocusConfig>>>,

    tool_router: ToolRouter<Self>,
}

impl AudioHost {
    pub fn new() -> Self {
        // Try to load focus config from default path, fall back to None
        let focus_config = match FocusConfig::load_from_file("focus_mode.json") {
            Ok(config) => {
                info!("Focus Mode config loaded: {} plugins configured",
                      config.focus_mode.len());
                Some(config)
            }
            Err(e) => {
                info!("No Focus Mode config found ({}), exposing all writable params", e);
                None
            }
        };

        Self {
            plugin: Arc::new(Mutex::new(None)),
            plugin_info: Arc::new(Mutex::new(None)),
            module: Arc::new(Mutex::new(None)),
            scan_cache: Arc::new(Mutex::new(Vec::new())),
            focus_config: Arc::new(Mutex::new(focus_config)),
            tool_router: Self::tool_router(),
        }
    }
}
```

**Source:** Rust idiom for optional configuration, adapted from [Serde default values](https://serde.rs/attr-default.html).

### Pattern 3: HashSet Filtering in list_params

**What:** Check `HashSet::contains(param_id)` during parameter enumeration loop. O(1) membership test.

**When to use:** Filtering against a whitelist with fast lookup required (100+ parameters).

**Implementation:**
```rust
#[tool(description = "List all writable parameters with current values. Filtered by Focus Mode if configured. Call load_plugin first.")]
fn list_params(&self) -> Result<String, String> {
    info!("list_params called");

    let plugin_guard = self
        .plugin
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let plugin = plugin_guard
        .as_ref()
        .ok_or_else(|| "No plugin loaded. Call load_plugin first.".to_string())?;

    // Get current plugin classId for Focus Mode lookup
    let plugin_info_guard = self
        .plugin_info
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let plugin_class_id = plugin_info_guard
        .as_ref()
        .map(|info| info.uid.clone());

    // Get Focus Mode config for current plugin (if any)
    let focus_config_guard = self
        .focus_config
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let exposed_params = match (&*focus_config_guard, &plugin_class_id) {
        (Some(config), Some(class_id)) => config.get_exposed_params(class_id),
        _ => None, // No config or no plugin loaded = expose all
    };

    let count = plugin.get_parameter_count();
    let mut parameters = Vec::new();

    for i in 0..count {
        if let Ok(info) = plugin.get_parameter_info(i) {
            // Filter 1: Must be writable and not hidden (existing logic)
            if !info.is_writable() || info.is_hidden() {
                continue;
            }

            // Filter 2: Must be in Focus Mode config (if config exists)
            if let Some(allowed) = exposed_params {
                if !allowed.contains(&info.id) {
                    continue; // Parameter not exposed in Focus Mode
                }
            }

            // Parameter passed filters, include it
            let value = plugin.get_parameter(info.id);
            let display = plugin
                .get_parameter_display(info.id)
                .unwrap_or_else(|_| format!("{:.3}", value));

            parameters.push(serde_json::json!({
                "id": info.id,
                "name": info.title,
                "value": value,
                "display": display,
            }));
        }
    }

    let response = serde_json::json!({
        "parameters": parameters,
        "count": parameters.len(),
    });

    info!("list_params found {} exposed parameters", parameters.len());
    Ok(serde_json::to_string_pretty(&response).unwrap())
}
```

**Performance:** HashSet membership test is O(1) average case. For 100 parameters with 10 exposed, this adds ~100 hash lookups, negligible compared to COM calls.

**Source:** Pattern adapted from [Rust HashSet contains](https://doc.rust-lang.org/std/collections/struct.HashSet.html) and existing `list_params` logic.

### Pattern 4: Manual Reload via MCP Tool (Not Auto-Reload)

**What:** Add `reload_focus_config` MCP tool that re-reads JSON file and updates `Arc<Mutex<Option<FocusConfig>>>`.

**When to use:** Phase 5 success criterion #4 says "reloading changes which parameters appear" but doesn't require automatic file watching.

**Implementation:**
```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReloadFocusConfigRequest {
    /// Optional path to focus config JSON. Defaults to "focus_mode.json".
    #[schemars(description = "Optional path to focus config JSON. Defaults to focus_mode.json")]
    pub path: Option<String>,
}

#[tool(description = "Reload Focus Mode configuration from JSON file. Changes apply to next list_params call.")]
fn reload_focus_config(
    &self,
    Parameters(req): Parameters<ReloadFocusConfigRequest>,
) -> Result<String, String> {
    let path = req.path.as_deref().unwrap_or("focus_mode.json");
    info!("reload_focus_config called: {}", path);

    let new_config = match FocusConfig::load_from_file(path) {
        Ok(config) => {
            info!("Focus Mode config reloaded: {} plugins configured",
                  config.focus_mode.len());
            Some(config)
        }
        Err(e) => {
            return Err(format!("Failed to reload config: {}", e));
        }
    };

    let mut config_guard = self
        .focus_config
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    *config_guard = new_config;

    let response = serde_json::json!({
        "status": "reloaded",
        "path": path,
    });

    Ok(serde_json::to_string_pretty(&response).unwrap())
}
```

**Alternative (Phase 7+):** Use `notify` crate with file watcher thread that auto-reloads on changes. Requires spawning watcher thread, debouncing file events (100ms typical), and handling cross-platform file system differences. Overkill for offline MVP.

**Source:** Manual reload pattern standard in CLI tools. Auto-reload via notify documented in [Rust hot-reloading](https://github.com/junkurihara/rust-hot-reloader) and [notify examples](https://github.com/notify-rs/notify/blob/main/examples/hot_reload_tide/src/main.rs).

### Anti-Patterns to Avoid

- **Vec<u32> instead of HashSet<u32>:** Linear search O(n) vs constant time O(1). With 100+ parameters, this matters.
- **Dynamic allocation in list_params:** The parameter filtering loop shouldn't allocate per parameter. HashSet membership is stack-only.
- **Global static config:** Using `lazy_static!` or `OnceLock` prevents reload without restart. Use `Arc<Mutex<Option<>>>` for mutability.
- **Panic on missing config:** Config should be optional. `expect()` or `unwrap()` on file load would crash on missing file.
- **Case-sensitive classId lookup:** VST3 UIDs are hex strings, normalize to uppercase for robustness (handles "a1b2c3d4" vs "A1B2C3D4").

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| JSON parsing | Manual string parsing | serde_json | Schema validation, error messages, edge cases (escaped strings, unicode, numbers) |
| File watching | Poll loop checking mtime | notify crate (Phase 7+) | Cross-platform inotify/FSEvents/ReadDirectoryChanges, debouncing, resource cleanup |
| Configuration validation | Runtime checks scattered in code | JSON Schema + Serde validation | Fail fast at load time, clear error messages, self-documenting schema |
| Hash algorithm | Custom hash function | std::collections::hash_map::DefaultHasher | Cryptographic security not needed, default is fast and good enough |

**Key insight:** Configuration handling looks simple but has edge cases (file encoding, JSON escape sequences, concurrent modification, platform path differences). Using serde_json handles these correctly.

## Common Pitfalls

### Pitfall 1: Forgetting Default Behavior

**What goes wrong:** Code assumes config always exists, crashes when file missing or malformed.

**Why it happens:** Phase 5 success criterion #3 requires "Without any config file, list_params returns all non-read-only parameters". Easy to overlook during implementation.

**How to avoid:** Use `Option<FocusConfig>` everywhere. Test with no config file explicitly.

**Warning signs:** Calls to `expect()`, `unwrap()`, or `.ok_or("Config required")` during config load.

### Pitfall 2: Plugin ClassId Case Sensitivity

**What goes wrong:** Focus config uses lowercase classId "a1b2c3d4ef567890...", but PluginInfo stores uppercase "A1B2C3D4EF567890...". HashMap lookup fails silently, all parameters filtered out.

**Why it happens:** VST3 TUID is case-insensitive hex, but HashMap is case-sensitive. Different tools/plugins may emit different casing.

**How to avoid:** Normalize classId to uppercase in both config lookup and HashMap keys. Document that config file should use uppercase.

**Warning signs:** Focus Mode config loads successfully but list_params still returns all parameters. Check HashMap key normalization.

### Pitfall 3: Stale Config After Plugin Switch

**What goes wrong:** User loads Plugin A with Focus Mode config, then loads Plugin B. If code caches the "exposed params" reference from Plugin A, Plugin B gets wrong filter.

**Why it happens:** Caching exposed_params HashSet reference outside the lock can lead to stale data.

**How to avoid:** Re-query `focus_config.get_exposed_params(class_id)` inside `list_params` every time. Lock overhead is negligible for config reads.

**Warning signs:** list_params returns correct params for first plugin but wrong params after load_plugin called again.

### Pitfall 4: Lock Ordering Deadlock

**What goes wrong:** `list_params` locks `focus_config` then `plugin_info` then `plugin`. Another tool locks them in different order. Deadlock.

**Why it happens:** Multiple mutexes without consistent lock ordering.

**How to avoid:** Always lock in same order: `plugin` → `plugin_info` → `focus_config`. Or minimize critical sections (release locks between reads).

**Warning signs:** MCP tools hang under concurrent requests. Use `cargo test --features deadlock-detection` or manual code review of lock ordering.

### Pitfall 5: Exposing Read-Only or Hidden Parameters

**What goes wrong:** User puts read-only parameter ID in Focus Mode config, AI tries to write it via `set_param`, get cryptic error.

**Why it happens:** Config schema doesn't validate that parameter IDs are writable. User might copy parameter IDs from plugin documentation without checking flags.

**How to avoid:** Filter with BOTH Focus Mode AND `is_writable() && !is_hidden()`. Focus Mode is an additional filter, not a replacement.

**Warning signs:** User reports "set_param fails even though parameter is in list_params". Check if parameter is read-only.

## Code Examples

Verified patterns for Phase 5 implementation:

### Loading JSON Config with Error Handling

```rust
// Source: Serde error handling best practices
// https://serde.rs/error-handling.html

use std::fs;
use serde_json;

fn load_config_safe(path: &str) -> Result<FocusConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("Config file '{}' not found", path)
            } else {
                format!("Failed to read '{}': {}", path, e)
            }
        })?;

    serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in '{}': {} (line {}, column {})",
                              path, e, e.line(), e.column()))
}
```

### HashMap with Case-Insensitive Lookup

```rust
// Source: Rust std::collections::HashMap
// https://doc.rust-lang.org/std/collections/struct.HashMap.html

impl FocusConfig {
    pub fn get_exposed_params(&self, class_id: &str) -> Option<&HashSet<u32>> {
        // Normalize to uppercase for case-insensitive matching
        let key = class_id.to_uppercase();
        self.focus_mode.get(&key)
    }
}
```

### Optional Config with Default Fallback

```rust
// Source: Rust Option combinator patterns
// https://doc.rust-lang.org/std/option/index.html

// Load config with graceful fallback
let focus_config = FocusConfig::load_from_file("focus_mode.json")
    .ok(); // Convert Result<T, E> to Option<T>, swallowing error

// Use config if exists, otherwise default behavior
let exposed = focus_config
    .as_ref()
    .and_then(|cfg| cfg.get_exposed_params(&class_id));

match exposed {
    Some(param_set) => {
        // Filter using param_set.contains(&id)
    }
    None => {
        // Expose all writable parameters (default)
    }
}
```

### Thread-Safe Config Reload

```rust
// Source: Rust Arc and Mutex shared state patterns
// https://doc.rust-lang.org/book/ch16-03-shared-state.html

use std::sync::{Arc, Mutex};

struct AudioHost {
    focus_config: Arc<Mutex<Option<FocusConfig>>>,
}

impl AudioHost {
    fn reload_focus_config(&self, path: &str) -> Result<(), String> {
        let new_config = FocusConfig::load_from_file(path)?;

        let mut guard = self.focus_config.lock()
            .map_err(|e| format!("Lock poisoned: {}", e))?;

        *guard = Some(new_config);
        Ok(())
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| lazy_static! for globals | std::sync::OnceLock / LazyLock | Rust 1.70 (2023) / 1.80 (2024) | Standard library support, no macro needed |
| notify v4 | notify v8.2.0 | 2023-2024 | Better cross-platform support, debouncing built-in |
| once_cell crate | std::sync::OnceLock | Rust 1.70 (2023) | Standard library stabilized lazy initialization |
| Manual JSON parsing | serde_json with derive macros | Serde 1.0 (2017, stable) | Type-safe deserialization, schema validation |

**Deprecated/outdated:**
- **lazy_static crate:** Replaced by std::sync::LazyLock in Rust 1.80+. Still works but standard library preferred.
- **config-rs with env vars:** Heavyweight for simple JSON config. Serde sufficient for Phase 5 needs.
- **Global mutable static:** Unsafe without synchronization. Use Arc<Mutex<>> instead.

**Current best practices (as of 2026):**
- Use serde_json for JSON config loading (standard, 62M+ downloads)
- Use std::sync::OnceLock for lazy initialization if Rust 1.70+ (standard library)
- Use Arc<Mutex<Option<T>>> for reloadable shared state (Tokio/async compatible)
- Use HashSet for O(1) whitelist filtering (stdlib, no dependencies)

## Open Questions

### 1. Config File Location

**What we know:** Offline MVP runs as standalone binary, no installation. Config could be:
- Project-local: `./focus_mode.json` (simple, works with git)
- User config dir: `~/.config/agent-audio/focus_mode.json` (XDG spec compliant)

**What's unclear:** Where should config file live for best UX?

**Recommendation:** Project-local for Phase 5. XDG dirs for Phase 7+ (DAW plugin needs system-wide config). Use `directories` crate if moving to XDG.

### 2. Hot-Reload vs Manual Reload

**What we know:** Success criterion #4 says "Modifying the Focus Mode config and reloading changes which parameters appear" but doesn't specify automatic file watching.

**What's unclear:** Does "reloading" mean user calls a tool, or should it auto-detect file changes?

**Recommendation:** Manual reload via `reload_focus_config` MCP tool for Phase 5. Auto-reload via `notify` crate deferred to Phase 7+ (real-time DAW integration needs it more).

### 3. Config Schema Versioning

**What we know:** Phase 5 schema is simple (classId -> param IDs). Future phases might add:
- Parameter ranges/constraints (PARAM-07: Validation)
- Parameter grouping (UI organization)
- Per-parameter AI hints (descriptions for Claude)

**What's unclear:** Should we add a `"version": 1` field now for future-proofing?

**Recommendation:** No version field for Phase 5. Add when schema actually changes (Phase 7+). YAGNI principle.

## Sources

### Primary (HIGH confidence)

- [Serde field attributes](https://serde.rs/field-attrs.html) - Optional field handling, default values
- [Serde error handling](https://serde.rs/error-handling.html) - Proper JSON parse error reporting
- [Rust HashSet documentation](https://doc.rust-lang.org/std/collections/struct.HashSet.html) - O(1) membership testing
- [Rust Arc and Mutex](https://doc.rust-lang.org/book/ch16-03-shared-state.html) - Shared state concurrency
- [serde_json crate](https://docs.rs/serde_json) - JSON parsing with serde
- [Rust HashMap documentation](https://doc.rust-lang.org/std/collections/struct.HashMap.html) - Key-value lookups

### Secondary (MEDIUM confidence)

- [How to Use lazy_static for Runtime Initialization in Rust](https://oneuptime.com/blog/post/2026-01-25-rust-lazy-static/view) - Verified std::sync::LazyLock is now preferred
- [How to Use Collections (Vec, HashMap, HashSet) in Rust](https://oneuptime.com/blog/post/2026-02-01-rust-collections/view) - Collection performance characteristics
- [How to Handle Configuration with Config-rs in Rust](https://oneuptime.com/blog/post/2026-02-01-rust-config-rs-configuration/view) - Config-rs overkill for Phase 5
- [notify crate documentation](https://docs.rs/notify/) - File watching for hot-reload (Phase 7+)
- [cross-xdg crate](https://docs.rs/cross-xdg/latest/cross_xdg/) - XDG config dirs if moving from project-local
- [Rust file reading](https://blog.logrocket.com/how-to-read-files-rust/) - std::fs::read_to_string patterns
- [JSON Schema best practices](https://jsonconsole.com/blog/json-best-practices-writing-clean-maintainable-data-structures) - Whitelist validation approach

### Tertiary (LOW confidence)

- [VST3 plugin classId discussion](https://forum.juce.com/t/how-to-get-vst3-class-id-aka-cid-aka-component-id/41041) - ClassID is 128-bit TUID hex string
- [Rust hot-reloading examples](https://github.com/junkurihara/rust-hot-reloader) - File watching patterns (not verified for this codebase)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - serde_json and stdlib collections are proven, already in use
- Architecture: HIGH - Extends existing `list_params` logic, minimal changes required
- Pitfalls: MEDIUM - Identified from first principles, not verified with integration tests yet

**Research date:** 2026-02-15
**Valid until:** 60 days (stable domain, serde and stdlib don't change frequently)

**Dependencies on prior phases:**
- Phase 4: Requires `list_params` MCP tool (exists, verified in 04-01-PLAN.md)
- Phase 3: Requires parameter enumeration and filtering (exists, verified in 03-RESEARCH.md)
- Phase 1: Requires plugin classId extraction (exists, PluginInfo.uid field)

**Blocks future phases:**
- Phase 6: State management could save Focus Mode config alongside presets
- Phase 7+: Real-time DAW integration needs file watching for hot-reload
