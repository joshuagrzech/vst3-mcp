//! XEmbed protocol helpers for X11 window embedding.
//!
//! Implements the XEmbed protocol messages required for embedding a VST3
//! plugin's child window inside the host's parent window on Linux X11.
//!
//! Reference: https://specifications.freedesktop.org/xembed-spec/xembed-spec-latest.html

use x11rb::atom_manager;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// XEmbed protocol message types
pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 1;
pub const XEMBED_WINDOW_DEACTIVATE: u32 = 2;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_FOCUS_OUT: u32 = 5;

// XEmbed protocol version
pub const XEMBED_VERSION: u32 = 0;

// XEmbed focus direction for FOCUS_IN detail
pub const XEMBED_FOCUS_CURRENT: u32 = 0;

atom_manager! {
    /// XEmbed atoms used for the embedding protocol.
    pub XEmbedAtoms: XEmbedAtomsCookie {
        _XEMBED,
        _XEMBED_INFO,
    }
}

/// Send an XEmbed protocol message to a window.
///
/// This constructs a ClientMessage event with the _XEMBED atom type
/// and sends it to the specified window.
pub fn send_xembed_message(
    conn: &impl Connection,
    atoms: &XEmbedAtoms,
    window: Window,
    message: u32,
    detail: u32,
    data1: u32,
    data2: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window,
        type_: atoms._XEMBED,
        data: ClientMessageData::from([message, detail, data1, data2, 0]),
    };

    conn.send_event(false, window, EventMask::NO_EVENT, event)?;
    conn.flush()?;

    Ok(())
}

/// Send XEMBED_EMBEDDED_NOTIFY to the plugin's child window.
///
/// This completes the XEmbed handshake by telling the plugin that its
/// window has been embedded in the host's parent window.
pub fn send_embedded_notify(
    conn: &impl Connection,
    atoms: &XEmbedAtoms,
    plugin_window: Window,
    parent_window: Window,
) -> Result<(), Box<dyn std::error::Error>> {
    send_xembed_message(
        conn,
        atoms,
        plugin_window,
        XEMBED_EMBEDDED_NOTIFY,
        0, // detail
        parent_window,
        XEMBED_VERSION,
    )
}

/// Send XEMBED_WINDOW_ACTIVATE to the plugin's child window.
pub fn send_window_activate(
    conn: &impl Connection,
    atoms: &XEmbedAtoms,
    plugin_window: Window,
) -> Result<(), Box<dyn std::error::Error>> {
    send_xembed_message(
        conn,
        atoms,
        plugin_window,
        XEMBED_WINDOW_ACTIVATE,
        0,
        0,
        0,
    )
}

/// Send XEMBED_WINDOW_DEACTIVATE to the plugin's child window.
pub fn send_window_deactivate(
    conn: &impl Connection,
    atoms: &XEmbedAtoms,
    plugin_window: Window,
) -> Result<(), Box<dyn std::error::Error>> {
    send_xembed_message(
        conn,
        atoms,
        plugin_window,
        XEMBED_WINDOW_DEACTIVATE,
        0,
        0,
        0,
    )
}

/// Send XEMBED_FOCUS_IN to the plugin's child window.
pub fn send_focus_in(
    conn: &impl Connection,
    atoms: &XEmbedAtoms,
    plugin_window: Window,
) -> Result<(), Box<dyn std::error::Error>> {
    send_xembed_message(
        conn,
        atoms,
        plugin_window,
        XEMBED_FOCUS_IN,
        XEMBED_FOCUS_CURRENT,
        0,
        0,
    )
}

/// Send XEMBED_FOCUS_OUT to the plugin's child window.
pub fn send_focus_out(
    conn: &impl Connection,
    atoms: &XEmbedAtoms,
    plugin_window: Window,
) -> Result<(), Box<dyn std::error::Error>> {
    send_xembed_message(
        conn,
        atoms,
        plugin_window,
        XEMBED_FOCUS_OUT,
        0,
        0,
        0,
    )
}
