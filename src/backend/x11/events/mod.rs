//! X11 backend event handlers and helpers.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::lifecycle::{is_window_iconic, manage};
use crate::contexts::WmCtxX11;
use crate::types::{Client, Rect, WindowId};
use std::collections::HashMap;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// scan helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub(crate) struct InitialWindowGeometry {
    pub rect: Rect,
    pub border_width: u32,
}

/// Query the geometry that an X11 window had before instantWM manages it.
///
/// A missing reply usually means the window disappeared while the event was
/// being processed, so callers skip it instead of inventing fallback geometry.
pub(crate) fn query_initial_window_geometry(
    x11: &X11BackendRef,
    window: WindowId,
) -> Option<InitialWindowGeometry> {
    let conn = x11.conn;
    let x11_window: Window = window.into();
    conn.get_geometry(x11_window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|geometry| InitialWindowGeometry {
            rect: Rect {
                x: geometry.x as i32,
                y: geometry.y as i32,
                w: geometry.width as i32,
                h: geometry.height as i32,
            },
            border_width: geometry.border_width as u32,
        })
}

/// Returns `true` when the `override_redirect` attribute is set on `window`.
pub(crate) fn is_override_redirect(x11: &X11BackendRef, window: WindowId) -> bool {
    let conn = x11.conn;
    let x11_window: Window = window.into();
    conn.get_window_attributes(x11_window)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|wa| wa.override_redirect)
        .unwrap_or(false)
}

/// Partition `children` into `(managed, transients)`.
fn classify_windows(
    clients: &HashMap<WindowId, Client>,
    x11: &X11BackendRef,
    x11_runtime: &crate::backend::x11::X11RuntimeConfig,
    children: Vec<Window>,
) -> (Vec<WindowId>, Vec<WindowId>) {
    let mut managed = Vec::new();
    let mut transients = Vec::new();

    let conn = x11.conn;

    for x11_window in children {
        let window = WindowId::from(x11_window);
        if is_override_redirect(x11, window) {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = conn
            .get_window_attributes(x11_window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| wa.map_state == MapState::VIEWABLE)
            .unwrap_or(false);
        let is_iconic = is_window_iconic(x11, x11_runtime, window);

        if !is_viewable && !is_iconic {
            continue;
        }

        // Skip already-managed windows.
        if clients.contains_key(&window) {
            continue;
        }

        // Check WM_TRANSIENT_FOR directly using the already-borrowed conn.
        let is_transient = conn
            .get_property(
                false,
                x11_window,
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
            transients.push(window);
        } else {
            managed.push(window);
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

    let (managed, transients) = classify_windows(
        &ctx.core.model().clients,
        &ctx.x11,
        ctx.x11_runtime,
        children,
    );

    for window in managed.into_iter().chain(transients) {
        let Some(initial_geometry) = query_initial_window_geometry(&ctx.x11, window) else {
            continue;
        };
        let mut tmp = ctx.reborrow();
        manage(
            &mut tmp,
            window,
            initial_geometry.rect,
            initial_geometry.border_width,
        );
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
