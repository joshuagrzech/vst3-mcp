//! Plugin lifecycle state machine with COM RAII.
//!
//! `PluginInstance` wraps the full VST3 lifecycle:
//! Created -> SetupDone -> Active -> Processing
//!
//! Uses runtime enum state checking. All unsafe COM code is contained here.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::Arc;

use tracing::{debug, warn};
use vst3::com_scrape_types::{ComPtr, ComWrapper};
use vst3::Steinberg::Vst::{
    AudioBusBuffers, BusDirections_::*, BusInfo as VstBusInfo, IComponent, IComponentTrait,
    IComponentHandler, IAudioProcessor, IAudioProcessorTrait, IConnectionPoint,
    IConnectionPointTrait, IEditController, IEditControllerTrait,
    MediaTypes_::*, ParameterInfo, ProcessData, ProcessSetup,
    ProcessModes_::kOffline, SymbolicSampleSizes_::kSample32,
    ParamID, ParamValue,
};
use vst3::Steinberg::{
    kResultOk, FUnknown, IBStream, IBStreamTrait, IPluginBaseTrait,
    IPluginFactoryTrait, TUID, int32,
    IBStream_::IStreamSeekMode_::*,
};

use super::host_app::{ComponentHandler, HostApp};
use super::module::VstModule;
use super::types::{
    BusDirection, BusInfo, BusType, HostError, ParamInfo, PluginState,
};

/// A queued parameter change to be delivered via ProcessData.
#[allow(dead_code)]
struct ParameterChange {
    id: ParamID,
    value: ParamValue,
}

/// A loaded and initialized VST3 plugin instance.
///
/// Manages the full lifecycle state machine:
/// Created -> SetupDone -> Active -> Processing
///
/// Drop automatically performs correct teardown.
pub struct PluginInstance {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    controller: Option<ComPtr<IEditController>>,
    state: PluginState,
    class_id: TUID,

    // Keep host objects alive for the plugin's lifetime
    _host_app: ComWrapper<HostApp>,
    _handler: ComWrapper<ComponentHandler>,

    // Connection points for component <-> controller (if connected)
    _comp_connection: Option<ComPtr<IConnectionPoint>>,
    _ctrl_connection: Option<ComPtr<IConnectionPoint>>,

    // Parameter change queue
    param_changes: VecDeque<ParameterChange>,

    // Keep the module alive as long as this instance exists.
    // The module holds the dlopen Library handle; dropping it while
    // COM pointers still reference code in the shared library causes UB.
    _module: Arc<VstModule>,
}

