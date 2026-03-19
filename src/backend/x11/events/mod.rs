//! X11 backend event handlers and helpers.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::lifecycle::{is_window_iconic, manage};
use crate::contexts::{CoreCtx, WmCtxX11};
use crate::types::{Rect, WindowId};
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// scan helpers
// ---------------------------------------------------------------------------

/// Fetch the geometry and border width for `win`.
///
/// Returns a fallback (`800×600`, border `1`) when the request fails.
pub(crate) fn get_win_geometry(_core: &CoreCtx, x11: &X11BackendRef, win: WindowId) -> (Rect, u32) {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    conn.get_geometry(x11_win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|geo| {
            (
                Rect {
                    x: geo.x as i32,
                    y: geo.y as i32,
                    w: geo.width as i32,
                    h: geo.height as i32,
                },
                geo.border_width as u32,
            )
        })
        .unwrap_or((
            Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600,
            },
            1,
        ))
}

/// Returns `true` when the `override_redirect` attribute is set on `win`.
pub(crate) fn is_override_redirect(_core: &CoreCtx, x11: &X11BackendRef, win: WindowId) -> bool {
    let conn = x11.conn;
    let x11_win: Window = win.into();
    conn.get_window_attributes(x11_win)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|wa| wa.override_redirect)
        .unwrap_or(false)
}

/// Partition `children` into `(managed, transients)`.
fn classify_windows(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &crate::backend::x11::X11RuntimeConfig,
    children: Vec<Window>,
) -> (Vec<WindowId>, Vec<WindowId>) {
    let mut managed = Vec::new();
    let mut transients = Vec::new();

    let conn = x11.conn;

    for win in children {
        let win_id = WindowId::from(win);
        if is_override_redirect(core, x11, win_id) {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| wa.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        let is_iconic = is_window_iconic(x11, x11_runtime, win_id);

        if !is_viewable && !is_iconic {
            continue;
        }

        // Skip already-managed windows.
        if core.g.clients.contains_key(&win_id) {
            continue;
        }

        // Check WM_TRANSIENT_FOR directly using the already-borrowed conn.
        let is_transient = conn
            .get_property(
                false,
                win,
                AtomEnum::WM_TRANSIENT_FOR,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut it| it.next()))
            .is_some();
        if is_transient {
            transients.push(win_id);
        } else {
            managed.push(win_id);
        }
    }

    (managed, transients)
}

/// Adopt all pre-existing X11 windows at WM startup.
pub fn scan(ctx: &mut WmCtxX11<'_>) {
    let conn = ctx.x11.conn;
    let root = ctx.x11_runtime.root;

    let children = {
        let Ok(tree_cookie) = conn.query_tree(root) else {
            return;
        };
        let Ok(tree_reply) = tree_cookie.reply() else {
            return;
        };
        tree_reply.children
    };

    let (managed, transients) = classify_windows(&ctx.core, &ctx.x11, ctx.x11_runtime, children);

    for win in managed.into_iter().chain(transients) {
        let (geo, border_width) = get_win_geometry(&ctx.core, &ctx.x11, win);
        let mut tmp = ctx.reborrow();
        manage(&mut tmp, win, geo, border_width);
    }
}

// ---------------------------------------------------------------------------
// Re-exports from submodules
// ---------------------------------------------------------------------------

pub mod handlers;
pub mod loop_fn;
pub mod setup;

pub use loop_fn::run;
pub use setup::{check_other_wm, setup, setup_root};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;

pub const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
pub const XEMBED_FOCUS_IN: u32 = 4;
pub const XEMBED_WINDOW_ACTIVATE: u32 = 5;
pub const XEMBED_MODALITY_ON: u32 = 10;
pub const XEMBED_EMBEDDED_VERSION: u32 = 0;
