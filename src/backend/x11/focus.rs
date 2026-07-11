//! X11-specific focus operations: input focus, button grabs, client messages,
//! ConfigureNotify, urgency hints, and border colour updates.

#![allow(clippy::too_many_arguments)]

use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::backend::x11::constants::WM_HINTS_URGENCY_HINT;
use crate::contexts::CoreCtx;
use crate::core_state::CoreState;
use crate::types::{ButtonTarget, WindowId};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

// ---------------------------------------------------------------------------
// ConfigureNotify
// ---------------------------------------------------------------------------

/// Send a synthetic `ConfigureNotify` event to `win`.
pub fn configure(globals: &crate::core_state::CoreState, x11: &X11BackendRef, win: WindowId) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let Some(c) = globals.model.clients.get(&win) else {
        return;
    };

    let event = ConfigureNotifyEvent {
        response_type: CONFIGURE_NOTIFY_EVENT,
        sequence: 0,
        event: x11_win,
        window: x11_win,
        above_sibling: 0,
        x: c.geo.x as i16,
        y: c.geo.y as i16,
        width: c.geo.w as u16,
        height: c.geo.h as u16,
        border_width: c.border_width as u16,
        override_redirect: false,
    };

    let _ = conn.send_event(false, x11_win, EventMask::STRUCTURE_NOTIFY, event);
    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// ClientMessage helper
// ---------------------------------------------------------------------------

/// Send a `ClientMessage` event to `win`.
pub fn send_event(
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) -> bool {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let wmatom_protocols = x11_runtime.wmatom.protocols;
    let wmatom_take_focus = x11_runtime.wmatom.take_focus;
    let wmatom_delete = x11_runtime.wmatom.delete;

    let (exists, message_type) = if proto == wmatom_take_focus || proto == wmatom_delete {
        let supported = read_wm_protocols(conn, x11_win, x11_runtime.wmatom.protocols)
            .into_iter()
            .any(|p| p == proto);
        (supported, wmatom_protocols)
    } else {
        (true, proto)
    };

    if exists {
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: x11_win,
            type_: message_type,
            data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
        };
        let _ = conn.send_event(false, x11_win, EventMask::from(mask), event);
        let _ = conn.flush();
    }

    exists
}

// ---------------------------------------------------------------------------
// Border colour
// ---------------------------------------------------------------------------

/// Update the border color of `win` based on its current focus and floating state.
pub fn refresh_border_color(
    globals: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    focused: bool,
) {
    let scheme = &x11_runtime.borderscheme;
    let Some(c) = globals.model.clients.get(&win) else {
        return;
    };

    let pixel = if focused {
        let has_tiling = globals.selected_monitor().is_tiling_layout();
        let isfloating = c.mode.is_free_positioned() || !has_tiling;
        if isfloating {
            scheme.float_focus.bg.pixel()
        } else {
            scheme.tile_focus.bg.pixel()
        }
    } else {
        scheme.normal.bg.pixel()
    };

    let x11_win: Window = win.into();
    let _ = x11.conn.change_window_attributes(
        x11_win,
        &ChangeWindowAttributesAux::new().border_pixel(Some(pixel)),
    );
}

// ---------------------------------------------------------------------------
// Focus / unfocus
// ---------------------------------------------------------------------------

/// Give input focus to `win`.
pub fn set_focus(
    globals: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let Some(c) = globals.model.clients.get(&win) else {
        return;
    };

    if !c.never_focus {
        let x11_win: Window = win.into();
        let _ = x11
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, x11_win, CURRENT_TIME);
        let _ = x11.conn.change_property32(
            PropMode::REPLACE,
            x11_runtime.root,
            x11_runtime.netatom.active_window,
            AtomEnum::WINDOW,
            &[x11_win],
        );
    }

    refresh_border_color(globals, x11, x11_runtime, win, true);

    grab_buttons(globals, x11, x11_runtime, win, true);

    let _ = send_event(
        x11,
        x11_runtime,
        win,
        x11_runtime.wmatom.take_focus,
        0,
        x11_runtime.wmatom.take_focus as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    );

    let _ = x11.conn.flush();
}

/// Remove focus from `win`.
///
/// When `redirect_to_root` is true, input focus is redirected to the root
/// window and `_NET_ACTIVE_WINDOW` is cleared.
pub fn unfocus_win(
    globals: &CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    redirect_to_root: bool,
) {
    if win == WindowId::default() {
        return;
    }

    grab_buttons(globals, x11, x11_runtime, win, false);

    refresh_border_color(globals, x11, x11_runtime, win, false);

    if redirect_to_root {
        let _ = x11
            .conn
            .set_input_focus(InputFocus::POINTER_ROOT, x11_runtime.root, CURRENT_TIME);
        let _ = x11
            .conn
            .delete_property(x11_runtime.root, x11_runtime.netatom.active_window);
    }

    let _ = x11.conn.flush();
}

// ---------------------------------------------------------------------------
// Button grabs
// ---------------------------------------------------------------------------

