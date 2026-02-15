//! GUI module for VST3 plugin editor window hosting.
//!
//! Provides window creation, IPlugView lifecycle management,
//! IPlugFrame/IRunLoop COM implementations, and XEmbed protocol
//! support for embedding plugin editor GUIs on Linux X11.

pub mod plugframe;
pub mod runloop;
pub mod window;
pub mod xembed;

pub use window::open_editor_window;
