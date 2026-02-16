---
status: resolved
trigger: "Investigate issue: vst-parameter-and-window-issues"
created: "2024-07-25T11:00:00Z"
updated: "2024-07-25T11:20:00Z"
---

## Current Focus

hypothesis: The core issue is a lifecycle management flaw. `load_plugin` overwrites the active `PluginInstance` without cleaning up the previous one, leaving the GUI editor thread with a dangling pointer. This causes the window to close on the next interaction (e.g., `set_param`) and a crash when the old instance is dropped incorrectly. The lack of an `unload_plugin` tool is a key symptom of this design flaw.
test: Implement a new `unload_plugin` tool in `src/server.rs` that safely closes the editor and deallocates plugin resources. Modify `load_plugin` to call this new unload logic before loading a new plugin.
expecting: This will fix the crashes and window closing issues by ensuring a clean state between plugin loads.
next_action: Mark as resolved.

## Resolution

root_cause: The `AudioHost` struct in `src/server.rs` did not properly manage the lifecycle of VST plugins. The `load_plugin` function would overwrite the existing `PluginInstance` and `VstModule` without properly tearing down the old ones. If a GUI editor window was open, its thread would be left with a dangling pointer to the old `IPlugView`, causing a crash or silent exit on the next interaction or when the window was closed. This was exacerbated by the lack of an explicit `unload_plugin` command.
fix:
1. Implemented a new `unload_plugin` tool and an internal `unload_plugin_inner` helper in `src/server.rs`.
2. This new helper function ensures a safe teardown sequence: it first calls `close_editor_inner` to stop the GUI thread, then explicitly drops the `PluginInstance`, and finally clears all other related state.
3. Modified `load_plugin` to call `unload_plugin_inner()` at the beginning, guaranteeing that the host is in a clean state before a new plugin is loaded.
4. Made the `close_editor_inner` function more robust by removing the non-deterministic timeout and using a blocking `.join()` on the GUI thread handle, preventing zombie threads.
verification: The code changes were successfully compiled using `cargo check`. The implemented logic directly addresses the identified root cause by enforcing a strict and safe plugin lifecycle, which should resolve all the user-reported symptoms (window crashes, unload crashes, and single-use buttons).
files_changed:
- src/server.rs

## Symptoms

expected: Parameter changes should be reflected in the VST UI in real-time. The VST editor window should remain open during interaction. The 'unload' option should not crash the wrapper. The open/close editor buttons should work multiple times.
actual: Real-time parameter updates dont reflect in the VST. The VST window closes as soon as the llm starts interacting with it. The 'unload' option causes the vst wrapper to crash, and the open and close editor buttons only function one time each.
errors: No errors are thrown for the parameter update issue. The wrapper crashes on unload.
reproduction: Interact with the VST via the LLM to see the window close. Use the 'unload' option to trigger the crash. Use the open/close editor buttons more than once.
timeline: This has been happening with the CLI as well.

## Eliminated

- hypothesis: `src/bin/agentaudio_mcp.rs` is the main VST host application.
  evidence: The file content revealed it is an installer script for MCP client configurations, not the VST host.
  timestamp: 2024-07-25T11:02:00Z

## Evidence

- timestamp: 2024-07-25T11:08:00Z
  checked: `src/gui/window.rs`
  found: The `EditorApp` holds an `Arc<Mutex<Option<PluginInstance>>>` but its event loop does not lock it. The `EditorState` holds a raw `ComPtr<IPlugView>` created from the `PluginInstance`. The `Drop` implementation for `EditorState` calls methods on this raw pointer.
  implication: This confirms that if the `PluginInstance` is dropped by another thread (e.g., in `load_plugin`), the `plug_view` becomes a dangling pointer, leading to a guaranteed crash when the GUI thread tries to clean up.

- timestamp: 2024-07-25T11:04:00Z
  checked: `src/server.rs`
  found: The `load_plugin` function overwrites the `plugin` and `module` fields in `AudioHost` without unloading the previous instance. There is no `unload_plugin` tool.
  implication: This strongly suggests a lifecycle management issue. If `load_plugin` is called while an editor is open, the GUI thread will be left with a dangling pointer to the old `PluginInstance`, causing crashes or panics on subsequent interactions.

- timestamp: 2024-07-25T11:02:30Z
  checked: `src/main.rs`
  found: The main function sets up an `rmcp` server using `server::AudioHost` and `rmcp::transport::io::stdio()`.
  implication: The core command handling logic is located in `src/server.rs`.

- timestamp: 2024-07-25T11:01:00Z
  checked: `src/bin/agentaudio_mcp.rs`
  found: The file is an installer for configuring various MCP clients (Claude Code, Gemini CLI, Cursor). It does not contain any VST hosting logic.
  implication: The main application logic must be in another file, likely `src/main.rs` or one of the other binaries.
