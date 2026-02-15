use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Information about a discovered VST3 plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Human-readable plugin name.
    pub name: String,
    /// Plugin vendor/manufacturer.
    pub vendor: String,
    /// Unique class ID as a hex-encoded string (32 hex chars).
    pub uid: String,
    /// Plugin category (e.g., "Audio Module Class", "Fx|EQ").
    pub category: String,
    /// Plugin version string.
    pub version: String,
    /// Path to the .vst3 bundle on disk.
    pub path: PathBuf,
}

/// Information about a single audio or event bus.
#[derive(Debug, Clone)]
pub struct BusInfo {
    /// Human-readable bus name.
    pub name: String,
    /// Number of channels in this bus.
    pub channel_count: i32,
    /// Whether this is an audio or event bus.
    pub bus_type: BusType,
    /// Whether this bus is input or output.
    pub direction: BusDirection,
    /// Whether this bus is active by default.
    pub is_default_active: bool,
}

/// Type of a bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusType {
    Audio,
    Event,
}

/// Direction of a bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusDirection {
    Input,
    Output,
}

/// Information about a single automatable parameter.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    /// Unique parameter ID within the plugin.
    pub id: u32,
    /// Human-readable parameter name.
    pub title: String,
    /// Parameter unit label (e.g., "dB", "Hz", "%").
    pub units: String,
    /// Default value in normalized [0, 1] range.
    pub default_normalized: f64,
    /// Number of discrete steps (0 = continuous).
    pub step_count: i32,
    /// Parameter flags from the plugin.
    pub flags: u32,
}

impl ParamInfo {
    /// Check if this parameter can be written by the host.
    ///
    /// A writable parameter must have kCanAutomate flag AND NOT have kIsReadOnly flag.
    pub fn is_writable(&self) -> bool {
        const K_CAN_AUTOMATE: u32 = 1 << 0;
        const K_IS_READ_ONLY: u32 = 1 << 1;

        (self.flags & K_CAN_AUTOMATE != 0) && (self.flags & K_IS_READ_ONLY == 0)
    }

    /// Check if this parameter should be hidden from UI.
    pub fn is_hidden(&self) -> bool {
        const K_IS_HIDDEN: u32 = 1 << 5;
        self.flags & K_IS_HIDDEN != 0
    }

    /// Check if this is a bypass parameter.
    pub fn is_bypass(&self) -> bool {
        const K_IS_BYPASS: u32 = 1 << 4;
        self.flags & K_IS_BYPASS != 0
    }

    /// Check if this parameter is read-only.
    pub fn is_read_only(&self) -> bool {
        const K_IS_READ_ONLY: u32 = 1 << 1;
        self.flags & K_IS_READ_ONLY != 0
    }
}

/// Plugin lifecycle state.
///
/// VST3 plugins must transition through these states in order:
/// Created -> SetupDone -> Active -> Processing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is instantiated and initialized, but not yet set up for processing.
    Created,
    /// Plugin has been set up with sample rate and block size, buses activated.
    SetupDone,
    /// Plugin is active (setActive(true) called), ready to start processing.
    Active,
    /// Plugin is actively processing audio (setProcessing(true) called).
    Processing,
}

/// Errors from the VST3 hosting layer.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("failed to load VST3 module: {0}")]
    ModuleLoadFailed(String),

    #[error("factory error: {0}")]
    FactoryError(String),

    #[error("plugin initialization failed: {0}")]
    InitializeFailed(String),

    #[error("plugin setup failed: {0}")]
    SetupFailed(String),

    #[error("plugin activation failed: {0}")]
    ActivationFailed(String),

    #[error("processing error: {0}")]
    ProcessingFailed(String),

    #[error("preset error: {0}")]
    PresetError(String),

    #[error("scan error: {0}")]
    ScanError(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
