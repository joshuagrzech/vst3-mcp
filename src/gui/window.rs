//! EditorWindow: winit window creation and IPlugView lifecycle management.
//!
//! Opens a native X11 window, attaches the VST3 plugin's IPlugView editor,
//! handles XEmbed protocol handshake, and dispatches IRunLoop events in
//! the winit event loop.

use std::ffi::c_void;
use std::os::fd::BorrowedFd;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use polling::{Event as PollEvent, Events, PollMode, Poller};
use tracing::{debug, error, info, warn};
use vst3::Steinberg::Vst::IEditControllerTrait;
use vst3::Steinberg::{IPlugFrame, IPlugView, IPlugViewTrait, ViewRect, kResultOk};
use vst3::com_scrape_types::{ComPtr, ComWrapper};
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
    /// The winit window.
    window: Arc<Window>,
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
    /// Pending resize from IPlugFrame::resizeView (shared with PlugFrame).
    pending_resize: super::plugframe::PendingResize,
    /// Current window size in physical pixels (tracks actual size to skip no-op resizes).
    current_size: (u32, u32),
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
    /// One-shot sender to signal that the editor opened (or failed).
    /// Consumed on first use so we only signal once.
    opened_tx: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    /// External close signal (set by close_editor tool).
    close_signal: Arc<std::sync::atomic::AtomicBool>,
}

impl EditorApp {
    fn new(
        plugin: Arc<Mutex<Option<PluginInstance>>>,
        plugin_name: String,
        runloop: Arc<HostRunLoop>,
        opened_tx: std::sync::mpsc::Sender<Result<(), String>>,
        close_signal: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            plugin,
            plugin_name,
            state: None,
            runloop,
            opened_tx: Some(opened_tx),
            close_signal,
        }
    }
}

impl EditorApp {
    /// Properly tear down the editor by dropping `EditorState`.
    fn cleanup_editor(&mut self) {
        if self.state.take().is_some() {
            // EditorState::drop() performs IPlugView teardown. Calling removed()/setFrame()
            // here as well can double-dispose plugin views and crash some plugins on unload.
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
                // Signal that the editor opened successfully
                if let Some(tx) = self.opened_tx.take() {
                    let _ = tx.send(Ok(()));
                }
            }
            Err(e) => {
                error!("Failed to create editor window: {}", e);
                // Signal the error so open_editor doesn't hang
                if let Some(tx) = self.opened_tx.take() {
                    let _ = tx.send(Err(e.clone()));
                }
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
                self.cleanup_editor();
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
            WindowEvent::Resized(_size) => {
                // Do NOT call plug_view.onSize() here.
                // Resize is handled via the pending_resize mechanism in about_to_wait,
                // which calls onSize exactly once per resizeView request.
                // Calling onSize from here causes a feedback loop where the plugin
                // repositions its child window on every frame.
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Check external close signal
        if self.close_signal.load(std::sync::atomic::Ordering::Relaxed) {
            info!("External close signal received, closing editor");
            self.cleanup_editor();
            event_loop.exit();
            return;
        }

        let Some(state) = &mut self.state else {
            return;
        };

        // 1. Handle pending resize from IPlugFrame::resizeView
        apply_pending_resize(state);

        // 2. Poll X11 events for CreateNotify (child window detection)
        poll_x11_events(state);

        // 3. Dispatch IRunLoop timers
        state.runloop.dispatch_timers();

        // 4. Poll registered FDs and dispatch to event handlers
        poll_and_dispatch_fds(state);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Ensure editor state is cleaned up
        self.cleanup_editor();
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
        ComPtr::from_raw(view_ptr).ok_or_else(|| "Invalid IPlugView pointer".to_string())?
    };

    // Check platform support
    let supported =
        unsafe { plug_view.isPlatformTypeSupported(PLATFORM_TYPE_X11.as_ptr() as *const i8) };
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
            warn!(
                "IPlugView::getSize failed (code {}), using default 800x600",
                result
            );
            view_rect.right = 800;
            view_rect.bottom = 600;
        }
    }

    let width = (view_rect.right - view_rect.left).max(100) as u32;
    let height = (view_rect.bottom - view_rect.top).max(100) as u32;

    // Drop plugin lock before creating window (no longer needed)
    drop(plugin_guard);

    // Create winit window (not user-resizable; plugin controls size via resizeView)
    let window_attrs = WindowAttributes::default()
        .with_title(format!("{} - Plugin Editor", plugin_name))
        .with_inner_size(PhysicalSize::new(width, height))
        .with_resizable(false);

    let window = Arc::new(
        event_loop
            .create_window(window_attrs)
            .map_err(|e| format!("Failed to create window: {}", e))?,
    );

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

    // Shared pending resize slot (PlugFrame writes, event loop reads)
    let pending_resize: super::plugframe::PendingResize = Arc::new(Mutex::new(None));

