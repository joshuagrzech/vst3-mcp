//! Host-side implementation of VST3 IEventList.
//!
//! Used to deliver note events to the plugin processor during process() calls.

use std::cell::RefCell;

use vst3::Steinberg::Vst::{Event, IEventList, IEventListTrait};
use vst3::Steinberg::{kInvalidArgument, kResultOk, tresult};

/// Host-side event list for VST3 note/event delivery.
pub struct EventList {
    events: RefCell<Vec<Event>>,
}

impl EventList {
    /// Create a new event list with pre-allocated capacity.
    pub fn new(capacity: usize) -> vst3::com_scrape_types::ComWrapper<Self> {
        vst3::com_scrape_types::ComWrapper::new(Self {
            events: RefCell::new(Vec::with_capacity(capacity.max(1))),
        })
    }

    /// Clear all queued events while keeping allocated capacity.
    pub fn clear(&self) {
        self.events.borrow_mut().clear();
    }

    /// Push a new event.
    pub fn push(&self, event: Event) {
        self.events.borrow_mut().push(event);
    }
}

impl vst3::Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> i32 {
        self.events.borrow().len() as i32
    }

    unsafe fn getEvent(&self, index: i32, e: *mut Event) -> tresult {
        if e.is_null() || index < 0 {
            return kInvalidArgument;
        }

        let events = self.events.borrow();
        let Some(event) = events.get(index as usize) else {
            return kInvalidArgument;
        };

        // Safety: `e` was checked for null and points to caller-provided storage.
        unsafe {
            *e = *event;
        }
        kResultOk
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }

        // Safety: `e` was checked for null and points to caller-provided event.
        let event = unsafe { *e };
        self.events.borrow_mut().push(event);
        kResultOk
    }
}
