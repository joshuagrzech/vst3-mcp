//! EditorWindow: winit window creation and IPlugView lifecycle management.
//!
//! Opens a native X11 window, attaches the VST3 plugin's IPlugView editor,
//! handles XEmbed protocol handshake, and dispatches IRunLoop events in
//! the winit event loop.

use std::ffi::c_void;
use std::os::fd::BorrowedFd;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polling::{Event as PollEvent, Events, PollMode, Poller};
use tracing::{debug, error, info, warn};
use vst3::com_scrape_types::{ComPtr, ComWrapper};
use vst3::Steinberg::{
    kResultOk, IPlugFrame, IPlugView, IPlugViewTrait, ViewRect,
};
use vst3::Steinberg::Vst::IEditControllerTrait;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::hosting::plugin::PluginInstance;

use super::plugframe::PlugFrame;
use super::runloop::HostRunLoop;
use super::xembed::{self, XEmbedAtoms};

/// The VST3 platform type string for X11 embedding.
const PLATFORM_TYPE_X11: &[u8] = b"X11EmbedWindowID\0";

/// State for the editor window once created.
struct EditorState {
    /// The winit window (kept alive to maintain X11 window lifetime).
    _window: Window,
    /// The plugin's IPlugView COM pointer.
    plug_view: ComPtr<IPlugView>,
    /// The IPlugFrame COM wrapper (must stay alive while plug_view references it).
    _plug_frame: ComWrapper<PlugFrame>,
    /// X11 connection for XEmbed protocol.
    x11_conn: RustConnection,
    /// XEmbed atoms.
    xembed_atoms: XEmbedAtoms,
    /// The X11 Window ID of our parent window.
    parent_window_id: u32,
    /// The X11 Window ID of the plugin's child window (detected via CreateNotify).
    plugin_window_id: Option<u32>,
    /// Shared run loop for timer/FD dispatch.
    runloop: Arc<HostRunLoop>,
    /// Poller for FD monitoring.
    poller: Poller,
    /// Buffer for poll events.
    poll_events: Events,
    /// Whether the XEmbed handshake is complete.
    xembed_complete: bool,
}

/// Application handler for the editor window event loop.
struct EditorApp {
    /// Plugin instance (locked to access controller for createView).
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    /// Plugin name for window title.
    plugin_name: String,
    /// Editor state (created in `resumed`).
    state: Option<EditorState>,
    /// Shared run loop (created before event loop).
    runloop: Arc<HostRunLoop>,
}

impl EditorApp {
    fn new(
        plugin: Arc<Mutex<Option<PluginInstance>>>,
        plugin_name: String,
        runloop: Arc<HostRunLoop>,
    ) -> Self {
        Self {
            plugin,
            plugin_name,
            state: None,
            runloop,
        }
    }
}