    // Create IPlugFrame (provides IRunLoop to the plugin + resize channel)
    let plug_frame = PlugFrame::new(Arc::clone(&runloop), Arc::clone(&pending_resize));

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
        window,
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
        pending_resize,
        current_size: (width, height),
    })
}

/// Extract X11 Window ID from a winit window using raw-window-handle.
fn get_x11_window_id(window: &Window) -> Result<u32, String> {
    use raw_window_handle::HasWindowHandle;

    let handle = window
        .window_handle()
        .map_err(|e| format!("Failed to get window handle: {}", e))?;

    match handle.as_raw() {
        raw_window_handle::RawWindowHandle::Xlib(xlib) => Ok(xlib.window as u32),
        raw_window_handle::RawWindowHandle::Xcb(xcb) => Ok(xcb.window.get()),
        other => Err(format!("Not running on X11 (got {:?})", other)),
    }
}

/// Apply a pending resize request from IPlugFrame::resizeView.
///
/// Resizes the host window and calls IPlugView::onSize() exactly once,
/// avoiding the feedback loop that would occur if onSize were called
/// from the winit Resized event handler.
fn apply_pending_resize(state: &mut EditorState) {
    let requested = {
        let mut pending = state.pending_resize.lock().unwrap();
        pending.take()
    };

    if let Some((w, h)) = requested {
        // Skip if the window is already at the requested size (avoids confusing
        // the plugin with a redundant onSize call during initial attach).
        if (w, h) == state.current_size {
            return;
        }

        // Resize the host window
        let _ = state.window.request_inner_size(PhysicalSize::new(w, h));

        // Notify the plugin of the new size
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        unsafe {
            let result = state.plug_view.onSize(&mut rect);
            if result != kResultOk {
                debug!("IPlugView::onSize({w}x{h}) returned {result}");
            }
        }

        state.current_size = (w, h);
        debug!("Applied pending resize: {}x{}", w, h);
    }
}

