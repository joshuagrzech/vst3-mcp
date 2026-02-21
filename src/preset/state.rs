//! Plugin state save/restore via getState/setState.
//!
//! Bridges between plugin instances and .vstpreset files.
//! Handles the critical component+controller state sync required by the VST3 spec.

use std::path::Path;

use vst3::Steinberg::IBStream;
use vst3::Steinberg::Vst::IComponentTrait;
use vst3::Steinberg::Vst::IEditControllerTrait;
use vst3::Steinberg::kResultOk;

use super::vstpreset;
use crate::hosting::plugin::{PluginInstance, VecStream};
use crate::hosting::types::HostError;

/// Save the current state of a plugin instance to a .vstpreset file.
///
/// Captures both component (processor) state and controller state,
/// then writes them to the Steinberg .vstpreset binary format.
pub fn save_plugin_state(plugin: &PluginInstance, path: &Path) -> Result<(), HostError> {
    // Get component state
    let comp_stream = VecStream::new();
    let comp_ptr = comp_stream.to_com_ptr::<IBStream>().ok_or_else(|| {
        HostError::PresetError("failed to create IBStream for component state".to_string())
    })?;

    unsafe {
        let result = plugin.component().getState(comp_ptr.as_ptr());
        if result != kResultOk {
            return Err(HostError::PresetError(format!(
                "component.getState failed with code {}",
                result
            )));
        }
    }

    // Get controller state (optional -- some plugins don't have separate controller state)
    let ctrl_data = if let Some(ctrl) = plugin.controller() {
        let ctrl_stream = VecStream::new();
        let ctrl_ptr = ctrl_stream.to_com_ptr::<IBStream>().ok_or_else(|| {
            HostError::PresetError("failed to create IBStream for controller state".to_string())
        })?;

        unsafe {
            let result = ctrl.getState(ctrl_ptr.as_ptr());
            if result == kResultOk {
                let data = ctrl_stream.data().to_vec();
                if data.is_empty() { None } else { Some(data) }
            } else {
                // Not all plugins support separate controller state
                None
            }
        }
    } else {
        None
    };

    // Convert class ID (TUID) to 32-byte ASCII hex
    let class_id = tuid_to_ascii_hex(plugin.class_id());

    let comp_data = comp_stream.data().to_vec();

    vstpreset::save_preset(path, &class_id, &comp_data, ctrl_data.as_deref())
}

/// Restore plugin state from a .vstpreset file.
///
/// Loads the preset data and applies it to both the component and controller,
/// following the VST3 spec requirement to call both setState and setComponentState.
pub fn restore_plugin_state(plugin: &mut PluginInstance, path: &Path) -> Result<(), HostError> {
    let preset = vstpreset::load_preset(path)?;

    // Validate class ID matches
    let expected_id = tuid_to_ascii_hex(plugin.class_id());
    if preset.class_id != expected_id {
        return Err(HostError::PresetError(format!(
            "class ID mismatch: preset has {:?}, plugin has {:?}",
            String::from_utf8_lossy(&preset.class_id),
            String::from_utf8_lossy(&expected_id),
        )));
    }

    // Apply component state
    let comp_stream = VecStream::from_data(preset.component_state.clone());
    let comp_ptr = comp_stream.to_com_ptr::<IBStream>().ok_or_else(|| {
        HostError::PresetError("failed to create IBStream for component state".to_string())
    })?;

    unsafe {
        let result = plugin.component().setState(comp_ptr.as_ptr());
        if result != kResultOk {
            return Err(HostError::PresetError(format!(
                "component.setState failed with code {}",
                result
            )));
        }
    }

    // CRITICAL (Pitfall 6): After component.setState(), ALWAYS call
    // controller.setComponentState() with the SAME data to sync controller.
    if let Some(ctrl) = plugin.controller() {
        let sync_stream = VecStream::from_data(preset.component_state);
        let sync_ptr = sync_stream.to_com_ptr::<IBStream>().ok_or_else(|| {
            HostError::PresetError("failed to create IBStream for sync".to_string())
        })?;

        unsafe {
            let result = ctrl.setComponentState(sync_ptr.as_ptr());
            if result != kResultOk {
                // Some plugins don't implement setComponentState -- log but don't fail
                tracing::warn!("controller.setComponentState returned {}", result);
            }
        }

        // Apply controller-specific state if present
        if let Some(ctrl_state) = preset.controller_state {
            let ctrl_stream = VecStream::from_data(ctrl_state);
            let ctrl_ptr = ctrl_stream.to_com_ptr::<IBStream>().ok_or_else(|| {
                HostError::PresetError("failed to create IBStream for controller state".to_string())
            })?;

            unsafe {
                let result = ctrl.setState(ctrl_ptr.as_ptr());
                if result != kResultOk {
                    tracing::warn!("controller.setState returned {}", result);
                }
            }
        }
    }

    Ok(())
}

/// Convert a 16-byte TUID to a 32-byte ASCII hex string.
fn tuid_to_ascii_hex(tuid: &[i8; 16]) -> [u8; 32] {
    let mut hex = [0u8; 32];
    for (i, &byte) in tuid.iter().enumerate() {
        let b = byte as u8;
        let hi = b >> 4;
        let lo = b & 0x0F;
        hex[i * 2] = if hi < 10 { b'0' + hi } else { b'A' + hi - 10 };
        hex[i * 2 + 1] = if lo < 10 { b'0' + lo } else { b'A' + lo - 10 };
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tuid_to_ascii_hex() {
        let tuid: [i8; 16] = [
            0x01,
            0x23,
            0x45,
            0x67,
            0x89u8 as i8,
            0xABu8 as i8,
            0xCDu8 as i8,
            0xEFu8 as i8,
            0x00,
            0x11,
            0x22,
            0x33,
            0x44,
            0x55,
            0x66,
            0x77,
        ];
        let hex = tuid_to_ascii_hex(&tuid);
        assert_eq!(&hex, b"0123456789ABCDEF0011223344556677");
    }

    #[test]
    fn test_tuid_to_ascii_hex_zeros() {
        let tuid: [i8; 16] = [0; 16];
        let hex = tuid_to_ascii_hex(&tuid);
        assert_eq!(&hex, b"00000000000000000000000000000000");
    }
}
