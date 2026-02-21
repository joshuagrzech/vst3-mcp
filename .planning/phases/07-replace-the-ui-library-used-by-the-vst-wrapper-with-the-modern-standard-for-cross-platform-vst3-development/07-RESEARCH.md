# Phase 7: Replace UI Library (nih_plug_egui -> nih_plug_vizia) - Research

**Researched:** 2026-02-21
**Domain:** nih-plug GUI framework migration (nih_plug_egui to nih_plug_vizia)
**Confidence:** HIGH

## Summary

The project currently uses `nih_plug_egui` in the VST3 wrapper plugin (`crates/agentaudio-wrapper-vst3/src/lib.rs`) to implement the wrapper's `editor()` method. The nih_plug_egui README explicitly states "Consider using `nih_plug_iced` or `nih_plug_vizia` instead," making this a soft-deprecated interface. The modern standard for cross-platform VST3 plugin UIs in nih-plug is `nih_plug_vizia`, which is what the nih-plug production plugins use and which has a richer widget ecosystem.

The migration scope is narrow: only the `editor()` method and related `WrapperParams::editor_state` in the wrapper crate need to change. The standalone host GUI (`src/gui/app.rs` using `eframe`) and the plugin editor window system (`src/gui/window.rs` using `winit`) are separate subsystems and are NOT affected by this phase.

The key differences between egui and vizia in the nih-plug context: egui uses an immediate-mode model (closure called every frame), while vizia uses a retained/reactive data model (Lens-based data binding). The migration requires replacing `EguiState` with `ViziaState`, `create_egui_editor` with `create_vizia_editor`, and rewriting the UI layout using Vizia's builder API instead of egui's immediate-mode calls.

**Primary recommendation:** Replace `nih_plug_egui` with `nih_plug_vizia` in the wrapper crate, keeping the UI functionality identical (instance display, MCP endpoint, plugin path input, load/unload, open/close editor, status message).

## Current State Analysis

### What Exists Now (wrapper crate)

```
crates/agentaudio-wrapper-vst3/Cargo.toml:
  nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git" }

crates/agentaudio-wrapper-vst3/src/lib.rs:
  use nih_plug_egui::{EguiState, create_egui_editor, egui};

  #[derive(Params)]
  struct WrapperParams {
      #[persist = "editor-state"]
      editor_state: Arc<EguiState>,       // <-- changes to Arc<ViziaState>
  }

  fn editor(&mut self, ...) -> Option<Box<dyn Editor>> {
      create_egui_editor(                  // <-- changes to create_vizia_editor
          self.params.editor_state.clone(),
          (),
          |_, _| {},
          move |ctx, _setter, _state| {
              egui::Window::new("AgentAudio Wrapper").show(ctx, |ui| {
                  // immediate-mode egui UI
              });
          },
      )
  }
```

### What Is NOT Changing

- `src/gui/app.rs` — standalone host GUI using `eframe` (unrelated to plugin format)
- `src/gui/window.rs` — plugin editor embedding using `winit` (unrelated to wrapper UI)
- `src/bin/vst3-gui.rs` — standalone binary entry point
- The `eframe` dependency in the root `Cargo.toml`

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| nih_plug_vizia | git (nih-plug repo) | Vizia GUI adapter for nih-plug | Explicitly recommended over egui by nih-plug maintainers; used by all production plugins in nih-plug repo |
| vizia | git (robbert-vdh/vizia, patched-2024-05-06 tag) | Retained-mode GUI framework | Bundled via nih_plug_vizia; do not add separately |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| atomic_float | 0.1 | AtomicF32 for shared state | Only if adding peak meters or similar |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| nih_plug_vizia | nih_plug_egui | egui is softer-deprecated; maintainers direct to vizia/iced |
| nih_plug_vizia | nih_plug_iced | iced is less commonly used in nih-plug examples; vizia has more nih-plug-specific widgets |

**Installation (Cargo.toml change in wrapper crate):**
```toml
# Remove:
nih_plug_egui = { git = "https://github.com/robbert-vdh/nih-plug.git" }

# Add:
nih_plug_vizia = { git = "https://github.com/robbert-vdh/nih-plug.git" }
```

## Architecture Patterns

### Recommended Project Structure

The wrapper crate only has `src/lib.rs`. The migration is entirely within that file plus `Cargo.toml`.

```
crates/agentaudio-wrapper-vst3/
├── Cargo.toml           # swap nih_plug_egui -> nih_plug_vizia
└── src/
    └── lib.rs           # swap EguiState->ViziaState, create_egui_editor->create_vizia_editor
```

### Pattern 1: ViziaState Declaration

**What:** Replace `EguiState` with `ViziaState` in params struct.
**When to use:** Always when migrating from egui to vizia.