impl ApplicationHandler for EditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return; // Already initialized
        }

        match create_editor_state(
            event_loop,
            &self.plugin,
            &self.plugin_name,
            Arc::clone(&self.runloop),
        ) {
            Ok(state) => {
                info!("Editor window created successfully");
                self.state = Some(state);
            }
            Err(e) => {
                error!("Failed to create editor window: {}", e);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                info!("Editor window close requested");
                // Clean up the editor state (calls IPlugView::removed in Drop)
                self.state.take();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                if let Some(state) = &self.state {
                    if let Some(plugin_wid) = state.plugin_window_id {
                        if focused {
                            let _ = xembed::send_window_activate(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                plugin_wid,
                            );
                            let _ = xembed::send_focus_in(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                plugin_wid,
                            );
                        } else {
                            let _ = xembed::send_focus_out(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                plugin_wid,
                            );
                            let _ = xembed::send_window_deactivate(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                plugin_wid,
                            );
                        }
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = &self.state {
                    let mut rect = ViewRect {
                        left: 0,
                        top: 0,
                        right: size.width as i32,
                        bottom: size.height as i32,
                    };
                    unsafe {
                        let result = state.plug_view.onSize(&mut rect);
                        if result != kResultOk {
                            debug!("IPlugView::onSize returned {}", result);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(state) = &mut self.state else {
            return;
        };

        // 1. Poll X11 events for CreateNotify (child window detection)
        poll_x11_events(state);

        // 2. Dispatch IRunLoop timers
        state.runloop.dispatch_timers();

        // 3. Poll registered FDs and dispatch to event handlers
        poll_and_dispatch_fds(state);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Ensure editor state is cleaned up
        self.state.take();
        info!("Editor event loop exiting");
    }
}

/// Create the editor window and attach the plugin's IPlugView.
fn create_editor_state(
    event_loop: &ActiveEventLoop,
    plugin: &Arc<Mutex<Option<PluginInstance>>>,
    plugin_name: &str,
    runloop: Arc<HostRunLoop>,
) -> Result<EditorState, String> {
    // Lock plugin to access controller
    let plugin_guard = plugin.lock().map_err(|e| format!("Lock error: {}", e))?;
    let plugin_inst = plugin_guard
        .as_ref()
        .ok_or_else(|| "No plugin loaded".to_string())?;

    // Get IEditController and create view
    let controller = plugin_inst
        .controller()
        .ok_or_else(|| "Plugin has no edit controller".to_string())?;

    let plug_view: ComPtr<IPlugView> = unsafe {
        let view_ptr = controller.createView(b"editor\0".as_ptr() as *const i8);
        if view_ptr.is_null() {
            return Err("createView returned null -- plugin may not have an editor".to_string());
        }
        ComPtr::from_raw(view_ptr)
            .ok_or_else(|| "Invalid IPlugView pointer".to_string())?
    };

    // Check platform support
    let supported = unsafe {
        plug_view.isPlatformTypeSupported(PLATFORM_TYPE_X11.as_ptr() as *const i8)
    };
    if supported != kResultOk {
        return Err(format!(
            "Plugin doesn't support X11EmbedWindowID (returned {})",
            supported
        ));
    }

    // Query plugin's preferred size
    let mut view_rect: ViewRect = ViewRect {
        left: 0,
        top: 0,
        right: 800,
        bottom: 600,
    };
    unsafe {
        let result = plug_view.getSize(&mut view_rect);
        if result != kResultOk {
            warn!("IPlugView::getSize failed (code {}), using default 800x600", result);
            view_rect.right = 800;
            view_rect.bottom = 600;
        }
    }

    let width = (view_rect.right - view_rect.left).max(100) as u32;
    let height = (view_rect.bottom - view_rect.top).max(100) as u32;

    // Drop plugin lock before creating window (no longer needed)
    drop(plugin_guard);

    // Create winit window
    let window_attrs = WindowAttributes::default()
        .with_title(format!("{} - Plugin Editor", plugin_name))
        .with_inner_size(PhysicalSize::new(width, height))
        .with_resizable(false); // Fixed size for Phase 04.1

    let window = event_loop
        .create_window(window_attrs)
        .map_err(|e| format!("Failed to create window: {}", e))?;

    // Get X11 Window ID from winit
    let parent_window_id = get_x11_window_id(&window)?;

    info!(
        "Created host window {:08X} ({}x{}) for '{}'",
        parent_window_id, width, height, plugin_name
    );

    // Establish direct X11 connection for XEmbed protocol
    let (x11_conn, _screen_num) =
        RustConnection::connect(None).map_err(|e| format!("X11 connect failed: {}", e))?;

    // Intern XEmbed atoms
    let xembed_atoms = XEmbedAtoms::new(&x11_conn)
        .map_err(|e| format!("Failed to create XEmbed atom cookies: {}", e))?
        .reply()
        .map_err(|e| format!("Failed to intern XEmbed atoms: {}", e))?;

    // Select SubstructureNotify events on parent window to detect plugin's child window
    x11_conn
        .change_window_attributes(
            parent_window_id,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::FOCUS_CHANGE),
        )
        .map_err(|e| format!("Failed to select X11 events: {}", e))?
        .check()
        .map_err(|e| format!("Failed to apply X11 event mask: {}", e))?;

    // Create IPlugFrame (provides IRunLoop to the plugin)
    let plug_frame = PlugFrame::new(Arc::clone(&runloop));

    // Set the plug frame on the view
    unsafe {
        let frame_ptr = plug_frame
            .to_com_ptr::<IPlugFrame>()
            .ok_or_else(|| "Failed to get IPlugFrame COM pointer".to_string())?;

        let result = plug_view.setFrame(frame_ptr.as_ptr());
        if result != kResultOk {
            warn!("IPlugView::setFrame returned {} (non-fatal)", result);
        }
    }

    // Attach plugin view to our X11 window
    unsafe {
        let result = plug_view.attached(
            parent_window_id as usize as *mut c_void,
            PLATFORM_TYPE_X11.as_ptr() as *const i8,
        );
        if result != kResultOk {
            // Clean up on failure
            plug_view.setFrame(std::ptr::null_mut());
            return Err(format!("IPlugView::attached failed with code {}", result));
        }
    }

    info!(
        "Plugin editor attached to X11 window {:08X}",
        parent_window_id
    );

    // Create poller for FD monitoring
    let poller = Poller::new().map_err(|e| format!("Failed to create poller: {}", e))?;

    Ok(EditorState {
        _window: window,
        plug_view,
        _plug_frame: plug_frame,
        x11_conn,
        xembed_atoms,
        parent_window_id,
        plugin_window_id: None,
        runloop,
        poller,
        poll_events: Events::new(),
        xembed_complete: false,
    })
}

/// Extract X11 Window ID from a winit window using raw-window-handle.
fn get_x11_window_id(window: &Window) -> Result<u32, String> {
    use raw_window_handle::HasWindowHandle;

    let handle = window
        .window_handle()
        .map_err(|e| format!("Failed to get window handle: {}", e))?;

    match handle.as_raw() {
        raw_window_handle::RawWindowHandle::Xlib(xlib) => {
            Ok(xlib.window as u32)
        }
        raw_window_handle::RawWindowHandle::Xcb(xcb) => {
            Ok(xcb.window.get())
        }
        other => Err(format!("Not running on X11 (got {:?})", other)),
    }
}

/// Poll X11 events for CreateNotify (plugin child window creation).
fn poll_x11_events(state: &mut EditorState) {
    loop {
        match state.x11_conn.poll_for_event() {
            Ok(Some(event)) => {
                match event {
                    x11rb::protocol::Event::CreateNotify(create) => {
                        if create.parent == state.parent_window_id
                            && state.plugin_window_id.is_none()
                        {
                            let child_id = create.window;
                            state.plugin_window_id = Some(child_id);

                            info!(
                                "Plugin created child window {:08X} inside parent {:08X}",
                                child_id, state.parent_window_id
                            );

                            // Complete XEmbed handshake
                            if let Err(e) = xembed::send_embedded_notify(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                child_id,
                                state.parent_window_id,
                            ) {
                                warn!("Failed to send XEMBED_EMBEDDED_NOTIFY: {}", e);
                            } else {
                                debug!("Sent XEMBED_EMBEDDED_NOTIFY to {:08X}", child_id);
                            }

                            // Send window activate and focus
                            let _ = xembed::send_window_activate(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                child_id,
                            );
                            let _ = xembed::send_focus_in(
                                &state.x11_conn,
                                &state.xembed_atoms,
                                child_id,
                            );

                            state.xembed_complete = true;
                            info!("XEmbed handshake complete for child {:08X}", child_id);
                        }
                    }
                    x11rb::protocol::Event::MapNotify(_) => {
                        // Child window mapped -- ensure it's visible
                        debug!("MapNotify received");
                    }
                    _ => {
                        // Ignore other X11 events
                    }
                }
            }
            Ok(None) => break, // No more events
            Err(e) => {
                warn!("X11 poll_for_event error: {}", e);
                break;
            }
        }
    }
}

/// Poll registered FDs and dispatch to IRunLoop event handlers.
fn poll_and_dispatch_fds(state: &mut EditorState) {
    let fds = state.runloop.get_registered_fds();
    if fds.is_empty() {
        return;
    }

    // Register FDs with the poller (re-register each time since set may change)
    for (idx, &fd) in fds.iter().enumerate() {
        // Use modify-or-add pattern: try to modify first, add if it fails
        unsafe {
            let event = PollEvent::new(idx, true, false);
            if state.poller.modify_with_mode(
                BorrowedFd::borrow_raw(fd),
                event,
                PollMode::Level,
            ).is_err() {
                let _ = state.poller.add_with_mode(fd, event, PollMode::Level);
            }
        }
    }

    // Poll with zero timeout (non-blocking)
    state.poll_events.clear();
    match state.poller.wait(&mut state.poll_events, Some(Duration::ZERO)) {
        Ok(_) => {
            let ready_fds: Vec<RawFd> = state
                .poll_events
                .iter()
                .filter_map(|ev| fds.get(ev.key).copied())
                .collect();

            if !ready_fds.is_empty() {
                state.runloop.dispatch_ready_fds(&ready_fds);
            }
        }
        Err(e) => {
            if e.kind() != std::io::ErrorKind::Interrupted {
                debug!("Poller wait error: {}", e);
            }
        }
    }
}

impl Drop for EditorState {
    fn drop(&mut self) {
        info!("Cleaning up editor state");

        // Detach plugin view BEFORE dropping the window
        unsafe {
            let result = self.plug_view.removed();
            if result != kResultOk {
                warn!("IPlugView::removed() returned {}", result);
            }

            // Clear frame reference
            self.plug_view.setFrame(std::ptr::null_mut());
        }

        info!("Editor state cleaned up (IPlugView removed, frame cleared)");
    }
}

/// Open the plugin's editor window (blocking).
///
/// This function creates a winit event loop on the current thread and
/// blocks until the editor window is closed. It should be called from
/// a dedicated GUI thread (NOT the Tokio async runtime thread).
///
/// # Arguments
/// * `plugin` - Arc<Mutex<Option<PluginInstance>>> shared with AudioHost
/// * `plugin_name` - Human-readable plugin name for the window title
///
/// # Returns
/// Ok(()) when the window is closed normally, Err on failure.
pub fn open_editor_window(
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    plugin_name: String,
) -> Result<(), String> {
    info!("Opening editor window for '{}'", plugin_name);

    // Create shared run loop (used by both IPlugFrame and event loop)
    let runloop = Arc::new(HostRunLoop::new());

    // Create winit event loop
    let event_loop = EventLoop::new()
        .map_err(|e| format!("Failed to create event loop: {}", e))?;

    // Set control flow: poll continuously for timer/FD dispatch
    event_loop.set_control_flow(ControlFlow::Poll);

    // Create application handler
    let mut app = EditorApp::new(plugin, plugin_name, runloop);

    // Run the event loop (blocks until window closes)
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("Event loop error: {}", e))?;

    info!("Editor window closed");
    Ok(())
}