impl PluginInstance {
    /// Create a plugin instance from a module and class ID.
    ///
    /// The `module` is wrapped in `Arc` to ensure the shared library
    /// cannot be unloaded while this instance (and its COM pointers) exist.
    /// The class_id should be the raw 16-byte TUID from scanning.
    pub fn from_factory(
        module: Arc<VstModule>,
        class_id: &TUID,
        host_app: ComWrapper<HostApp>,
        handler: ComWrapper<ComponentHandler>,
    ) -> Result<Self, HostError> {
        let factory = module.factory();

        // 1. Create the component via factory.createInstance()
        let component: ComPtr<IComponent> = unsafe {
            let mut obj: *mut c_void = std::ptr::null_mut();
            let result = factory.createInstance(
                class_id.as_ptr(),
                <IComponent as vst3::Interface>::IID.as_ptr() as *const i8,
                &mut obj,
            );
            if result != kResultOk || obj.is_null() {
                return Err(HostError::InitializeFailed(
                    "factory.createInstance failed for IComponent".to_string(),
                ));
            }
            ComPtr::from_raw(obj as *mut IComponent).ok_or_else(|| {
                HostError::InitializeFailed("null IComponent pointer".to_string())
            })?
        };

        // 2. Initialize component with host context
        let host_ptr = host_app
            .to_com_ptr::<FUnknown>()
            .ok_or_else(|| {
                HostError::InitializeFailed("failed to get FUnknown from HostApp".to_string())
            })?;

        unsafe {
            let result = component.initialize(host_ptr.as_ptr());
            if result != kResultOk {
                return Err(HostError::InitializeFailed(format!(
                    "component.initialize failed with code {}",
                    result
                )));
            }
        }

        // 3. Query IAudioProcessor from component
        let processor: ComPtr<IAudioProcessor> = component.cast().ok_or_else(|| {
            HostError::InitializeFailed(
                "component does not implement IAudioProcessor".to_string(),
            )
        })?;

        // 4. Get or create the edit controller
        let controller: Option<ComPtr<IEditController>> = {
            // First try: query directly from component (common case)
            if let Some(ctrl) = component.cast::<IEditController>() {
                Some(ctrl)
            } else {
                // Second try: get controller class ID and create from factory
                let mut ctrl_cid: TUID = [0; 16];
                let result = unsafe { component.getControllerClassId(&mut ctrl_cid) };
                if result == kResultOk && ctrl_cid != [0; 16] {
                    let mut obj: *mut c_void = std::ptr::null_mut();
                    let result = unsafe {
                        factory.createInstance(
                            ctrl_cid.as_ptr(),
                            <IEditController as vst3::Interface>::IID.as_ptr() as *const i8,
                            &mut obj,
                        )
                    };
                    if result == kResultOk && !obj.is_null() {
                        let ctrl = unsafe { ComPtr::from_raw(obj as *mut IEditController) };
                        if let Some(ref c) = ctrl {
                            // Initialize the separate controller
                            unsafe {
                                let init_result = c.initialize(host_ptr.as_ptr());
                                if init_result != kResultOk {
                                    warn!("controller.initialize failed with code {}", init_result);
                                }
                            }
                        }
                        ctrl
                    } else {
                        debug!("no separate edit controller available");
                        None
                    }
                } else {
                    debug!("no controller class ID available");
                    None
                }
            }
        };

        // 5. Set the component handler on the controller
        if let Some(ref ctrl) = controller
            && let Some(hp) = handler.to_com_ptr::<IComponentHandler>()
        {
            unsafe {
                let result = ctrl.setComponentHandler(hp.as_ptr());
                if result != kResultOk {
                    debug!("setComponentHandler returned {}", result);
                }
            }
        }

        // 6. Connect component <-> controller via IConnectionPoint
        let (comp_connection, ctrl_connection) = if let Some(ref ctrl) = controller {
            let comp_cp: Option<ComPtr<IConnectionPoint>> = component.cast();
            let ctrl_cp: Option<ComPtr<IConnectionPoint>> = ctrl.cast();

            if let (Some(ccp), Some(kcp)) = (&comp_cp, &ctrl_cp) {
                unsafe {
                    let r1 = ccp.connect(kcp.as_ptr());
                    let r2 = kcp.connect(ccp.as_ptr());
                    if r1 != kResultOk || r2 != kResultOk {
                        debug!(
                            "IConnectionPoint connect returned: comp={}, ctrl={}",
                            r1, r2
                        );
                    }
                }
            }
            (comp_cp, ctrl_cp)
        } else {
            (None, None)
        };

        debug!("plugin instance created successfully");

        Ok(PluginInstance {
            component,
            processor,
            controller,
            state: PluginState::Created,
            class_id: *class_id,
            _host_app: host_app,
            _handler: handler,
            _comp_connection: comp_connection,
            _ctrl_connection: ctrl_connection,
            param_changes: VecDeque::new(),
            _module: module,
        })
    }

    /// Get the current lifecycle state.
    pub fn state(&self) -> PluginState {
        self.state
    }

    /// Get the class ID of this plugin instance.
    pub fn class_id(&self) -> &TUID {
        &self.class_id
    }

    /// Set up the plugin for processing.
    ///
    /// Transitions from Created -> SetupDone.
    pub fn setup(&mut self, sample_rate: f64, max_block_size: i32) -> Result<(), HostError> {
        if self.state != PluginState::Created {
            return Err(HostError::InvalidState(format!(
                "setup requires Created state, current: {:?}",
                self.state
            )));
        }

        // Call setupProcessing
        let mut setup = ProcessSetup {
            processMode: kOffline as i32,
            symbolicSampleSize: kSample32 as i32,
            maxSamplesPerBlock: max_block_size,
            sampleRate: sample_rate,
        };

        unsafe {
            let result = self.processor.setupProcessing(&mut setup);
            if result != kResultOk {
                return Err(HostError::SetupFailed(format!(
                    "setupProcessing failed with code {}",
                    result
                )));
            }
        }

        // Activate default audio buses
        self.activate_default_buses()?;

        self.state = PluginState::SetupDone;
        debug!("plugin setup complete ({}Hz, {} block size)", sample_rate, max_block_size);
        Ok(())
    }

