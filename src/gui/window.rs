//! EditorWindow: winit window creation and IPlugView lifecycle management.
//!
//! Opens a native window, attaches the VST3 plugin's IPlugView editor, and
//! dispatches IRunLoop events in the winit event loop.
//! On Linux/X11, this also performs the XEmbed handshake.

use std::ffi::c_void;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

#[cfg(unix)]
use std::os::fd::BorrowedFd;
#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(not(unix))]
type RawFd = i32;

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
/// The VST3 platform type string for Win32 HWND embedding.
const PLATFORM_TYPE_HWND: &[u8] = b"HWND\0";
/// The VST3 platform type string for macOS NSView embedding.
const PLATFORM_TYPE_NSVIEW: &[u8] = b"NSView\0";

/// Runtime-selected editor embedding platform.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
enum EditorPlatform {
    X11(u32),
    Hwnd(*mut c_void),
    NsView(*mut c_void),
}

impl EditorPlatform {
    fn platform_type(self) -> &'static [u8] {
        match self {
            Self::X11(_) => PLATFORM_TYPE_X11,
            Self::Hwnd(_) => PLATFORM_TYPE_HWND,
            Self::NsView(_) => PLATFORM_TYPE_NSVIEW,
        }
    }

    fn parent_handle(self) -> *mut c_void {
        match self {
            Self::X11(window_id) => window_id as usize as *mut c_void,
            Self::Hwnd(hwnd) => hwnd,
            Self::NsView(ns_view) => ns_view,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::X11(_) => "X11EmbedWindowID",
            Self::Hwnd(_) => "HWND",
            Self::NsView(_) => "NSView",
        }
    }
}

/// Linux/X11-specific embedding state.
struct X11EmbedState {
    /// X11 connection for XEmbed protocol.
    conn: RustConnection,
    /// XEmbed atoms.
    atoms: XEmbedAtoms,
    /// The X11 Window ID of our parent window.
    parent_window_id: u32,
    /// The X11 Window ID of the plugin's child window (detected via CreateNotify).
    plugin_window_id: Option<u32>,
    /// Whether the XEmbed handshake is complete.
    _xembed_complete: bool,
}

/// State for the editor window once created.
struct EditorState {
    /// The winit window.
    window: Arc<Window>,
    /// The plugin's IPlugView COM pointer.
    plug_view: ComPtr<IPlugView>,
    /// The IPlugFrame COM wrapper (must stay alive while plug_view references it).
    _plug_frame: ComWrapper<PlugFrame>,
    /// Platform used for IPlugView::attached.
    platform: EditorPlatform,
    /// Linux/X11-only embedding state.
    x11: Option<X11EmbedState>,
    /// Shared run loop for timer/FD dispatch.
    runloop: Arc<HostRunLoop>,
    /// Poller for FD monitoring.
    poller: Poller,
    /// Buffer for poll events.
    poll_events: Events,
    /// Pending resize from IPlugFrame::resizeView (shared with PlugFrame).
    pending_resize: super::plugframe::PendingResize,
    /// Current window size in physical pixels (tracks actual size to skip no-op resizes).
    current_size: (u32, u32),
    /// Prevents duplicate onSize() when host-initiated resizes re-enter via WindowEvent::Resized.
    pending_host_resize_event: Option<(u32, u32)>,
    /// Whether the plugin reports user-resizable editor support.
    plugin_can_resize: bool,
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
                    send_focus_change(state, focused);
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    apply_host_resize(state, size.width.max(1), size.height.max(1));
                }
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

    // Query plugin resize capabilities before creating the host window.
    let plugin_can_resize = unsafe { plug_view.canResize() == kResultOk };

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

    // Create winit window. If the plugin supports resizing, allow the user to
    // resize the outer host window too so host/plugin stay in sync.
    let window_attrs = WindowAttributes::default()
        .with_title(format!("{} - Plugin Editor", plugin_name))
        .with_inner_size(PhysicalSize::new(width, height))
        .with_resizable(plugin_can_resize);

    let window = Arc::new(
        event_loop
            .create_window(window_attrs)
            .map_err(|e| format!("Failed to create window: {}", e))?,
    );

    // Resolve the native window handle to a VST3 embedding platform.
    let platform = detect_editor_platform(&window)?;
    let supported =
        unsafe { plug_view.isPlatformTypeSupported(platform.platform_type().as_ptr() as *const i8) };
    if supported != kResultOk {
        return Err(format!(
            "Plugin doesn't support {} (returned {})",
            platform.display_name(),
            supported
        ));
    }

    let x11 = match platform {
        EditorPlatform::X11(parent_window_id) => Some(init_x11_embed_state(parent_window_id)?),
        EditorPlatform::Hwnd(_) | EditorPlatform::NsView(_) => None,
    };

    match platform {
        EditorPlatform::X11(parent_window_id) => info!(
            "Created host window {:08X} ({}x{}) for '{}'",
            parent_window_id, width, height, plugin_name
        ),
        EditorPlatform::Hwnd(_) | EditorPlatform::NsView(_) => info!(
            "Created host window via {} backend ({}x{}) for '{}'",
            platform.display_name(),
            width,
            height,
            plugin_name
        ),
    }

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

    // Attach plugin view to the host's native window.
    unsafe {
        let result = plug_view.attached(
            platform.parent_handle(),
            platform.platform_type().as_ptr() as *const i8,
        );
        if result != kResultOk {
            // Clean up on failure
            plug_view.setFrame(std::ptr::null_mut());
            return Err(format!("IPlugView::attached failed with code {}", result));
        }
    }

    info!(
        "Plugin editor attached with platform {}",
        platform.display_name()
    );

    // Create poller for FD monitoring
    let poller = Poller::new().map_err(|e| format!("Failed to create poller: {}", e))?;

    let mut state = EditorState {
        window,
        plug_view,
        _plug_frame: plug_frame,
        platform,
        x11,
        runloop,
        poller,
        poll_events: Events::new(),
        pending_resize,
        current_size: (width, height),
        pending_host_resize_event: None,
        plugin_can_resize,
    };

    // Keep host and plugin on the exact same initial size to avoid black gutters.
    notify_plugin_size(&mut state, width, height);

    Ok(state)
}

