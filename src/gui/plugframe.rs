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

use std::sync::{Arc, Mutex};

use tracing::debug;
use vst3::com_scrape_types::ComWrapper;
use vst3::Steinberg::Linux::IRunLoop;
use vst3::Steinberg::{
    kResultOk, tresult, IPlugFrame, IPlugFrameTrait, IPlugView, ViewRect,
};

use super::runloop::HostRunLoop;

/// Pending resize request from the plugin (width, height in physical pixels).
pub type PendingResize = Arc<Mutex<Option<(u32, u32)>>>;

/// Combined IPlugFrame + IRunLoop COM object.
///
/// The plugin obtains IRunLoop by calling queryInterface on the IPlugFrame
/// pointer passed via IPlugView::setFrame(). By implementing both interfaces
/// on the same COM object, queryInterface automatically returns the correct
/// vtable for IRunLoop queries.
pub struct PlugFrame {
    /// Shared reference to the run loop for timer/FD dispatch.
    _runloop_ref: Arc<HostRunLoop>,
    /// Shared slot where resizeView deposits the requested size.
    /// The event loop picks it up and performs the actual resize + onSize.
    pending_resize: PendingResize,
}

impl PlugFrame {
    /// Create a new PlugFrame with an associated HostRunLoop.
    ///
    /// `pending_resize` is shared with the event loop so that resizeView
    /// can signal a resize request without directly touching the window
    /// or the IPlugView (avoiding feedback loops).
    pub fn new(
        runloop: Arc<HostRunLoop>,
        pending_resize: PendingResize,
    ) -> ComWrapper<Self> {
        ComWrapper::new(PlugFrame {
            _runloop_ref: runloop,
            pending_resize,
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
        if new_size.is_null() {
            return kResultOk;
        }

        let rect = unsafe { &*new_size };
        let w = (rect.right - rect.left).max(1) as u32;
        let h = (rect.bottom - rect.top).max(1) as u32;
        debug!("IPlugFrame::resizeView requested: {}x{}", w, h);

        // Store the requested size; the event loop will resize the window
        // and call IPlugView::onSize() in about_to_wait.
        if let Ok(mut pending) = self.pending_resize.lock() {
            *pending = Some((w, h));
        }

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
