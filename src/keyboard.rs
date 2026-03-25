use crate::actions::{KeyAction, execute_key_action};
use crate::backend::x11::X11BackendRef;
use crate::backend::x11::X11RuntimeConfig;
use crate::config::ModeConfig;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::floating::{
    SnapDir, change_snap, reset_snap, save_floating_geometry, set_overlay_mode, toggle_floating,
    unhide_one,
};
use crate::focus::{direction_focus, focus_stack};

use crate::layouts::arrange;
use crate::types::*;
use crate::types::{Direction, StackDirection};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn keycode_to_keysym<C: Connection>(conn: &C, keycode: u8, index: usize) -> u32 {
    if let Ok(cookie) = conn.get_keyboard_mapping(keycode, 1)
        && let Ok(reply) = cookie.reply()
        && index < reply.keysyms_per_keycode as usize
    {
        return reply.keysyms[index];
    }
    0
}

pub fn handle_keysym(ctx: &mut WmCtx, keysym: u32, mod_mask: u32) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = crate::util::clean_mask(mod_mask, numlockmask) as u16;

    let current_mode = ctx.current_mode().to_string();

    // Super + Escape always resets to default mode
    if !current_mode.is_empty()
        && current_mode != "default"
        && keysym == crate::config::keysyms::XK_ESCAPE
        && cleaned
            == crate::util::clean_mask(crate::config::keybindings::MODKEY, numlockmask) as u16
    {
        ctx.reset_mode();
        ctx.request_bar_update(None);
        return true;
    }

    let (action, transient) = resolve_key_action(
        ctx.core().globals().cfg.keys.as_slice(),
        ctx.core().globals().cfg.desktop_keybinds.as_slice(),
        &ctx.core().globals().cfg.modes,
        ctx.selected_client(),
        &current_mode,
        keysym,
        cleaned,
        numlockmask,
    )
    .map(|resolution| (Some(resolution.action), resolution.transient))
    .unwrap_or((None, false));

    if let Some(action) = action {
        execute_key_action(ctx, &action);
        if transient {
            ctx.reset_mode();
            ctx.request_bar_update(None);
        }
        true
    } else {
        false
    }
}

#[derive(Clone)]
struct KeyResolution {
    action: KeyAction,
    transient: bool,
}

pub(crate) fn desktop_bindings_enabled(
    selected_client: Option<WindowId>,
    current_mode: &str,
) -> bool {
    (!current_mode.is_empty() && current_mode != "default") || selected_client.is_none()
}

fn find_matching_action(
    keys: &[Key],
    keysym: u32,
    cleaned: u16,
    numlockmask: u32,
) -> Option<KeyAction> {
    keys.iter()
        .find(|key| {
            keysym == key.keysym
                && crate::util::clean_mask(key.mod_mask, numlockmask) as u16 == cleaned
        })
        .map(|key| key.action.clone())
}

fn resolve_key_action(
    keys: &[Key],
    desktop_keybinds: &[Key],
    modes: &std::collections::HashMap<String, ModeConfig>,
    selected_client: Option<WindowId>,
    current_mode: &str,
    keysym: u32,
    cleaned: u16,
    numlockmask: u32,
) -> Option<KeyResolution> {
    if !current_mode.is_empty() && current_mode != "default" {
        let mode_cfg = modes.get(current_mode);
        let transient = mode_cfg.is_some_and(|m| m.transient);
        if let Some(action) = mode_cfg
            .and_then(|mode| {
                find_matching_action(mode.keybinds.as_slice(), keysym, cleaned, numlockmask)
            })
            .or_else(|| find_matching_action(keys, keysym, cleaned, numlockmask))
            .or_else(|| find_matching_action(desktop_keybinds, keysym, cleaned, numlockmask))
        {
            return Some(KeyResolution { action, transient });
        }
        return None;
    }

    find_matching_action(keys, keysym, cleaned, numlockmask)
        .or_else(|| {
            if desktop_bindings_enabled(selected_client, current_mode) {
                find_matching_action(desktop_keybinds, keysym, cleaned, numlockmask)
            } else {
                None
            }
        })
        .map(|action| KeyResolution {
            action,
            transient: false,
        })
}

pub fn key_press_x11(ctx: &mut WmCtxX11, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;
    let keysym = keycode_to_keysym(ctx.x11.conn, keycode, 0);
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    let _ = handle_keysym(&mut wm_ctx, keysym, state.bits() as u32);
}

pub fn key_release_x11(_ctx: &mut WmCtxX11, _e: &KeyReleaseEvent) {}