/// Resolve the native window handle to a VST3 embedding platform.
fn detect_editor_platform(window: &Window) -> Result<EditorPlatform, String> {
    use raw_window_handle::HasWindowHandle;

    let handle = window
        .window_handle()
        .map_err(|e| format!("Failed to get window handle: {}", e))?;

    match handle.as_raw() {
        raw_window_handle::RawWindowHandle::Xlib(xlib) => Ok(EditorPlatform::X11(xlib.window as u32)),
        raw_window_handle::RawWindowHandle::Xcb(xcb) => Ok(EditorPlatform::X11(xcb.window.get())),
        raw_window_handle::RawWindowHandle::Win32(win32) => {
            #[cfg(target_os = "windows")]
            {
                Ok(EditorPlatform::Hwnd(win32.hwnd.get() as usize as *mut c_void))
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = win32;
                Err("Received Win32 handle on non-Windows build".to_string())
            }
        }
        raw_window_handle::RawWindowHandle::AppKit(appkit) => {
            #[cfg(target_os = "macos")]
            {
                Ok(EditorPlatform::NsView(appkit.ns_view.as_ptr()))
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = appkit;
                Err("Received AppKit handle on non-macOS build".to_string())
            }
        }
        raw_window_handle::RawWindowHandle::Wayland(_) => Err(
            "Wayland native handles are not directly supported by VST3 IPlugView attachment; \
             on Linux this host runs with X11/XWayland for editor embedding."
                .to_string(),
        ),
        other => Err(format!(
            "Unsupported window backend for VST3 editor embedding: {:?}",
            other
        )),
    }
}