    /// Activate the plugin.
    ///
    /// Transitions from SetupDone -> Active.
    pub fn activate(&mut self) -> Result<(), HostError> {
        if self.state != PluginState::SetupDone {
            return Err(HostError::InvalidState(format!(
                "activate requires SetupDone state, current: {:?}",
                self.state
            )));
        }

        unsafe {
            let result = self.component.setActive(1);
            if result != kResultOk {
                return Err(HostError::ActivationFailed(format!(
                    "setActive(true) failed with code {}",
                    result
                )));
            }
        }

        self.state = PluginState::Active;
        debug!("plugin activated");
        Ok(())
    }

    /// Start processing.
    ///
    /// Transitions from Active -> Processing.
    pub fn start_processing(&mut self) -> Result<(), HostError> {
        if self.state != PluginState::Active {
            return Err(HostError::InvalidState(format!(
                "start_processing requires Active state, current: {:?}",
                self.state
            )));
        }

        unsafe {
            let result = self.processor.setProcessing(1);
            if result != kResultOk {
                return Err(HostError::ProcessingFailed(format!(
                    "setProcessing(true) failed with code {}",
                    result
                )));
            }
        }

        self.state = PluginState::Processing;
        debug!("processing started");
        Ok(())
    }

    /// Stop processing.
    ///
    /// Transitions from Processing -> Active.
    pub fn stop_processing(&mut self) -> Result<(), HostError> {
        if self.state != PluginState::Processing {
            return Err(HostError::InvalidState(format!(
                "stop_processing requires Processing state, current: {:?}",
                self.state
            )));
        }

        unsafe {
            let result = self.processor.setProcessing(0);
            if result != kResultOk {
                warn!("setProcessing(false) returned {}", result);
            }
        }

        self.state = PluginState::Active;
        debug!("processing stopped");
        Ok(())
    }

    /// Deactivate the plugin.
    ///
    /// Transitions from Active -> SetupDone.
    pub fn deactivate(&mut self) -> Result<(), HostError> {
        if self.state != PluginState::Active {
            return Err(HostError::InvalidState(format!(
                "deactivate requires Active state, current: {:?}",
                self.state
            )));
        }

        unsafe {
            let result = self.component.setActive(0);
            if result != kResultOk {
                warn!("setActive(false) returned {}", result);
            }
        }

        self.state = PluginState::SetupDone;
        debug!("plugin deactivated");
        Ok(())
    }