fn grab_keys_for_key<C: Connection>(
    conn: &C,
    root: Window,
    modifiers: &[u16],
    key: &Key,
    keycode: u8,
) {
    for &modif in modifiers {
        let _ = grab_key(
            conn,
            false,
            root,
            ((key.mod_mask as u16) | modif).into(),
            keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
    }
}

pub fn grab_keys_x11(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    let conn = x11.conn;
    let root = x11_runtime.root;
    let numlockmask = x11_runtime.numlockmask;
    let keys = core.globals().cfg.keys.as_slice();
    let desktop_keybinds = core.globals().cfg.desktop_keybinds.as_slice();
    let modes = &core.globals().cfg.modes;

    let _ = ungrab_key(conn, 0, root, ModMask::ANY);

    let (keycode_min, keycode_max): (u8, u8) = (conn.setup().min_keycode, conn.setup().max_keycode);

    let modifiers: [u16; 4] = [
        0,
        ModMask::LOCK.bits(),
        numlockmask as u16,
        (numlockmask as u16) | ModMask::LOCK.bits(),
    ];

    let mapping = match conn
        .get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    {
        Some(mapping) => mapping,
        None => return,
    };

    let get_keysym = |keycode: u8| -> u32 {
        let index = (keycode - keycode_min) as usize * mapping.keysyms_per_keycode as usize;
        if index < mapping.keysyms.len() {
            mapping.keysyms[index]
        } else {
            0
        }
    };

    for keycode in keycode_min..=keycode_max {
        let keysym = get_keysym(keycode);
        if keysym == 0 {
            continue;
        }

        for key in keys {
            if keysym == key.keysym {
                grab_keys_for_key(conn, root, &modifiers, key, keycode);
            }
        }

        for mode in modes.values() {
            for key in &mode.keybinds {
                if keysym == key.keysym {
                    grab_keys_for_key(conn, root, &modifiers, key, keycode);
                }
            }
        }

        let current_mode = &core.globals().behavior.current_mode;
        let desktop_bindings_enabled =
            desktop_bindings_enabled(core.selected_client(), current_mode);

        if desktop_bindings_enabled {
            for key in desktop_keybinds {
                if keysym == key.keysym {
                    grab_keys_for_key(conn, root, &modifiers, key, keycode);
                }
            }
        }
    }

    let _ = conn.flush();
}

pub fn update_num_lock_mask_x11(
    _core: &mut CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
) {
    let new_numlockmask = {
        let conn = x11.conn;
        let Ok(cookie) = conn.get_modifier_mapping() else {
            return;
        };
        let Ok(reply) = cookie.reply() else {
            return;
        };
        let (keycode_min, keycode_max) = (conn.setup().min_keycode, conn.setup().max_keycode);
        let mapping = match conn
            .get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        {
            Some(mapping) => mapping,
            None => return,
        };

        let mut new_numlockmask: u32 = 0;
        for (i, keycode) in reply.keycodes.iter().enumerate() {
            if *keycode >= keycode_min && *keycode <= keycode_max {
                let idx = (*keycode - keycode_min) as usize * mapping.keysyms_per_keycode as usize;
                let keysym = if idx < mapping.keysyms.len() {
                    mapping.keysyms[idx]
                } else {
                    0
                };
                // XK_Num_Lock keysym (X11 keysym 0xff7f) — used to detect
                // which modifier bit corresponds to Num Lock so we can mask
                // it out when matching keybindings.
                if keysym == 0xff7f {
                    let mod_index = i / reply.keycodes_per_modifier() as usize;
                    // X11 supports at most 8 modifier bits (Mod1–Mod5 + Shift/Control/Lock).
                    if mod_index < 8 {
                        new_numlockmask = 1 << mod_index;
                    }
                }
            }
        }

        new_numlockmask
    };

    x11_runtime.numlockmask = new_numlockmask;
}

pub fn up_press(ctx: &mut WmCtx) {
    let (selected_window, overlay_win, is_floating) = {
        let mon = ctx.core().globals().selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let is_floating = sel.is_some_and(|w| ctx.core().globals().clients.is_floating(w));
        (sel, overlay, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if selected_window == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Top);
        return;
    }

    if is_floating {
        toggle_floating(ctx);
        return;
    }

    if let Some(win) = selected_window {
        crate::client::hide(ctx, win);
    }
}

pub fn down_press(ctx: &mut WmCtx) {
    if unhide_one(ctx) {
        return;
    }

    let (selected_window, overlay_win, snap_status, is_floating) = {
        let mon = ctx.core().globals().selected_monitor();
        let sel = mon.sel;
        let overlay = mon.overlay;
        let (snap_status, is_floating) = sel
            .and_then(|w| {
                ctx.core()
                    .globals()
                    .clients
                    .get(&w)
                    .map(|c| (c.snap_status, c.is_floating))
            })
            .unwrap_or((SnapPosition::None, false));
        (sel, overlay, snap_status, is_floating)
    };

    if selected_window.is_none() {
        return;
    }

    if snap_status != SnapPosition::None {
        if let Some(win) = selected_window {
            reset_snap(ctx, win);
        }
        return;
    }

    if selected_window == overlay_win {
        set_overlay_mode(ctx, OverlayMode::Bottom);
        return;
    }

    if !is_floating {
        toggle_floating(ctx);
    }
}

pub fn up_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = !ctx.core().globals().selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus(ctx, Direction::Up);
        return;
    }

    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.selected_client() {
            if let WmCtx::X11(x11_ctx) = ctx {
                crate::client::refresh_border_color_x11(
                    &x11_ctx.core,
                    &x11_ctx.x11,
                    x11_ctx.x11_runtime,
                    win,
                    false,
                );
            }
            change_snap(ctx, win, SnapDir::Up);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn down_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = !ctx.core().globals().selected_monitor().is_tiling_layout();

    if is_overview {
        direction_focus(ctx, Direction::Down);
        return;
    }

    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.selected_client() {
            change_snap(ctx, win, SnapDir::Down);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn space_toggle(ctx: &mut WmCtx) {
    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if !has_tiling {
        let Some(win) = ctx.selected_client() else {
            return;
        };

        let snap_status = {
            ctx.core()
                .globals()
                .clients
                .get(&win)
                .map(|c| c.snap_status)
                .unwrap_or(SnapPosition::None)
        };

        if snap_status != SnapPosition::None {
            reset_snap(ctx, win);
        } else {
            let border_width = ctx.core().globals().cfg.border_width_px;
            ctx.set_border(win, border_width);

            if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
                save_floating_geometry(client);
                client.snap_status = SnapPosition::Maximized;
            }

            let selmon_id = ctx.core().globals().selected_monitor_id();
            arrange(ctx, Some(selmon_id));
        }
    } else {
        toggle_floating(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::NamedAction;

    fn named(action: NamedAction) -> KeyAction {
        KeyAction::Named {
            action,
            args: Vec::new(),
        }
    }

    #[test]
    fn resolve_key_action_prefers_mode_binding_and_marks_transient() {
        let mode_key = Key {
            mod_mask: 1,
            keysym: 42,
            action: named(NamedAction::FocusNext),
        };
        let global_key = Key {
            mod_mask: 1,
            keysym: 42,
            action: named(NamedAction::FocusPrev),
        };
        let mut modes = std::collections::HashMap::new();
        modes.insert(
            "resize".to_string(),
            ModeConfig {
                description: None,
                transient: true,
                keybinds: vec![mode_key],
            },
        );

        let resolved = resolve_key_action(&[global_key], &[], &modes, None, "resize", 42, 1, 0)
            .expect("expected action");

        match resolved.action {
            KeyAction::Named { action, .. } => assert_eq!(action, NamedAction::FocusNext),
            _ => panic!("unexpected action kind"),
        }
        assert!(resolved.transient);
    }

    #[test]
    fn resolve_key_action_uses_desktop_bindings_only_without_selected_client() {
        let desktop_key = Key {
            mod_mask: 0,
            keysym: 9,
            action: named(NamedAction::ToggleLayout),
        };

        let resolved = resolve_key_action(
            &[],
            &[desktop_key],
            &std::collections::HashMap::new(),
            None,
            "default",
            9,
            0,
            0,
        )
        .expect("expected desktop action");

        match resolved.action {
            KeyAction::Named { action, .. } => assert_eq!(action, NamedAction::ToggleLayout),
            _ => panic!("unexpected action kind"),
        }

        let blocked = resolve_key_action(
            &[],
            &[Key {
                mod_mask: 0,
                keysym: 9,
                action: named(NamedAction::ToggleLayout),
            }],
            &std::collections::HashMap::new(),
            Some(WindowId(1)),
            "default",
            9,
            0,
            0,
        );
        assert!(blocked.is_none());
    }

    #[test]
    fn desktop_bindings_enabled_in_non_default_mode_even_with_selection() {
        assert!(desktop_bindings_enabled(Some(WindowId(1)), "resize"));
        assert!(!desktop_bindings_enabled(Some(WindowId(1)), "default"));
        assert!(desktop_bindings_enabled(None, "default"));
    }
}