```rust
// Source: https://nih-plug.robbertvanderhelm.nl/nih_plug_vizia/index.html
use nih_plug_vizia::{ViziaState, ViziaTheming, create_vizia_editor, vizia::prelude::*};

#[derive(Params)]
struct WrapperParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,
}

impl Default for WrapperParams {
    fn default() -> Self {
        Self {
            editor_state: ViziaState::from_size(560, 420),
        }
    }
}
```

### Pattern 2: Lens-Based Data Sharing

**What:** Vizia uses a Lens/data binding model instead of egui closures. Shared state must be wrapped in a Lens-derivable struct.
**When to use:** When the editor closure needs access to `SharedState` or `GuiState`.

```rust
// Source: nih-plug gain_gui_vizia example
// https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/gain_gui_vizia/src/editor.rs

#[derive(Lens)]
struct EditorData {
    shared: SharedState,         // Clone-able shared state
    gui_state: Arc<Mutex<GuiState>>,
}

impl Model for EditorData {}
```

### Pattern 3: create_vizia_editor Call

**What:** The editor creation function takes ViziaState, theming enum, and a closure receiving `&mut Context`.
**When to use:** In the `Plugin::editor()` method.

```rust
// Source: https://nih-plug.robbertvanderhelm.nl/nih_plug_vizia/index.html
fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
    let shared = self.shared.clone();
    let gui_state = Arc::clone(&self.gui_state);

    create_vizia_editor(
        self.params.editor_state.clone(),
        ViziaTheming::Custom,    // applies nih_plug_vizia default theming
        move |cx, _gui_context| {
            EditorData {
                shared: shared.clone(),
                gui_state: Arc::clone(&gui_state),
            }
            .build(cx);

            // UI layout goes here
            VStack::new(cx, |cx| {
                Label::new(cx, "AgentAudio Wrapper");
                // ... more widgets
            });
        },
    )
}
```

### Pattern 4: Vizia Widget Layout

**What:** Vizia uses a builder pattern with layout modifiers instead of egui's immediate-mode.
**When to use:** For all UI elements.

```rust
// Source: nih-plug gain_gui_vizia/src/editor.rs
VStack::new(cx, |cx| {
    Label::new(cx, EditorData::shared.map(|s| s.instance_id.to_string()));
    Label::new(cx, "MCP endpoint:");
    Label::new(cx, EditorData::shared.map(|s| {
        s.endpoint().unwrap_or_else(|| "starting...".into())
    }));

    // Text input
    Textbox::new(cx, EditorData::gui_state.map(|gs| {
        gs.lock().ok().map(|g| g.plugin_path.clone()).unwrap_or_default()
    }))
    .on_submit(|cx, val, _| { /* update shared state */ });

    // Buttons
    HStack::new(cx, |cx| {
        Button::new(cx, |cx| Label::new(cx, "Load"))
            .on_press(move |cx| { /* load action */ });
        Button::new(cx, |cx| Label::new(cx, "Unload"))
            .on_press(move |cx| { /* unload action */ });
    });

    // ResizeHandle MUST be last
    ResizeHandle::new(cx);
});
```

### Pattern 5: Checking Editor Open State

**What:** `ViziaState::is_open()` replaces `EguiState::is_open()`.
**When to use:** In `process()` to skip expensive calculations when GUI is closed.

```rust
// Before (egui):
if params.editor_state.is_open() { ... }

// After (vizia):
if params.editor_state.is_open() { ... }
// API is identical - same method name, same return type
```

### Anti-Patterns to Avoid

- **Placing ResizeHandle before other elements:** Vizia's event targeting changed; ResizeHandle MUST be the last element declared in the GUI. Placing it earlier causes ResizeHandle events to shadow other widget interactions.
- **Adding `vizia` as a direct dependency:** `nih_plug_vizia` re-exports `vizia`; adding it separately risks version conflicts since nih-plug uses a custom-patched git fork, not the crates.io version.
- **Trying to use egui-style closures:** Vizia is not immediate-mode. State must be shared via `Lens`-derived structs and `Model`, not closure captures updated every frame.
- **Calling `ViziaState::from_size` with the same field name if serialized:** The persist key `"editor-state"` is fine but must match the old key if you want to preserve user window positions across the migration.
- **Mutating shared state directly in button handlers:** In Vizia, event emission is preferred. For a simple wrapper with Mutex-guarded state, direct Mutex locking in `on_press` closures is acceptable since these are not on the audio thread.
- **Registering fonts when using `ViziaTheming::Custom`:** The Custom theming registers Noto Sans Light automatically. Don't register it again.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Editor open/close tracking | Custom AtomicBool | `ViziaState::is_open()` | Built into ViziaState, persisted correctly |
| Window size persistence | Manual serialization | `#[persist = "editor-state"]` on `Arc<ViziaState>` | nih-plug handles serialize/deserialize |
| Parameter sliders | Custom slider widget | `ParamSlider::new(cx, params, \|p\| &p.some_param)` | Handles DAW automation, value display, reset |
| Font registration | `cx.add_font_memory(...)` | `ViziaTheming::Custom` | Auto-registers Noto Sans Light |

