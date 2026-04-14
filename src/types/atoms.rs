//! X11 atom types.
//!
//! Named structs for X11 protocol atoms

/// Named struct for WM protocol atoms (replaces `wmatom: [u32; 4]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct WmAtoms {
    /// WM_PROTOCOLS atom.
    pub protocols: u32,
    /// WM_DELETE_WINDOW atom.
    pub delete: u32,
    /// WM_STATE atom.
    pub state: u32,
    /// WM_TAKE_FOCUS atom.
    pub take_focus: u32,
}

/// Named struct for EWMH / NET atoms (replaces `netatom: [u32; 14]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct NetAtoms {
    /// _NET_ACTIVE_WINDOW atom.
    pub active_window: u32,
    /// _NET_SUPPORTED atom.
    pub supported: u32,
    /// _NET_SYSTEM_TRAY atom.
    pub system_tray: u32,
    /// _NET_SYSTEM_TRAY_OPCODE atom.
    pub system_tray_op: u32,
    /// _NET_SYSTEM_TRAY_ORIENTATION atom.
    pub system_tray_orientation: u32,
    /// _NET_SYSTEM_TRAY_ORIENTATION_HORZ atom.
    pub system_tray_orientation_horz: u32,
    /// _NET_WM_NAME atom.
    pub wm_name: u32,
    /// _NET_WM_STATE atom.
    pub wm_state: u32,
    /// _NET_WM_CHECK atom.
    pub wm_check: u32,
    /// _NET_WM_STATE_FULLSCREEN atom.
    pub wm_fullscreen: u32,
    /// _NET_WM_WINDOW_TYPE atom.
    pub wm_window_type: u32,
    /// _NET_WM_WINDOW_TYPE_DIALOG atom.
    pub wm_window_type_dialog: u32,
    /// _NET_CLIENT_LIST atom.
    pub client_list: u32,
    /// _NET_CLIENT_INFO atom.
    pub client_info: u32,
}

/// Named struct for XEmbed / ICCCM atoms (replaces `xatom: [u32; 3]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct XAtoms {
    /// MANAGER atom for XEmbed.
    pub manager: u32,
    /// _XEMBED atom.
    pub xembed: u32,
    /// _XEMBED_INFO atom.
    pub xembed_info: u32,
}
