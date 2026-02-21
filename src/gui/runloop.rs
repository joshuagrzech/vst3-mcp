//! Linux::IRunLoop COM implementation for VST3 plugin event dispatch.
//!
//! Plugins on Linux use IRunLoop (obtained via IPlugFrame queryInterface)
//! to register file descriptor event handlers and timers for their UI thread.
//! The host must provide this interface and dispatch events during the
//! GUI event loop.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(not(unix))]
type RawFd = i32;

use tracing::debug;
use vst3::Steinberg::Linux::{
    FileDescriptor, IEventHandler, IRunLoop, IRunLoopTrait, ITimerHandler, TimerInterval,
};
use vst3::Steinberg::{FUnknown, kResultOk, tresult};
use vst3::com_scrape_types::Unknown;

/// A registered timer entry with handler, interval, and last fire time.
struct TimerEntry {
    handler: *mut ITimerHandler,
    interval_ms: u64,
    last_fire: Instant,
}

// Safety: TimerEntry contains a raw COM pointer that is only accessed
// from the GUI thread. The Mutex around the Vec<TimerEntry> ensures
// no concurrent access.
unsafe impl Send for TimerEntry {}

/// Host-side implementation of Linux::IRunLoop.
///
/// Stores registered file descriptor event handlers and timers from plugins.
/// The host's GUI event loop calls `dispatch_timers()` and `dispatch_ready_fds()`
/// on each iteration to process plugin events.
pub struct HostRunLoop {
    event_handlers: Mutex<HashMap<RawFd, *mut IEventHandler>>,
    timers: Mutex<Vec<TimerEntry>>,
}

// Safety: HostRunLoop is accessed behind a Mutex in practice (via Arc).
// The raw pointers inside are COM pointers that are AddRef'd on registration
// and only accessed from the GUI thread.
unsafe impl Send for HostRunLoop {}
unsafe impl Sync for HostRunLoop {}

impl HostRunLoop {
    /// Create a new HostRunLoop.
    pub fn new() -> Self {
        HostRunLoop {
            event_handlers: Mutex::new(HashMap::new()),
            timers: Mutex::new(Vec::new()),
        }
    }

    /// Get all registered file descriptors (for polling setup).
    pub fn get_registered_fds(&self) -> Vec<RawFd> {
        self.event_handlers
            .lock()
            .map(|h| h.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Called from event loop when FDs are ready.
    pub fn dispatch_ready_fds(&self, ready_fds: &[RawFd]) {
        let handlers = self.event_handlers.lock().unwrap();
        for &fd in ready_fds {
            if let Some(&handler) = handlers.get(&fd) {
                unsafe {
                    let vtbl = &*(*handler).vtbl;
                    (vtbl.onFDIsSet)(handler, fd);
                }
            }
        }
    }

    /// Called from event loop on each iteration to fire expired timers.
    pub fn dispatch_timers(&self) {
        let mut timers = self.timers.lock().unwrap();
        let now = Instant::now();

        for entry in timers.iter_mut() {
            let elapsed = now.duration_since(entry.last_fire).as_millis() as u64;
            if elapsed >= entry.interval_ms {
                unsafe {
                    let vtbl = &*(*entry.handler).vtbl;
                    (vtbl.onTimer)(entry.handler);
                }
                entry.last_fire = now;
            }
        }
    }

    /// Get the shortest timer interval in milliseconds (for event loop timeout).
    /// Returns None if no timers are registered.
    pub fn min_timer_interval_ms(&self) -> Option<u64> {
        self.timers
            .lock()
            .ok()
            .and_then(|t| t.iter().map(|e| e.interval_ms).min())
    }
}

impl vst3::Class for HostRunLoop {
    type Interfaces = (IRunLoop,);
}

impl IRunLoopTrait for HostRunLoop {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut IEventHandler,
        fd: FileDescriptor,
    ) -> tresult {
        if handler.is_null() {
            return vst3::Steinberg::kInvalidArgument;
        }

        // AddRef the handler to keep it alive while registered
        unsafe { FUnknown::add_ref(handler as *mut FUnknown) };

        self.event_handlers
            .lock()
            .unwrap()
            .insert(fd as RawFd, handler);
        debug!("IRunLoop: registered event handler for FD {}", fd);
        kResultOk
    }

    unsafe fn unregisterEventHandler(&self, handler: *mut IEventHandler) -> tresult {
        let mut handlers = self.event_handlers.lock().unwrap();
        let removed: Vec<RawFd> = handlers
            .iter()
            .filter(|&(_, h)| std::ptr::eq(*h, handler))
            .map(|(&fd, _)| fd)
            .collect();

        for fd in &removed {
            handlers.remove(fd);
        }

        if !removed.is_empty() {
            // Release the handler reference we took in registerEventHandler
            unsafe { FUnknown::release(handler as *mut FUnknown) };
            debug!("IRunLoop: unregistered event handler (FDs: {:?})", removed);
        }

        kResultOk
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut ITimerHandler,
        milliseconds: TimerInterval,
    ) -> tresult {
        if handler.is_null() {
            return vst3::Steinberg::kInvalidArgument;
        }

        // AddRef the handler
        unsafe { FUnknown::add_ref(handler as *mut FUnknown) };

        self.timers.lock().unwrap().push(TimerEntry {
            handler,
            interval_ms: milliseconds,
            last_fire: Instant::now(),
        });

        debug!(
            "IRunLoop: registered timer with interval {}ms",
            milliseconds
        );
        kResultOk
    }

    unsafe fn unregisterTimer(&self, handler: *mut ITimerHandler) -> tresult {
        let mut timers = self.timers.lock().unwrap();
        let original_len = timers.len();
        timers.retain(|entry| !std::ptr::eq(entry.handler, handler));

        if timers.len() < original_len {
            // Release the handler reference
            unsafe { FUnknown::release(handler as *mut FUnknown) };
            debug!("IRunLoop: unregistered timer");
        }

        kResultOk
    }
}

impl Drop for HostRunLoop {
    fn drop(&mut self) {
        // Release all held COM references
        let handlers = self.event_handlers.get_mut().unwrap();
        for (_, &handler) in handlers.iter() {
            unsafe {
                FUnknown::release(handler as *mut FUnknown);
            }
        }
        handlers.clear();

        let timers = self.timers.get_mut().unwrap();
        for entry in timers.iter() {
            unsafe {
                FUnknown::release(entry.handler as *mut FUnknown);
            }
        }
        timers.clear();

        debug!("HostRunLoop dropped, all handlers released");
    }
}