**Key insight:** nih_plug_vizia handles all plugin-format integration concerns (window creation, resize, scaling, DAW automation). The only code needed is declarative UI layout.

## Common Pitfalls

### Pitfall 1: ResizeHandle Position
**What goes wrong:** ResizeHandle in wrong position causes other widgets to not receive click/drag events.
**Why it happens:** Vizia's event targeting changed in 2023-12-30 overhaul; earlier elements shadow later ones.
**How to avoid:** Always declare `ResizeHandle::new(cx)` as the LAST element in the outermost container.
**Warning signs:** Buttons stop responding to clicks; sliders don't drag.

### Pitfall 2: Vizia Font Handling Changed
**What goes wrong:** Custom font registration code from old Vizia examples causes compile errors or incorrect theming.
**Why it happens:** "Font handling and choosing between different variations of the same font (e.g. Noto Sans versus Noto Sans Light) works very differently now." (CHANGELOG 2023-12-30)
**How to avoid:** Use `ViziaTheming::Custom` and do not manually register fonts. Only register fonts if using `ViziaTheming::None`.
**Warning signs:** Compile errors on `cx.add_font_memory`, font not found panics at runtime.

### Pitfall 3: Multi-Instance Crashes (Windows/macOS)
**What goes wrong:** Opening multiple instances of the wrapper plugin crashes the DAW on Windows or macOS.
**Why it happens:** Known Vizia issue; nih_plug_vizia includes a workaround patch (patched-2024-05-06).
**How to avoid:** Use the git dependency pointing to nih-plug's repo (which includes the patch). Do NOT pin to the crates.io vizia directly.
**Warning signs:** Crashes when loading second instance; only affects non-Linux platforms.

