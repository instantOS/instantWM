use crate::backend::Backend;
use crate::wm::Wm;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;

pub fn check_other_wm(conn: &RustConnection, root: Window) {
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    let result =
        conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));

    if result.is_err() || conn.flush().is_err() {
        panic!("instantwm: another window manager is already running");
    }
}

pub fn setup(_wm: &mut Wm) {}

pub fn setup_root(wm: &mut Wm) {
    let Backend::X11(data) = &mut wm.backend else {
        return;
    };

    let root = data.x11_runtime.root;
    let netatom = data.x11_runtime.netatom;
    let wm_check_atom = netatom.wm_check;
    let wm_name_atom = netatom.wm_name;
    let supported_atoms: Vec<u32> = vec![
        netatom.active_window,
        netatom.supported,
        netatom.system_tray,
        netatom.system_tray_op,
        netatom.system_tray_orientation,
        netatom.system_tray_orientation_horz,
        netatom.wm_name,
        netatom.wm_state,
        netatom.wm_check,
        netatom.wm_fullscreen,
        netatom.wm_window_type,
        netatom.wm_window_type_dialog,
        netatom.client_list,
        netatom.client_info,
    ];

    let mask = EventMask::SUBSTRUCTURE_REDIRECT
        | EventMask::SUBSTRUCTURE_NOTIFY
        | EventMask::BUTTON_PRESS
        | EventMask::POINTER_MOTION
        | EventMask::ENTER_WINDOW
        | EventMask::LEAVE_WINDOW
        | EventMask::STRUCTURE_NOTIFY
        | EventMask::PROPERTY_CHANGE
        | EventMask::KEY_PRESS
        | EventMask::KEY_RELEASE;

    let conn = &data.conn;
    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(mask));
    let _ = conn.flush();

    // Create the EWMH supporting WM check window.
    let wmcheckwin = conn.generate_id().unwrap_or(0);
    let _ = conn.create_window(
        0, // depth: CopyFromParent
        wmcheckwin,
        root,
        0,
        0,
        1,
        1,
        0, // x, y, w, h, border_width
        WindowClass::INPUT_OUTPUT,
        0, // visual: CopyFromParent
        &CreateWindowAux::new(),
    );

    // Set _NET_SUPPORTING_WM_CHECK on the check window itself.
    let _ = conn.change_property32(
        PropMode::REPLACE,
        wmcheckwin,
        wm_check_atom,
        AtomEnum::WINDOW,
        &[wmcheckwin],
    );

    // Set _NET_WM_NAME on the check window.
    let utf8_atom = conn
        .intern_atom(false, b"UTF8_STRING")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(AtomEnum::STRING.into());
    let _ = conn.change_property8(
        PropMode::REPLACE,
        wmcheckwin,
        wm_name_atom,
        utf8_atom,
        b"instantwm",
    );

    // Set _NET_SUPPORTING_WM_CHECK on the root window.
    let _ = conn.change_property32(
        PropMode::REPLACE,
        root,
        wm_check_atom,
        AtomEnum::WINDOW,
        &[wmcheckwin],
    );

    // Advertise _NET_SUPPORTED atoms on the root window.
    let _ = conn.change_property32(
        PropMode::REPLACE,
        root,
        netatom.supported,
        AtomEnum::ATOM,
        &supported_atoms,
    );

    // Clear stale client list and client info.
    let _ = conn.delete_property(root, netatom.client_list);
    let _ = conn.delete_property(root, netatom.client_info);
    let _ = conn.flush();

    // Now set wmcheckwin with mutable access
    data.x11_runtime.wmcheckwin = wmcheckwin;

    let mut ctx = wm.ctx();
    crate::monitor::update_geom(&mut ctx);

    if let crate::contexts::WmCtx::X11(mut x11_ctx) = ctx {
        crate::mouse::set_cursor_style(
            &mut crate::contexts::WmCtx::X11(x11_ctx.reborrow()),
            crate::types::AltCursor::Default,
        );
    }
}
