//! GUI module for VST3 plugin editor window hosting.
//!
//! On Linux: Full implementation with window creation, IPlugView lifecycle,
//! IPlugFrame/IRunLoop COM implementations, and XEmbed protocol for X11 embedding.
//!
//! On Windows/macOS: Stub implementations that return a "not yet implemented"
//! error. Phase B will add native HWND/NSView embedding and platform run loops.

#[cfg(target_os = "linux")]
pub mod plugframe;
#[cfg(target_os = "linux")]
pub mod runloop;
#[cfg(target_os = "linux")]
pub mod window;
#[cfg(target_os = "linux")]
pub mod xembed;

#[cfg(target_os = "linux")]
pub use window::{open_editor_window, open_editor_window_persistent};

#[cfg(not(target_os = "linux"))]
use std::sync::{Arc, RwLock};
#[cfg(not(target_os = "linux"))]
use std::sync::atomic::AtomicBool;

#[cfg(not(target_os = "linux"))]
use crate::hosting::plugin::PluginInstance;

/// Open the plugin's editor window (stub on non-Linux).
#[cfg(not(target_os = "linux"))]
pub fn open_editor_window(
    _plugin: Arc<std::sync::Mutex<Option<PluginInstance>>>,
    _plugin_name: String,
    opened_tx: std::sync::mpsc::Sender<Result<(), String>>,
    _close_signal: Arc<AtomicBool>,
) -> Result<(), String> {
    let msg = format!(
        "Plugin editor hosting is not yet implemented on {}. Use the plugin in a DAW for now.",
        std::env::consts::OS
    );
    let _ = opened_tx.send(Err(msg.clone()));
    Err(msg)
}

/// Open and maintain a persistent editor event loop (stub on non-Linux).
#[cfg(not(target_os = "linux"))]
pub fn open_editor_window_persistent(
    _plugin: Arc<std::sync::Mutex<Option<PluginInstance>>>,
    _plugin_name: Arc<RwLock<String>>,
    opened_tx: std::sync::mpsc::Sender<Result<(), String>>,
    _close_signal: Arc<AtomicBool>,
    _is_open: Arc<AtomicBool>,
    _exit_signal: Arc<AtomicBool>,
) -> Result<(), String> {
    let msg = format!(
        "Plugin editor hosting is not yet implemented on {}. Use the plugin in a DAW for now.",
        std::env::consts::OS
    );
    let _ = opened_tx.send(Err(msg.clone()));
    Err(msg)
}
