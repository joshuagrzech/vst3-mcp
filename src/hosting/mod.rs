//! VST3 plugin hosting layer.
//!
//! This module contains all unsafe COM code for loading, scanning,
//! and managing VST3 plugins. No unsafe blocks should exist outside
//! this module.

pub mod host_app;
pub mod module;
pub mod param_changes;
pub mod plugin;
pub mod scanner;
pub mod types;
