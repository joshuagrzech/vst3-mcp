//! IPlugFrame COM implementation for VST3 plugin editor hosting.
//!
//! IPlugFrame is passed to the plugin's IPlugView via setFrame(). The plugin
//! uses it for resize requests and (on Linux) to obtain IRunLoop for event
//! dispatch via queryInterface.
//!
//! Since the vst3 crate's `Class` trait auto-generates queryInterface to only
//! return interfaces listed in `type Interfaces`, and we need IPlugFrame to
//! also respond to IRunLoop queries, we implement a combined struct that
//! lists both interfaces.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::debug;
use vst3::com_scrape_types::ComWrapper;
use vst3::Steinberg::Linux::IRunLoop;
use vst3::Steinberg::{
    kResultOk, tresult, IPlugFrame, IPlugFrameTrait, IPlugView, ViewRect,
};

use super::runloop::HostRunLoop;

// #region agent log
fn agent_log(
    run_id: &str,
    hypothesis_id: &str,
    location: &str,
    message: &str,
    data: serde_json::Value,
) {
    use std::io::Write;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let payload = serde_json::json!({
        "id": format!("log_{}_{}", std::process::id(), ts),
        "timestamp": ts,
        "location": location,
        "message": message,
        "data": data,
        "runId": run_id,
        "hypothesisId": hypothesis_id,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/home/josh/Developer/vst3-mcp/.cursor/debug.log")
    {
        let _ = writeln!(f, "{}", payload);
    }
}
// #endregion

/// Combined IPlugFrame + IRunLoop COM object.
///
/// The plugin obtains IRunLoop by calling queryInterface on the IPlugFrame
/// pointer passed via IPlugView::setFrame(). By implementing both interfaces
/// on the same COM object, queryInterface automatically returns the correct
/// vtable for IRunLoop queries.
pub struct PlugFrame {
    /// Shared reference to the run loop for timer/FD dispatch.
    /// Kept as Arc so the event loop can also access it.
    _runloop_ref: Arc<HostRunLoop>,
}

impl PlugFrame {
    /// Create a new PlugFrame with an associated HostRunLoop.
    ///
    /// The `runloop` Arc is shared with the event loop so both the plugin
    /// (via IRunLoop) and the host (via dispatch methods) can access it.
    pub fn new(runloop: Arc<HostRunLoop>) -> ComWrapper<Self> {
        ComWrapper::new(PlugFrame {
            _runloop_ref: runloop,
        })
    }
}

impl vst3::Class for PlugFrame {
    type Interfaces = (IPlugFrame, IRunLoop);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(
        &self,
        _view: *mut IPlugView,
        new_size: *mut ViewRect,
    ) -> tresult {
        if !new_size.is_null() {
            let rect = unsafe { &*new_size };
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;
            debug!("IPlugFrame::resizeView requested: {}x{}", w, h);
            // #region agent log
            agent_log(
                "pre-fix",
                "H11",
                "src/gui/plugframe.rs:PlugFrame::resizeView",
                "plugin requested resize via IPlugFrame::resizeView",
                serde_json::json!({
                    "w": w,
                    "h": h,
                    "left": rect.left,
                    "top": rect.top,
                    "right": rect.right,
                    "bottom": rect.bottom,
                }),
            );
            // #endregion
        }
        // Accept all resize requests for now (fixed-size window in Phase 04.1)
        kResultOk
    }
}

impl vst3::Steinberg::Linux::IRunLoopTrait for PlugFrame {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut vst3::Steinberg::Linux::IEventHandler,
        fd: vst3::Steinberg::Linux::FileDescriptor,
    ) -> tresult {
        unsafe { self._runloop_ref.registerEventHandler(handler, fd) }
    }

    unsafe fn unregisterEventHandler(
        &self,
        handler: *mut vst3::Steinberg::Linux::IEventHandler,
    ) -> tresult {
        unsafe { self._runloop_ref.unregisterEventHandler(handler) }
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut vst3::Steinberg::Linux::ITimerHandler,
        milliseconds: vst3::Steinberg::Linux::TimerInterval,
    ) -> tresult {
        unsafe { self._runloop_ref.registerTimer(handler, milliseconds) }
    }

    unsafe fn unregisterTimer(
        &self,
        handler: *mut vst3::Steinberg::Linux::ITimerHandler,
    ) -> tresult {
        unsafe { self._runloop_ref.unregisterTimer(handler) }
    }
}