/// Grab or ungrab mouse buttons on `win` depending on whether it is focused.
pub fn grab_buttons(
    globals: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
    focused: bool,
) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let _ = ungrab_button(conn, ButtonIndex::from(0u8), x11_win, ModMask::ANY);

    let numlockmask = x11_runtime.numlockmask;
    let lock_mask = ModMask::LOCK.bits() as u32;
    let button_mask: u32 = EventMask::BUTTON_PRESS.bits() | EventMask::BUTTON_RELEASE.bits();
    let mut grabs: Vec<(u8, u32)> = Vec::new();

    if !focused {
        grabs.extend([(1, 0), (3, 0)]);
    }

    for button in &globals.config.bindings.buttons {
        if !button.matches(ButtonTarget::ClientWin) {
            continue;
        }
        if focused && button.mask == 0 {
            continue;
        }

        let grab = (button.button.to_x11_detail(), button.mask);
        if !grabs.contains(&grab) {
            grabs.push(grab);
        }
    }

    for (button, base_mask) in grabs {
        for &lock_variation in &[0u32, numlockmask, lock_mask, numlockmask | lock_mask] {
            let mods = ModMask::from((base_mask | lock_variation) as u16);
            let _ = conn.grab_button(
                false,
                x11_win,
                button_mask.into(),
                GrabMode::SYNC,
                GrabMode::SYNC,
                0u32,
                0u32,
                button.into(),
                mods,
            );
        }
    }

    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// Urgency hint
// ---------------------------------------------------------------------------

/// Clear the `XUrgencyHint` flag in the `WM_HINTS` property of `win`.
pub fn clear_urgency_hint(x11: &X11BackendRef, win: WindowId) {
    let conn = x11.conn;
    let x11_win: Window = win.into();

    let Ok(cookie) =
        conn.get_property(false, x11_win, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9)
    else {
        return;
    };

    let Ok(reply) = cookie.reply() else { return };

    let mut data: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();

    if data.is_empty() {
        return;
    }

    data[0] &= !WM_HINTS_URGENCY_HINT;

    let _ = conn.change_property32(
        PropMode::REPLACE,
        x11_win,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        &data,
    );

    let _ = conn.flush();
}

// ---------------------------------------------------------------------------
// X11FocusBackend – implements `FocusBackendOps` for the X11 path
// ---------------------------------------------------------------------------

use crate::focus::FocusBackendOps;
/// X11 implementation of `FocusBackendOps`.
pub struct X11FocusBackend<'a> {
    pub x11: &'a X11BackendRef<'a>,
    pub x11_runtime: &'a mut X11RuntimeConfig,
}

impl<'a> FocusBackendOps for X11FocusBackend<'a> {
    fn unfocus_current(&self, state: &CoreState, current: WindowId) {
        unfocus_win(state, self.x11, &*self.x11_runtime, current, false);
    }

    fn focus_window(&self, ctx: &mut CoreCtx<'_>, win: WindowId) {
        let is_urgent = ctx
            .state()
            .model
            .clients
            .get(&win)
            .map(|c| c.is_urgent)
            .unwrap_or(false);
        if is_urgent {
            if let Some(c) = ctx.model_mut().clients.get_mut(&win) {
                c.clear_urgency();
            }
            clear_urgency_hint(self.x11, win);
        }
        set_focus(ctx.state_mut(), self.x11, &*self.x11_runtime, win);
    }

    fn focus_none(&self) {
        let _ = self.x11.conn.set_input_focus(
            InputFocus::POINTER_ROOT,
            self.x11_runtime.root,
            CURRENT_TIME,
        );
        let _ = self.x11.conn.delete_property(
            self.x11_runtime.root,
            self.x11_runtime.netatom.active_window,
        );
        let _ = self.x11.conn.flush();
    }

    fn on_desktop_binding_state_changed(&self, state: &CoreState) {
        crate::backend::x11::keyboard::grab_keys(state, self.x11, &*self.x11_runtime);
    }
}

/// X11-only focus helper for call sites that hold disaggregated X11 types
/// rather than a full `WmCtx`.
pub fn focus_soft(
    core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    win: Option<WindowId>,
) {
    let mut backend = X11FocusBackend { x11, x11_runtime };
    if let Err(e) = crate::focus::focus_generic(core, win, &mut backend) {
        log::warn!("focus_soft({:?}) failed: {}", win, e);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn read_wm_protocols(
    conn: &x11rb::rust_connection::RustConnection,
    win: Window,
    protocols_atom: u32,
) -> Vec<u32> {
    conn.get_property(false, win, protocols_atom, AtomEnum::ATOM, 0, 1024)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32().map(|it| it.collect()))
        .unwrap_or_default()
}

fn ungrab_button(
    conn: &x11rb::rust_connection::RustConnection,
    button: ButtonIndex,
    win: Window,
    modifiers: ModMask,
) -> Result<(), x11rb::errors::ConnectionError> {
    conn.ungrab_button(button, win, modifiers)?;
    Ok(())
}