    /// Process a block of audio. Only available in Processing state.
    ///
    /// `inputs` and `outputs` are per-channel slices (planar/deinterleaved format).
    pub fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        num_samples: i32,
    ) -> Result<(), HostError> {
        if self.state != PluginState::Processing {
            return Err(HostError::InvalidState(format!(
                "process requires Processing state, current: {:?}",
                self.state
            )));
        }

        unsafe {
            // Build input AudioBusBuffers
            let mut input_channel_ptrs: Vec<*mut f32> = inputs
                .iter()
                .map(|ch| ch.as_ptr() as *mut f32)
                .collect();

            let mut input_bus = AudioBusBuffers {
                numChannels: inputs.len() as i32,
                silenceFlags: 0,
                __field0: std::mem::zeroed(),
            };
            input_bus.__field0.channelBuffers32 = if input_channel_ptrs.is_empty() {
                std::ptr::null_mut()
            } else {
                input_channel_ptrs.as_mut_ptr()
            };

            // Build output AudioBusBuffers
            let mut output_channel_ptrs: Vec<*mut f32> = outputs
                .iter_mut()
                .map(|ch| ch.as_mut_ptr())
                .collect();

            let mut output_bus = AudioBusBuffers {
                numChannels: outputs.len() as i32,
                silenceFlags: 0,
                __field0: std::mem::zeroed(),
            };
            output_bus.__field0.channelBuffers32 = if output_channel_ptrs.is_empty() {
                std::ptr::null_mut()
            } else {
                output_channel_ptrs.as_mut_ptr()
            };

            // Build ProcessData
            let mut process_data = ProcessData {
                processMode: kOffline as i32,
                symbolicSampleSize: kSample32 as i32,
                numSamples: num_samples,
                numInputs: if inputs.is_empty() { 0 } else { 1 },
                numOutputs: if outputs.is_empty() { 0 } else { 1 },
                inputs: if inputs.is_empty() {
                    std::ptr::null_mut()
                } else {
                    &mut input_bus
                },
                outputs: if outputs.is_empty() {
                    std::ptr::null_mut()
                } else {
                    &mut output_bus
                },
                inputParameterChanges: std::ptr::null_mut(),
                outputParameterChanges: std::ptr::null_mut(),
                inputEvents: std::ptr::null_mut(),
                outputEvents: std::ptr::null_mut(),
                processContext: std::ptr::null_mut(),
            };

            // TODO: Deliver queued parameter changes via IParameterChanges
            // For Phase 1, parameter changes via process() are deferred.
            // Drain the queue to avoid unbounded growth.
            self.param_changes.clear();

            let result = self.processor.process(&mut process_data);
            if result != kResultOk {
                return Err(HostError::ProcessingFailed(format!(
                    "process() failed with code {}",
                    result
                )));
            }
        }

        Ok(())
    }

    /// Queue a parameter change for delivery in the next process() call.
    pub fn queue_parameter_change(&mut self, id: u32, value: f64) {
        self.param_changes.push_back(ParameterChange { id, value });
    }

    /// Get the number of parameters exposed by the plugin.
    pub fn get_parameter_count(&self) -> i32 {
        match &self.controller {
            Some(ctrl) => unsafe { ctrl.getParameterCount() },
            None => 0,
        }
    }

    /// Get info about a parameter by index.
    pub fn get_parameter_info(&self, index: i32) -> Result<ParamInfo, HostError> {
        let ctrl = self.controller.as_ref().ok_or_else(|| {
            HostError::InvalidState("no edit controller available".to_string())
        })?;

        unsafe {
            let mut info: ParameterInfo = std::mem::zeroed();
            let result = ctrl.getParameterInfo(index, &mut info);
            if result != kResultOk {
                return Err(HostError::InvalidState(format!(
                    "getParameterInfo({}) failed with code {}",
                    index, result
                )));
            }

            Ok(ParamInfo {
                id: info.id,
                title: string128_to_string(&info.title),
                units: string128_to_string(&info.units),
                default_normalized: info.defaultNormalizedValue,
                step_count: info.stepCount,
                flags: info.flags as u32,
            })
        }
    }

    /// Get the current normalized value of a parameter.
    pub fn get_parameter(&self, id: u32) -> f64 {
        match &self.controller {
            Some(ctrl) => unsafe { ctrl.getParamNormalized(id) },
            None => 0.0,
        }
    }

    /// Get the tail length in samples reported by the plugin.
    ///
    /// Returns the number of samples the plugin needs to process after
    /// input ends (for effects like reverb/delay). Returns u32::MAX
    /// for infinite tail (generator plugins).
    pub fn get_tail_samples(&self) -> u32 {
        unsafe { self.processor.getTailSamples() }
    }

    /// Re-setup the plugin with a new sample rate.
    ///
    /// Performs: stop_processing -> deactivate -> setup(new_rate) -> activate -> start_processing.
    /// Only valid when in Processing state.
    pub fn re_setup(&mut self, sample_rate: f64, max_block_size: i32) -> Result<(), HostError> {
        if self.state != PluginState::Processing {
            return Err(HostError::InvalidState(format!(
                "re_setup requires Processing state, current: {:?}",
                self.state
            )));
        }

        self.stop_processing()?;
        self.deactivate()?;

        // Reset to Created state so setup() will accept it
        self.state = PluginState::Created;
        self.setup(sample_rate, max_block_size)?;
        self.activate()?;
        self.start_processing()?;

        Ok(())
    }

    /// Get bus information for the plugin.
    pub fn get_bus_info(&self) -> Vec<BusInfo> {
        let mut buses = Vec::new();

        for (media_type, bus_type) in [(kAudio, BusType::Audio), (kEvent, BusType::Event)] {
            for (direction, bus_dir) in [(kInput, BusDirection::Input), (kOutput, BusDirection::Output)] {
                let count = unsafe {
                    self.component.getBusCount(media_type as i32, direction as i32)
                };
                for i in 0..count {
                    let mut info: VstBusInfo = unsafe { std::mem::zeroed() };
                    let result = unsafe {
                        self.component.getBusInfo(media_type as i32, direction as i32, i, &mut info)
                    };
                    if result == kResultOk {
                        buses.push(BusInfo {
                            name: string128_to_string(&info.name),
                            channel_count: info.channelCount,
                            bus_type,
                            direction: bus_dir,
                            is_default_active: info.flags & 1 != 0, // kDefaultActive = 1
                        });
                    }
                }
            }
        }

        buses
    }

    /// Get the component COM pointer (for state save/restore).
    pub fn component(&self) -> &ComPtr<IComponent> {
        &self.component
    }

    /// Get the controller COM pointer if available (for state save/restore).
    pub fn controller(&self) -> Option<&ComPtr<IEditController>> {
        self.controller.as_ref()
    }

    /// Activate default audio buses.
    fn activate_default_buses(&self) -> Result<(), HostError> {
        for media_type in [kAudio as i32, kEvent as i32] {
            for direction in [kInput as i32, kOutput as i32] {
                let count = unsafe { self.component.getBusCount(media_type, direction) };
                for i in 0..count {
                    let mut info: VstBusInfo = unsafe { std::mem::zeroed() };
                    let result =
                        unsafe { self.component.getBusInfo(media_type, direction, i, &mut info) };
                    if result == kResultOk && (info.flags & 1 != 0) {
                        // kDefaultActive = 1
                        unsafe {
                            let _ = self.component.activateBus(media_type, direction, i, 1);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Drop for PluginInstance {
    fn drop(&mut self) {
        // Enforce correct teardown order:
        // setProcessing(false) -> setActive(false) -> disconnect -> terminate
        //
        // We use Option::take() to move COM pointers out of the struct so they
        // are dropped at the end of their enclosing block, BEFORE we call
        // terminate(). This ensures COM Release() happens in the correct order
        // relative to terminate() calls, rather than relying on implicit struct
        // field drop order.

        if self.state == PluginState::Processing {
            unsafe {
                let _ = self.processor.setProcessing(0);
            }
            self.state = PluginState::Active;
        }

        if self.state == PluginState::Active {
            unsafe {
                let _ = self.component.setActive(0);
            }
            self.state = PluginState::SetupDone;
        }

        // Disconnect and drop connection point COM pointers BEFORE terminate.
        // take() moves the value out of the Option, so ccp and kcp are dropped
        // at the end of this `if let` block, releasing their COM references.
        if let (Some(ccp), Some(kcp)) = (
            self._comp_connection.take(),
            self._ctrl_connection.take(),
        ) {
            unsafe {
                let _ = ccp.disconnect(kcp.as_ptr());
                let _ = kcp.disconnect(ccp.as_ptr());
            }
            // ccp and kcp dropped here, releasing COM references BEFORE terminate
        }

        // Terminate controller (if separate from component).
        // take() moves the controller out so it is dropped at the end of this
        // block, releasing its COM reference BEFORE component.terminate().
        if let Some(ctrl) = self.controller.take() {
            let comp_as_ctrl: Option<ComPtr<IEditController>> = self.component.cast();
            let is_same = comp_as_ctrl.as_ref().is_some_and(|c| {
                std::ptr::eq(c.as_ptr(), ctrl.as_ptr())
            });

            if !is_same {
                unsafe {
                    let _ = ctrl.terminate();
                }
            }
            // ctrl dropped here, releasing COM reference BEFORE component terminate
        }

        // Terminate component
        unsafe {
            let _ = self.component.terminate();
        }
        // component and processor ComPtrs dropped when struct is dropped.
        // _module (Arc<VstModule>) is dropped last, potentially unloading the
        // shared library only after all COM pointers have been released.

        debug!("plugin instance dropped cleanly");
    }
}

/// Convert a VST3 String128 (UTF-16) to a Rust String.
fn string128_to_string(s128: &[u16; 128]) -> String {
    let len = s128.iter().position(|&c| c == 0).unwrap_or(128);
    String::from_utf16_lossy(&s128[..len])
}

/// An IBStream implementation backed by a Vec<u8>.
///
/// Used for getState/setState operations to capture or provide
/// plugin state data. Uses UnsafeCell for interior mutability since
/// IBStreamTrait methods take `&self`.
pub struct VecStream {
    inner: UnsafeCell<VecStreamInner>,
}

struct VecStreamInner {
    data: Vec<u8>,
    position: usize,
}

impl VecStream {
    /// Create a new empty stream for writing.
    pub fn new() -> ComWrapper<VecStream> {
        ComWrapper::new(VecStream {
            inner: UnsafeCell::new(VecStreamInner {
                data: Vec::new(),
                position: 0,
            }),
        })
    }

    /// Create a stream pre-loaded with data for reading.
    pub fn from_data(data: Vec<u8>) -> ComWrapper<VecStream> {
        ComWrapper::new(VecStream {
            inner: UnsafeCell::new(VecStreamInner { data, position: 0 }),
        })
    }

    /// Get the accumulated data from the stream.
    pub fn data(&self) -> &[u8] {
        // Safety: No concurrent access -- VST3 plugins are single-threaded
        unsafe { &(*self.inner.get()).data }
    }
}

impl vst3::Class for VecStream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for VecStream {
    unsafe fn read(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_bytes_read: *mut int32,
    ) -> vst3::Steinberg::tresult {
        // Safety: IBStream is used single-threaded from the plugin side.
        let inner = unsafe { &mut *self.inner.get() };
        let available = inner.data.len().saturating_sub(inner.position);
        let to_read = (num_bytes as usize).min(available);

        if to_read > 0 && !buffer.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    inner.data.as_ptr().add(inner.position),
                    buffer as *mut u8,
                    to_read,
                );
            }
            inner.position += to_read;
        }

        if !num_bytes_read.is_null() {
            unsafe { *num_bytes_read = to_read as int32 };
        }

        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_bytes_written: *mut int32,
    ) -> vst3::Steinberg::tresult {
        // Safety: IBStream is used single-threaded from the plugin side.
        let inner = unsafe { &mut *self.inner.get() };
        let count = num_bytes as usize;

        if count > 0 && !buffer.is_null() {
            let slice = unsafe { std::slice::from_raw_parts(buffer as *const u8, count) };

            // If writing beyond current data, extend
            if inner.position + count > inner.data.len() {
                inner.data.resize(inner.position + count, 0);
            }
            inner.data[inner.position..inner.position + count].copy_from_slice(slice);
            inner.position += count;
        }

        if !num_bytes_written.is_null() {
            unsafe { *num_bytes_written = count as int32 };
        }

        kResultOk
    }

    unsafe fn seek(
        &self,
        pos: vst3::Steinberg::int64,
        mode: int32,
        result: *mut vst3::Steinberg::int64,
    ) -> vst3::Steinberg::tresult {
        // Safety: IBStream is used single-threaded from the plugin side.
        let inner = unsafe { &mut *self.inner.get() };

        let new_pos = match mode as u32 {
            x if x == kIBSeekSet => pos as usize,
            x if x == kIBSeekCur => (inner.position as i64 + pos) as usize,
            x if x == kIBSeekEnd => (inner.data.len() as i64 + pos) as usize,
            _ => return vst3::Steinberg::kInvalidArgument,
        };

        inner.position = new_pos;

        if !result.is_null() {
            unsafe { *result = new_pos as vst3::Steinberg::int64 };
        }

        kResultOk
    }

    unsafe fn tell(&self, pos: *mut vst3::Steinberg::int64) -> vst3::Steinberg::tresult {
        // Safety: IBStream is used single-threaded from the plugin side.
        let inner = unsafe { &mut *self.inner.get() };
        if !pos.is_null() {
            unsafe { *pos = inner.position as vst3::Steinberg::int64 };
        }
        kResultOk
    }
}
