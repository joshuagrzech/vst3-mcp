//! Host-side implementations of IParameterChanges and IParamValueQueue.
//!
//! These COM objects are used to deliver parameter changes to the plugin's
//! processor during process() calls with sample-accurate timing.

use std::cell::RefCell;
use vst3::com_scrape_types::ComWrapper;
use vst3::Steinberg::Vst::{
    IParameterChanges, IParameterChangesTrait, IParamValueQueue, IParamValueQueueTrait,
    ParamID, ParamValue,
};
use vst3::Steinberg::{kInvalidArgument, kResultOk};

/// Host-side implementation of IParamValueQueue.
///
/// Stores automation points (sampleOffset, value) for a single parameter.
/// Pre-allocated capacity prevents allocation during process() calls.
pub struct ParamValueQueue {
    param_id: RefCell<ParamID>,
    points: RefCell<Vec<(i32, ParamValue)>>,
}

impl ParamValueQueue {
    /// Create a new parameter value queue with pre-allocated capacity.
    pub fn new() -> ComWrapper<Self> {
        ComWrapper::new(ParamValueQueue {
            param_id: RefCell::new(0),
            points: RefCell::new(Vec::with_capacity(16)), // Pre-allocate for 16 points
        })
    }

    /// Set the parameter ID for this queue.
    pub fn set_parameter_id(&self, id: ParamID) {
        *self.param_id.borrow_mut() = id;
    }

    /// Clear all automation points without dropping capacity.
    pub fn clear(&self) {
        self.points.borrow_mut().clear();
    }

    /// Add an automation point (sample offset, normalized value).
    pub fn add_point(&self, offset: i32, value: ParamValue) {
        self.points.borrow_mut().push((offset, value));
    }
}

impl vst3::Class for ParamValueQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamValueQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        *self.param_id.borrow()
    }

    unsafe fn getPointCount(&self) -> i32 {
        self.points.borrow().len() as i32
    }

    unsafe fn getPoint(&self, index: i32, sample_offset: *mut i32, value: *mut ParamValue) -> i32 {
        let points = self.points.borrow();
        if index >= 0 && (index as usize) < points.len() {
            let (offset, val) = points[index as usize];
            if !sample_offset.is_null() {
                unsafe { *sample_offset = offset; }
            }
            if !value.is_null() {
                unsafe { *value = val; }
            }
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn addPoint(&self, sample_offset: i32, value: ParamValue, index: *mut i32) -> i32 {
        self.add_point(sample_offset, value);
        if !index.is_null() {
            unsafe { *index = (self.points.borrow().len() - 1) as i32; }
        }
        kResultOk
    }
}

/// Host-side implementation of IParameterChanges.
///
/// Manages a collection of parameter value queues, one per changed parameter.
/// Uses fixed capacity with reuse to avoid allocation during process() calls.
pub struct ParameterChanges {
    queues: RefCell<Vec<ComWrapper<ParamValueQueue>>>,
    active_count: RefCell<usize>,
}

impl ParameterChanges {
    /// Create a new parameter changes collection from a set of pre-allocated queues.
    pub fn new(queues: &[ComWrapper<ParamValueQueue>]) -> ComWrapper<Self> {
        ComWrapper::new(ParameterChanges {
            queues: RefCell::new(queues.to_vec()),
            active_count: RefCell::new(0),
        })
    }

    /// Clear all queues and reset active count (keeps capacity).
    pub fn clear(&self) {
        *self.active_count.borrow_mut() = 0;
        for queue in self.queues.borrow().iter() {
            queue.clear();
        }
    }

    /// Activate the next queue for a parameter ID, returning a reference to it.
    pub fn add_parameter(&self, id: ParamID) -> Option<&ComWrapper<ParamValueQueue>> {
        let mut count = self.active_count.borrow_mut();
        let queues = self.queues.borrow();

        if *count < queues.len() {
            let idx = *count;
            *count += 1;
            // Safety: We need to return a reference with the lifetime of &self,
            // but queues is borrowed. We drop the borrow and use unsafe to get
            // a pointer to the Vec's data, which is stable for the lifetime of self.
            drop(queues);
            // Safety: queues_ptr is valid for the lifetime of self, and idx is in bounds
            let queues_ptr = self.queues.as_ptr();
            let queue = unsafe { &(&(*queues_ptr))[idx] };
            queue.set_parameter_id(id);
            Some(queue)
        } else {
            None // Exceeded capacity
        }
    }
}

impl vst3::Class for ParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParameterChanges {
    unsafe fn getParameterCount(&self) -> i32 {
        *self.active_count.borrow() as i32
    }

    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        let queues = self.queues.borrow();
        if index >= 0 && (index as usize) < *self.active_count.borrow() {
            if let Some(ptr) = queues[index as usize]
                .to_com_ptr::<IParamValueQueue>()
            {
                ptr.as_ptr()
            } else {
                std::ptr::null_mut()
            }
        } else {
            std::ptr::null_mut()
        }
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut i32) -> *mut IParamValueQueue {
        let param_id = if !id.is_null() { unsafe { *id } } else { 0 };
        if let Some(queue) = self.add_parameter(param_id) {
            let count = *self.active_count.borrow();
            if !index.is_null() {
                unsafe { *index = (count - 1) as i32; }
            }
            if let Some(ptr) = queue.to_com_ptr::<IParamValueQueue>() {
                ptr.as_ptr()
            } else {
                std::ptr::null_mut()
            }
        } else {
            std::ptr::null_mut()
        }
    }
}