/// Create Linux/X11 embedding state (XEmbed atoms + event mask).
fn init_x11_embed_state(parent_window_id: u32) -> Result<X11EmbedState, String> {
    let (conn, _screen_num) =
        RustConnection::connect(None).map_err(|e| format!("X11 connect failed: {}", e))?;

    let atoms = XEmbedAtoms::new(&conn)
        .map_err(|e| format!("Failed to create XEmbed atom cookies: {}", e))?
        .reply()
        .map_err(|e| format!("Failed to intern XEmbed atoms: {}", e))?;

    conn.change_window_attributes(
        parent_window_id,
        &ChangeWindowAttributesAux::new()
            .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::FOCUS_CHANGE),
    )
    .map_err(|e| format!("Failed to select X11 events: {}", e))?
    .check()
    .map_err(|e| format!("Failed to apply X11 event mask: {}", e))?;

    Ok(X11EmbedState {
        conn,
        atoms,
        parent_window_id,
        plugin_window_id: None,
        _xembed_complete: false,
    })
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
        state.pending_host_resize_event = Some((w, h));
        let _ = state.window.request_inner_size(PhysicalSize::new(w, h));

        notify_plugin_size(state, w, h);
        debug!("Applied pending resize: {}x{}", w, h);
    }
}

/// Apply a resize initiated by the host window manager/user.
fn apply_host_resize(state: &mut EditorState, requested_w: u32, requested_h: u32) {
    let requested = (requested_w.max(1), requested_h.max(1));

    if let Some(expected) = state.pending_host_resize_event {
        if expected == requested {
            state.pending_host_resize_event = None;
            state.current_size = requested;
            return;
        }
        // The WM delivered a different size than requested; treat this as a fresh resize.
        state.pending_host_resize_event = None;
    }

    if requested == state.current_size {
        return;
    }

    // If plugin says its editor is fixed-size, snap the outer host window back.
    if !state.plugin_can_resize {
        let (w, h) = state.current_size;
        state.pending_host_resize_event = Some((w, h));
        let _ = state.window.request_inner_size(PhysicalSize::new(w, h));
        return;
    }

    let constrained = constrain_host_resize(state, requested.0, requested.1);
    if constrained != requested {
        state.pending_host_resize_event = Some(constrained);
        let _ = state
            .window
            .request_inner_size(PhysicalSize::new(constrained.0, constrained.1));
    }

    if constrained != state.current_size {
        notify_plugin_size(state, constrained.0, constrained.1);
    }
}

/// Ask the plugin to constrain a host-requested size.
fn constrain_host_resize(state: &EditorState, requested_w: u32, requested_h: u32) -> (u32, u32) {
    let mut rect = ViewRect {
        left: 0,
        top: 0,
        right: requested_w as i32,
        bottom: requested_h as i32,
    };

    unsafe {
        let result = state.plug_view.checkSizeConstraint(&mut rect);
        if result != kResultOk {
            return (requested_w, requested_h);
        }
    }

    (
        (rect.right - rect.left).max(1) as u32,
        (rect.bottom - rect.top).max(1) as u32,
    )
}

/// Query the actual X11 window geometry. On XWayland/Hyprland, winit's reported
/// inner_size can differ from the real X11 geometry due to scaling; using the
/// actual geometry prevents black space around the plugin UI.
fn x11_parent_geometry(x11: &X11EmbedState) -> Option<(u32, u32)> {
    let cookie = x11.conn.get_geometry(x11.parent_window_id as Drawable);
    let reply = cookie.ok()?.reply().ok()?;
    let w = reply.width.max(1) as u32;
    let h = reply.height.max(1) as u32;
    Some((w, h))
}

/// Notify the plugin view about the current host window size.
fn notify_plugin_size(state: &mut EditorState, w: u32, h: u32) {
    // On X11/XWayland, use actual parent geometry when available to fix Hyprland
    // scaling mismatch (winit inner_size vs real X11 window size).
    let (size_w, size_h) = if let Some(ref x11) = state.x11 {
        x11_parent_geometry(x11).unwrap_or((w, h))
    } else {
        (w, h)
    };

    let mut rect = ViewRect {
        left: 0,
        top: 0,
        right: size_w as i32,
        bottom: size_h as i32,
    };
    unsafe {
        let result = state.plug_view.onSize(&mut rect);
        if result != kResultOk {
            debug!("IPlugView::onSize({size_w}x{size_h}) returned {result}");
        }
    }
    state.current_size = (size_w, size_h);

    // Keep X11 child window aligned and sized to match the host.
    if let Some(x11) = state.x11.as_mut() {
        if let Some(child) = x11.plugin_window_id {
            if let Ok(cookie) = x11.conn.configure_window(
                child,
                &ConfigureWindowAux::new().x(0).y(0).width(size_w).height(size_h),
            ) {
                let _ = cookie.check();
            }
            let _ = x11.conn.flush();
        }
    }
}

