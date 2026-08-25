//! X11 backend event handlers and helpers.

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::constants::WM_STATE_ICONIC;
use crate::backend::x11::lifecycle::manage;
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

/// Fetch attributes and geometry concurrently for a new MapRequest.
pub(crate) fn query_manageable_window_geometry(
    x11: &X11BackendRef,
    window: WindowId,
) -> Option<InitialWindowGeometry> {
    let conn = x11.conn;
    let x11_window: Window = window.into();
    let attributes = conn.get_window_attributes(x11_window).ok()?;
    let geometry = conn.get_geometry(x11_window).ok()?;
    if attributes.reply().ok()?.override_redirect {
        return None;
    }
    let geometry = geometry.reply().ok()?;
    Some(InitialWindowGeometry {
        rect: Rect::new(
            geometry.x as i32,
            geometry.y as i32,
            geometry.width as i32,
            geometry.height as i32,
        ),
        border_width: geometry.border_width as u32,
    })
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

    // Queue classification for the entire root tree before reading anything.
    let requests: Vec<_> = children
        .into_iter()
        .map(|x11_window| {
            (
                x11_window,
                conn.get_window_attributes(x11_window),
                conn.get_property(
                    false,
                    x11_window,
                    x11_runtime.wmatom.state,
                    x11_runtime.wmatom.state,
                    0,
                    2,
                ),
                conn.get_property(
                    false,
                    x11_window,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    0,
                    1,
                ),
            )
        })
        .collect();

    for (x11_window, attributes, state, transient) in requests {
        let window = WindowId::from(x11_window);
        let Some(attributes) = attributes.ok().and_then(|cookie| cookie.reply().ok()) else {
            continue;
        };
        if attributes.override_redirect {
            continue;
        }

        // Skip windows that are neither visible nor iconic.
        let is_viewable = attributes.map_state == MapState::VIEWABLE;
        let is_iconic = state
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32()?.next())
            .is_some_and(|state| state as i32 == WM_STATE_ICONIC);

        if !is_viewable && !is_iconic {
            continue;
        }

        // Skip already-managed windows.
        if clients.contains_key(&window) {
            continue;
        }

        // Check WM_TRANSIENT_FOR directly using the already-borrowed conn.
        let is_transient = transient
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

    let geometry_requests: Vec<_> = managed
        .into_iter()
        .chain(transients)
        .filter_map(|window| Some((window, conn.get_geometry(window.into()).ok()?)))
        .collect();
    let geometries: Vec<_> = geometry_requests
        .into_iter()
        .filter_map(|(window, request)| {
            let geometry = request.reply().ok()?;
            Some((
                window,
                InitialWindowGeometry {
                    rect: Rect::new(
                        geometry.x as i32,
                        geometry.y as i32,
                        geometry.width as i32,
                        geometry.height as i32,
                    ),
                    border_width: geometry.border_width as u32,
                },
            ))
        })
        .collect();

    for (window, initial_geometry) in geometries {
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
