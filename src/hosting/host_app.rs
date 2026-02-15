//! IHostApplication and IComponentHandler COM implementations.
//!
//! These are the host-side COM interfaces that plugins require
//! during initialization and operation.

use std::ffi::c_void;

use tracing::debug;
use vst3::com_scrape_types::{Class, ComWrapper};
use vst3::Steinberg::Vst::{
    IComponentHandler, IComponentHandlerTrait, IHostApplication, IHostApplicationTrait,
    ParamID, ParamValue, String128,
};
use vst3::Steinberg::{
    kNotImplemented, kResultOk, tresult, FUnknown, IPluginBase, IPluginBaseTrait, TUID,
};

/// Host application identity, implementing IHostApplication.
///
/// Plugins query this during initialization to identify the host.
pub struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication, IPluginBase);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kResultOk;
        }

        let host_name = "VST3 MCP Host";
        let name_ref = unsafe { &mut *name };

        // Write UTF-16LE encoded name into the String128 buffer
        for (i, ch) in host_name.encode_utf16().enumerate() {
            if i >= 127 {
                break;
            }
            name_ref[i] = ch;
        }
        // Null terminate
        let len = host_name.encode_utf16().count().min(127);
        name_ref[len] = 0;

        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        _iid: *mut TUID,
        _obj: *mut *mut c_void,
    ) -> tresult {
        // Not implemented for Phase 1 -- no message passing needed yet.
        kNotImplemented
    }
}

impl IPluginBaseTrait for HostApp {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

/// Component handler implementing IComponentHandler.
///
/// Handles callbacks from the plugin's edit controller for parameter
/// changes and restart requests.
pub struct ComponentHandler;

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        debug!("beginEdit: param_id={}", id);
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        debug!("performEdit: param_id={}, value={}", id, value);
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        debug!("endEdit: param_id={}", id);
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: i32) -> tresult {
        debug!("restartComponent: flags=0x{:X}", flags);
        // For Phase 1, we just log restart requests.
        // Common flags: kLatencyChanged (0x100), kParamValuesChanged (0x01)
        kResultOk
    }
}

impl HostApp {
    /// Create a new HostApp wrapped in a ComWrapper for passing to plugins.
    pub fn new() -> ComWrapper<HostApp> {
        ComWrapper::new(HostApp)
    }
}

impl ComponentHandler {
    /// Create a new ComponentHandler wrapped in a ComWrapper.
    pub fn new() -> ComWrapper<ComponentHandler> {
        ComWrapper::new(ComponentHandler)
    }
}