### Pitfall 4: Lens on Non-Clone Types
**What goes wrong:** `#[derive(Lens)]` fails to compile on struct fields that don't implement Clone.
**Why it happens:** Vizia's Lens derive requires Clone on the struct.
**How to avoid:** Wrap non-Clone shared state in `Arc<Mutex<...>>` (already done in this project's `SharedState`). The `EditorData` struct must be Clone.
**Warning signs:** Compile error on `#[derive(Lens)]` mentioning Clone.

### Pitfall 5: Textbox State Update Pattern
**What goes wrong:** `Textbox` value doesn't reflect external state updates (e.g., if path is changed from MCP tool while editor is open).
**Why it happens:** Vizia's retained model means you need proper data binding, not one-off closure captures.
**How to avoid:** Bind textbox to a `Lens` mapping that reads from the current state. Use `on_submit` or `on_edit` for user input. For a simple wrapper, a Mutex-guarded `GuiState` struct read via lens is sufficient.
**Warning signs:** Textbox shows stale value; typing doesn't update underlying state.

## Code Examples

Verified patterns from official sources:

### Minimal create_vizia_editor Pattern
```rust
// Source: https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/gain_gui_vizia/src/lib.rs
// and https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/gain_gui_vizia/src/editor.rs

use nih_plug_vizia::{ViziaState, ViziaTheming, create_vizia_editor, vizia::prelude::*};

fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
    let shared = self.shared.clone();
    let gui_state = Arc::clone(&self.gui_state);

    create_vizia_editor(
        self.params.editor_state.clone(),
        ViziaTheming::Custom,
        move |cx, _gui_context| {
            // Bind data model
            EditorData {
                shared: shared.clone(),
                gui_state: Arc::clone(&gui_state),
            }
            .build(cx);

            // Layout
            VStack::new(cx, |cx| {
                // ... widgets using EditorData lens
                ResizeHandle::new(cx); // MUST be last
            });
        },
    )
}
```

### ViziaState in WrapperParams
```rust
// Source: https://nih-plug.robbertvanderhelm.nl/nih_plug_vizia/index.html
use nih_plug_vizia::ViziaState;

#[derive(Params)]
struct WrapperParams {
    #[persist = "editor-state"]
    editor_state: Arc<ViziaState>,
}

impl Default for WrapperParams {
    fn default() -> Self {
        Self {
            editor_state: ViziaState::from_size(560, 420),
        }
    }
}
```

### Lens-Derived EditorData
```rust
// Source: gain_gui_vizia example pattern
#[derive(Lens, Clone)]
struct EditorData {
    shared: SharedState,                   // must implement Clone
    gui_state: Arc<Mutex<GuiState>>,       // Arc<Mutex<T>> is Clone
}

impl Model for EditorData {}
```

### ParamSlider (if nih-plug params are needed in the future)
```rust
// Source: https://nih-plug.robbertvanderhelm.nl/nih_plug_vizia/widgets/index.html
ParamSlider::new(cx, EditorData::params, |params| &params.some_param);
```

### is_open() Check (unchanged API)
```rust
// Both EguiState and ViziaState expose identical is_open() method
// Source: nih_plug_vizia API docs
if self.params.editor_state.is_open() {
    // perform expensive GUI-visible calculations
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| nih_plug_egui | nih_plug_vizia | nih-plug README (pre-2024) | Vizia has richer parameter widgets, is actively maintained as the primary GUI |
| EguiState::from_size | ViziaState::from_size | Same API shape | Drop-in signature replacement |
| create_egui_editor | create_vizia_editor | Same API shape | Closure signature differs (no user state, no separate build vs update) |
| Immediate-mode egui UI | Retained Vizia Lens UI | N/A | Requires rewrite of UI layout code |

**Deprecated/outdated:**
- `nih_plug_egui`: README says "Consider using nih_plug_iced or nih_plug_vizia instead." Still compiles and works but not the recommended path.
- `egui 0.31` (in root Cargo.toml via eframe): Standalone host GUI only; completely separate from wrapper plugin UI.

## Open Questions

1. **Does `SharedState` implement `Clone`?**
   - What we know: `SharedState` contains `Arc<...>` fields which make it Clone-able by derivation.
   - What's unclear: Need to verify `#[derive(Clone)]` is on SharedState or can be added.
   - Recommendation: Check and add `#[derive(Clone)]` to `SharedState` if not present. All fields are `Arc<...>` so this is safe.

2. **Is ResizeHandle wanted in this wrapper UI?**
   - What we know: The current egui UI uses a floating `egui::Window` (which is moveable/resizable by default). Vizia's ResizeHandle provides explicit resize behavior.
   - What's unclear: Whether users need to resize the wrapper's own UI.
   - Recommendation: Include `ResizeHandle::new(cx)` at the bottom (it's the standard for nih-plug vizia editors); omit only if the wrapper UI should be fixed-size.

3. **Textbox widget availability in nih_plug_vizia**
   - What we know: Vizia has `Textbox` in its widget set. The nih_plug_vizia wrapper should expose it.
   - What's unclear: The exact import path (`vizia::prelude::*` should cover it).
   - Recommendation: Use `Textbox::new(cx, lens).on_submit(...)` for the plugin path field. Verify it compiles; if not, use `Label` + a manual text entry solution.

## Sources

### Primary (HIGH confidence)
- `https://nih-plug.robbertvanderhelm.nl/nih_plug_vizia/index.html` — ViziaState, ViziaTheming, create_vizia_editor API
- `https://nih-plug.robbertvanderhelm.nl/nih_plug_vizia/widgets/index.html` — Widget inventory (ParamSlider, ParamButton, PeakMeter, GenericUi, ResizeHandle)
- `https://github.com/robbert-vdh/nih-plug/blob/master/nih_plug_egui/README.md` — Deprecation recommendation: "Consider using nih_plug_iced or nih_plug_vizia instead"
- `https://github.com/robbert-vdh/nih-plug/blob/master/plugins/examples/gain_gui_vizia/src/editor.rs` — Reference implementation pattern
- `https://github.com/robbert-vdh/nih-plug/blob/master/CHANGELOG.md` — Breaking changes: ResizeHandle position, font handling, multi-instance fix

### Secondary (MEDIUM confidence)
- `https://github.com/vizia/vizia-plug` — Community vizia-plug example (alternative to nih_plug_vizia, confirms API patterns)
- `https://github.com/robbert-vdh/nih-plug/blob/master/nih_plug_vizia/src/lib.rs` — create_vizia_editor signature verified via WebFetch
- `https://nih-plug.robbertvanderhelm.nl/nih_plug_egui/index.html` — EguiState API (for comparison)

### Tertiary (LOW confidence)
- WebSearch results confirming community usage of vizia over egui for new projects (unverified claim counts)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — nih_plug_egui README explicitly recommends vizia; nih-plug production plugins use vizia
- Architecture: HIGH — API signatures verified via official docs; code examples from official repo
- Pitfalls: HIGH for ResizeHandle/fonts (CHANGELOG-cited); MEDIUM for Textbox/Lens/Clone specifics (inferred from vizia model)

**Research date:** 2026-02-21
**Valid until:** 2026-03-21 (nih-plug uses git deps; stability is moderate; recheck if nih-plug repo has major commits)