/// Poll X11 events for CreateNotify (plugin child window creation).
fn poll_x11_events(state: &mut EditorState) {
    loop {
        match state.x11_conn.poll_for_event() {
            Ok(Some(event)) => {
                match event {
                    x11rb::protocol::Event::CreateNotify(create) => {
                        if create.parent == state.parent_window_id {
                            let child_id = create.window;
                            let prev_child = state.plugin_window_id;
                            if state.plugin_window_id.is_none() {
                                state.plugin_window_id = Some(child_id);
                            }

                            info!(
                                "Plugin created child window {:08X} inside parent {:08X}",
                                child_id, state.parent_window_id
                            );

                            // Only do the initial XEmbed handshake for the first tracked child.
                            // If the plugin creates additional children later, we'll log them first
                            // (and decide how to handle them based on evidence).
                            if prev_child.is_none() {
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
                    }
                    x11rb::protocol::Event::ConfigureNotify(cfg) => {
                        // If the plugin child starts drifting, force it back to origin.
                        // Vital appears to emit ConfigureNotify with increasing x/y; keeping
                        // the embedded child at (0,0) prevents duplicate/partially visible views.
                        if state.plugin_window_id.is_some_and(|w| w == cfg.window)
                            && (cfg.x != 0 || cfg.y != 0)
                        {
                            if let Ok(cookie) = state
                                .x11_conn
                                .configure_window(cfg.window, &ConfigureWindowAux::new().x(0).y(0))
                            {
                                let _ = cookie.check();
                            }
                            let _ = state.x11_conn.flush();
                        }
                    }
                    x11rb::protocol::Event::MapNotify(_map) => {
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
            if state
                .poller
                .modify_with_mode(BorrowedFd::borrow_raw(fd), event, PollMode::Level)
                .is_err()
            {
                let _ = state.poller.add_with_mode(fd, event, PollMode::Level);
            }
        }
    }

    // Poll with zero timeout (non-blocking)
    state.poll_events.clear();
    match state
        .poller
        .wait(&mut state.poll_events, Some(Duration::ZERO))
    {
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

/// Open the plugin's editor window on the current thread.
///
/// This function creates a winit event loop on the current thread and
/// blocks until the editor window is closed. It should be called from
/// a dedicated GUI thread (NOT the Tokio async runtime thread).
///
/// The `opened_tx` channel is used to signal the caller as soon as the
/// editor window is successfully created (or if creation fails), allowing
/// the caller to return early without waiting for the window to close.
///
/// # Arguments
/// * `plugin` - Arc<Mutex<Option<PluginInstance>>> shared with AudioHost
/// * `plugin_name` - Human-readable plugin name for the window title
/// * `opened_tx` - Sender to signal editor open success/failure
///
/// # Returns
/// Ok(()) when the window is closed normally, Err on failure.
pub fn open_editor_window(
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    plugin_name: String,
    opened_tx: std::sync::mpsc::Sender<Result<(), String>>,
    close_signal: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    info!("Opening editor window for '{}'", plugin_name);

    // Create shared run loop (used by both IPlugFrame and event loop)
    let runloop = Arc::new(HostRunLoop::new());

    // Create winit event loop
    let mut builder = EventLoop::builder();

    // On Linux, allow EventLoop creation off the main thread and force X11 so we can
    // obtain an X11 window ID for `X11EmbedWindowID`.
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11().with_any_thread(true);
    }

    let event_loop = builder
        .build()
        .map_err(|e| format!("Failed to create event loop: {}", e))?;

    // Set control flow: poll continuously for timer/FD dispatch
    event_loop.set_control_flow(ControlFlow::Poll);

    // Create application handler with the opened signal channel and close handle
    let mut app = EditorApp::new(plugin, plugin_name, runloop, opened_tx, close_signal);

    // Run the event loop (blocks until window closes or close_signal is set)
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("Event loop error: {}", e))?;

    info!("Editor window closed");
    Ok(())
}

/// Persistent editor app that keeps a single winit event loop alive for
/// repeated open/close cycles. This avoids EventLoop recreation errors.
struct PersistentEditorApp {
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    plugin_name: Arc<RwLock<String>>,
    state: Option<EditorState>,
    runloop: Arc<HostRunLoop>,
    opened_tx: Option<std::sync::mpsc::Sender<Result<(), String>>>,
    close_signal: Arc<std::sync::atomic::AtomicBool>,
    is_open: Arc<std::sync::atomic::AtomicBool>,
}

impl PersistentEditorApp {
    fn new(
        plugin: Arc<Mutex<Option<PluginInstance>>>,
        plugin_name: Arc<RwLock<String>>,
        runloop: Arc<HostRunLoop>,
        opened_tx: std::sync::mpsc::Sender<Result<(), String>>,
        close_signal: Arc<std::sync::atomic::AtomicBool>,
        is_open: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            plugin,
            plugin_name,
            state: None,
            runloop,
            opened_tx: Some(opened_tx),
            close_signal,
            is_open,
        }
    }

    fn cleanup_editor(&mut self) {
        if self.state.take().is_some() {
            self.is_open
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn try_open_editor(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        if self.close_signal.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }

        let plugin_name = self
            .plugin_name
            .read()
            .ok()
            .map(|n| n.clone())
            .unwrap_or_else(|| "Unknown Plugin".to_string());

        match create_editor_state(
            event_loop,
            &self.plugin,
            &plugin_name,
            Arc::clone(&self.runloop),
        ) {
            Ok(state) => {
                self.state = Some(state);
                self.is_open
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(tx) = self.opened_tx.take() {
                    let _ = tx.send(Ok(()));
                }
            }
            Err(e) => {
                error!("Failed to create editor window: {}", e);
                // Stop retrying until the host explicitly re-requests open.
                self.close_signal
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Some(tx) = self.opened_tx.take() {
                    let _ = tx.send(Err(e));
                }
            }
        }
    }
}

impl ApplicationHandler for PersistentEditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.try_open_editor(event_loop);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.cleanup_editor();
                self.close_signal
                    .store(true, std::sync::atomic::Ordering::Relaxed);
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
            WindowEvent::Resized(_size) => {}
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.close_signal.load(std::sync::atomic::Ordering::Relaxed) {
            if self.state.is_some() {
                self.cleanup_editor();
            }
        } else if self.state.is_none() {
            self.try_open_editor(event_loop);
        }

        let Some(state) = &mut self.state else {
            return;
        };

        apply_pending_resize(state);
        poll_x11_events(state);
        state.runloop.dispatch_timers();
        poll_and_dispatch_fds(state);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.cleanup_editor();
    }
}

/// Open and maintain a persistent editor event loop on this thread.
///
/// The first open result is reported through `opened_tx`. Future open/close
/// requests are controlled by toggling `close_signal` and updating
/// `plugin_name`.
pub fn open_editor_window_persistent(
    plugin: Arc<Mutex<Option<PluginInstance>>>,
    plugin_name: Arc<RwLock<String>>,
    opened_tx: std::sync::mpsc::Sender<Result<(), String>>,
    close_signal: Arc<std::sync::atomic::AtomicBool>,
    is_open: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    info!("Starting persistent editor event loop");
    let runloop = Arc::new(HostRunLoop::new());

    let mut builder = EventLoop::builder();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11().with_any_thread(true);
    }

    let event_loop = builder
        .build()
        .map_err(|e| format!("Failed to create event loop: {}", e))?;

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = PersistentEditorApp::new(
        plugin,
        plugin_name,
        runloop,
        opened_tx,
        close_signal,
        is_open,
    );

    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("Event loop error: {}", e))?;

    Ok(())
}