/// Forward host focus changes to embedded X11 children (XEmbed).
fn send_focus_change(state: &EditorState, focused: bool) {
    let Some(x11) = state.x11.as_ref() else {
        return;
    };
    let Some(plugin_wid) = x11.plugin_window_id else {
        return;
    };

    if focused {
        let _ = xembed::send_window_activate(&x11.conn, &x11.atoms, plugin_wid);
        let _ = xembed::send_focus_in(&x11.conn, &x11.atoms, plugin_wid);
    } else {
        let _ = xembed::send_focus_out(&x11.conn, &x11.atoms, plugin_wid);
        let _ = xembed::send_window_deactivate(&x11.conn, &x11.atoms, plugin_wid);
    }
}

/// Poll X11 events for CreateNotify (plugin child window creation).
fn poll_x11_events(state: &mut EditorState) {
    let Some(x11) = state.x11.as_mut() else {
        return;
    };

    loop {
        match x11.conn.poll_for_event() {
            Ok(Some(event)) => {
                match event {
                    x11rb::protocol::Event::CreateNotify(create) => {
                        if create.parent == x11.parent_window_id {
                            let child_id = create.window;
                            let prev_child = x11.plugin_window_id;
                            if x11.plugin_window_id.is_none() {
                                x11.plugin_window_id = Some(child_id);
                            }

                            info!(
                                "Plugin created child window {:08X} inside parent {:08X}",
                                child_id, x11.parent_window_id
                            );

                            // Only do the initial XEmbed handshake for the first tracked child.
                            // If the plugin creates additional children later, we'll log them first
                            // (and decide how to handle them based on evidence).
                            if prev_child.is_none() {
                                // Complete XEmbed handshake
                                if let Err(e) = xembed::send_embedded_notify(
                                    &x11.conn,
                                    &x11.atoms,
                                    child_id,
                                    x11.parent_window_id,
                                ) {
                                    warn!("Failed to send XEMBED_EMBEDDED_NOTIFY: {}", e);
                                } else {
                                    debug!("Sent XEMBED_EMBEDDED_NOTIFY to {:08X}", child_id);
                                }

                                // Size child to fill parent (fixes Hyprland/XWayland black space)
                                if let Some((pw, ph)) = x11_parent_geometry(x11) {
                                    if let Ok(cookie) = x11.conn.configure_window(
                                        child_id,
                                        &ConfigureWindowAux::new().x(0).y(0).width(pw).height(ph),
                                    ) {
                                        let _ = cookie.check();
                                    }
                                    let _ = x11.conn.flush();
                                }

                                // Send window activate and focus
                                let _ = xembed::send_window_activate(&x11.conn, &x11.atoms, child_id);
                                let _ = xembed::send_focus_in(&x11.conn, &x11.atoms, child_id);

                                x11._xembed_complete = true;
                                info!("XEmbed handshake complete for child {:08X}", child_id);
                            }
                        }
                    }
                    x11rb::protocol::Event::ConfigureNotify(cfg) => {
                        // If the plugin child starts drifting, force it back to origin.
                        // Vital appears to emit ConfigureNotify with increasing x/y; keeping
                        // the embedded child at (0,0) prevents duplicate/partially visible views.
                        if x11.plugin_window_id.is_some_and(|w| w == cfg.window)
                            && (cfg.x != 0 || cfg.y != 0)
                        {
                            if let Ok(cookie) = x11
                                .conn
                                .configure_window(cfg.window, &ConfigureWindowAux::new().x(0).y(0))
                            {
                                let _ = cookie.check();
                            }
                            let _ = x11.conn.flush();
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
#[cfg(unix)]
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

/// Poll registered FDs and dispatch to IRunLoop event handlers.
#[cfg(not(unix))]
fn poll_and_dispatch_fds(_state: &mut EditorState) {}

impl Drop for EditorState {
    fn drop(&mut self) {
        info!(
            "Cleaning up editor state for {}",
            self.platform.display_name()
        );

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
                    send_focus_change(state, focused);
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    apply_host_resize(state, size.width.max(1), size.height.max(1));
                }
            }
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
