//! Keyboard input handling for Wayland compositor.

pub(crate) mod recovery;

#[cfg(test)]
mod virtual_keyboard_tests;

use crate::contexts::WmCtx;
use smithay::backend::input::{Event, InputBackend, InputEvent, KeyboardKeyEvent};
use smithay::input::keyboard::{FilterResult, KeyboardHandle, KeyboardSource};
use smithay::wayland::input_method::InputMethodSeat;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::virtual_keyboard::{
    VirtualKeyboardBackend, VirtualKeyboardDevice, VirtualKeyboardSpecialEvent,
};

use crate::backend::wayland::compositor::WaylandState;
use crate::backend::wayland::compositor::layer_shell::LayerKeyboardPolicy;
use crate::backend::wayland::input::modifiers_to_x11_mask;
use crate::types::Keysym;

use smithay::utils::SERIAL_COUNTER;

fn is_shortcut_recovery_chord(
    raw_keysym: Keysym,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> bool {
    raw_keysym == crate::config::keysyms::XK_ESCAPE
        && modifiers.logo
        && modifiers.shift
        && !modifiers.ctrl
        && !modifiers.alt
}

/// Why ordinary WM keybindings are withheld for the current key event.
///
/// Emergency VT switching and dismissal of an open compositor-owned menu are
/// handled separately before ordinary keybinding dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutSuppression {
    SessionLocked,
    /// A Smithay grab other than the compositor's input-method grab.
    KeyboardGrab,
    /// An active inhibitor whose associated surface currently has focus.
    SurfaceInhibitor,
    ExclusiveLayer,
}

/// Protocol and focus facts used to derive shortcut policy in one place.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ShortcutContext {
    session_locked: bool,
    keyboard_grabbed: bool,
    input_method_grabbed: bool,
    focused_surface_inhibited: bool,
    exclusive_layer: bool,
}

fn select_shortcut_suppression(context: ShortcutContext) -> Option<ShortcutSuppression> {
    if context.session_locked {
        Some(ShortcutSuppression::SessionLocked)
    } else if context.keyboard_grabbed && !context.input_method_grabbed {
        Some(ShortcutSuppression::KeyboardGrab)
    } else if context.focused_surface_inhibited {
        Some(ShortcutSuppression::SurfaceInhibitor)
    } else if context.exclusive_layer {
        Some(ShortcutSuppression::ExclusiveLayer)
    } else {
        None
    }
}

fn shortcut_suppression(
    state: &WaylandState,
    keyboard: &KeyboardHandle<WaylandState>,
) -> Option<ShortcutSuppression> {
    let session_locked = state.native.is_locked();
    let keyboard_grabbed = !session_locked && keyboard.is_grabbed();
    let input_method_grabbed =
        keyboard_grabbed && state.native.seat.input_method().keyboard_grabbed();
    // Lower-priority surface policy cannot change a lock/grab decision.
    let focus = if session_locked || (keyboard_grabbed && !input_method_grabbed) {
        None
    } else {
        keyboard.current_focus()
    };
    let focused_surface_inhibited = focus
        .as_ref()
        .and_then(WaylandFocus::wl_surface)
        .and_then(|surface| {
            state
                .native
                .seat
                .keyboard_shortcuts_inhibitor_for_surface(surface.as_ref())
        })
        .is_some_and(|inhibitor| inhibitor.is_active());
    let exclusive_layer = !focused_surface_inhibited
        && focus
            .as_ref()
            .and_then(WaylandFocus::wl_surface)
            .and_then(|surface| state.layer_keyboard_policy(surface.as_ref()))
            .is_some_and(LayerKeyboardPolicy::suppresses_wm_shortcuts);
    select_shortcut_suppression(ShortcutContext {
        session_locked,
        keyboard_grabbed,
        input_method_grabbed,
        focused_surface_inhibited,
        exclusive_layer,
    })
}

/// Detect if a key event corresponds to a VT switch (Ctrl+Alt+F1..F12 or XF86Switch_VT_1..12).
pub fn vt_switch_target(
    raw_keysym: Keysym,
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> Option<i32> {
    use crate::config::keysyms::*;

    // The XF86 and F1..F12 VT-switch keysyms are contiguous ranges, so the
    // target is the offset within the range plus one.
    let raw = raw_keysym.raw();
    if (XF86XK_SWITCH_VT_1.raw()..=XF86XK_SWITCH_VT_12.raw()).contains(&raw) {
        return Some((raw - XF86XK_SWITCH_VT_1.raw() + 1) as i32);
    }

    // Standard Ctrl + Alt + F1..F12
    if modifiers.ctrl && modifiers.alt && (XK_F1.raw()..=XK_F12.raw()).contains(&raw) {
        return Some((raw - XK_F1.raw() + 1) as i32);
    }

    None
}

/// Handle keyboard events attributed to a specific xkb input source.
///
/// Physical devices share [`KeyboardSource::MAIN`]; every virtual keyboard
/// carries its own source, so a keycode held through one device is not released
/// by another device releasing the same code.
pub fn handle_keyboard<B: InputBackend>(
    state: &mut WaylandState,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    source: KeyboardSource,
    event: impl KeyboardKeyEvent<B>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    // Protocols can install keyboard grabs without a compositor focus
    // request. Reassert the lock policy before any key can reach such a grab.
    if state.native.is_locked() {
        state.set_keyboard_focus(None, serial);
    }
    let key_code = event.key_code();
    let key_state = event.state();
    let time = event.time();
    // Smithay drops the keyboard lock before invoking this filter, so `data`
    // borrows the compositor directly for shared WM actions; no pointer bridge
    // or second compositor borrow is needed. Run actions before forwarding,
    // since they can change focus and interception.
    // Do not move this work into SeatHandler::focus_changed: that callback
    // runs with the keyboard lock held. `input_intercept` reports event pairs
    // that are not a transition for this source (a keycode already held or
    // released on it, or held elsewhere) as absorbed, so no binding runs and
    // nothing is forwarded twice.
    let intercepted = keyboard_handle.input_intercept(
        source,
        state,
        key_code,
        key_state,
        |data, modifiers, keysym| {
            if key_state == smithay::backend::input::KeyState::Released {
                if data.release_shortcut_recovery_key(key_code) {
                    data.native
                        .runtime
                        .intercepted_key_releases
                        .remove(&key_code);
                    return FilterResult::Intercept(());
                }
                if data.shortcut_recovery_is_armed() && (!modifiers.logo || !modifiers.shift) {
                    data.cancel_shortcut_recovery();
                }
                if data
                    .native
                    .runtime
                    .intercepted_key_releases
                    .remove(&key_code)
                {
                    return FilterResult::Intercept(());
                }
                return FilterResult::Forward;
            }
            let raw_keysym = keysym
                .raw_syms()
                .first()
                .map_or(Keysym::NONE, |ks| Keysym::new(ks.raw()));

            // Key-repeat events remain compositor-owned after recovery has
            // completed, until the matching physical release arrives.
            if data.shortcut_recovery_owns_key(key_code) {
                return FilterResult::Intercept(());
            }

            // Any additional press changes the deliberate recovery chord.
            // Cancel the gesture, then route that extra key normally.
            if data.shortcut_recovery_is_armed() {
                data.cancel_shortcut_recovery();
            }

            let suppression = shortcut_suppression(data, keyboard_handle);
            if matches!(
                suppression,
                Some(ShortcutSuppression::KeyboardGrab | ShortcutSuppression::SurfaceInhibitor)
            ) && is_shortcut_recovery_chord(raw_keysym, modifiers)
                && let Some(surface) = keyboard_handle
                    .current_focus()
                    .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()))
            {
                data.arm_shortcut_recovery(key_code, surface);
                data.native
                    .runtime
                    .intercepted_key_releases
                    .insert(key_code);
                return FilterResult::Intercept(());
            }

            // Emergency VT switching (Ctrl+Alt+F1..F12 or XF86Switch_VT_1..12)
            // This is processed unconditionally before window grabs and shortcut suppression.
            if let Some(vt) = vt_switch_target(raw_keysym, modifiers)
                && data.switch_vt(vt)
            {
                data.native
                    .runtime
                    .intercepted_key_releases
                    .insert(key_code);
                return FilterResult::Intercept(());
            }

            if raw_keysym == crate::config::keysyms::XK_ESCAPE {
                let closed_native = data.dismiss_native_systray_menu();
                let closed_hosted =
                    crate::backend::wayland::input::bar::close_systray_menu(&mut data.wm);
                if closed_hosted {
                    data.native.request_bar_redraw();
                }
                if closed_native || closed_hosted {
                    data.native
                        .runtime
                        .intercepted_key_releases
                        .insert(key_code);
                    return FilterResult::Intercept(());
                }
            }
            if suppression.is_none() {
                let mod_mask = modifiers_to_x11_mask(modifiers);
                let ctx = data.ctx();
                let WmCtx::Wayland(ctx) = ctx else {
                    return FilterResult::Forward;
                };
                let mut wm_ctx = WmCtx::Wayland(ctx);
                if crate::keyboard::handle_keysym(&mut wm_ctx, raw_keysym, mod_mask) {
                    data.native
                        .runtime
                        .intercepted_key_releases
                        .insert(key_code);
                    return FilterResult::Intercept(());
                }
            }
            FilterResult::Forward
        },
    );
    let Some((filter_result, mods_changed)) = intercepted else {
        return;
    };
    if matches!(filter_result, FilterResult::Forward) {
        keyboard_handle.input_forward(state, key_code, key_state, serial, time, mods_changed);
    }
}

/// Handle an event from a `zwp_virtual_keyboard_v1` device.
///
/// A virtual keyboard is an ordinary input device with its own keymap and
/// key-tracking source, so its keys take the same path as physical ones with
/// two differences: the keymap it sent has to be activated before its keycodes
/// mean anything, and a client that holds the keyboard grab and forwards keys
/// through its own virtual keyboard must not have those keys delivered back
/// into that grab.
///
/// These events are client-generated, so they deliberately do not count as
/// user activity: a synthetic key from `wtype` must not keep a screen awake.
pub fn handle_virtual_keyboard_event(
    state: &mut WaylandState,
    event: InputEvent<VirtualKeyboardBackend>,
) {
    match event {
        // The device becomes usable with its first keymap request; later
        // requests replace it.
        InputEvent::DeviceAdded { device }
        | InputEvent::Special(VirtualKeyboardSpecialEvent::KeymapChanged { device }) => {
            activate_virtual_keyboard_keymap(state, &device);
        }
        InputEvent::Keyboard { event } => {
            let device = Event::device(&event);
            let keyboard = state.native.keyboard.clone();
            // An input method holds a keyboard grab and forwards every key it
            // did not handle through its own virtual keyboard. Delivering those
            // into the grab hands them back to their sender, so they bypass
            // both the grab and this compositor's filter: the key already ran
            // through both on its way to the grab client.
            let sender_holds_grab = device.client().is_some_and(|client| {
                state.native.seat.input_method().keyboard_grab_client() == Some(client)
            });
            if sender_holds_grab {
                keyboard.input_forward_bypassing_grab(
                    state,
                    event.key_code(),
                    event.state(),
                    SERIAL_COUNTER.next_serial(),
                    event.time(),
                    true,
                );
                return;
            }
            handle_keyboard(state, &keyboard, device.source(), event);
        }
        InputEvent::Special(VirtualKeyboardSpecialEvent::Modifiers {
            mods_depressed,
            mods_latched,
            mods_locked,
            group,
            ..
        }) => {
            let keyboard = state.native.keyboard.clone();
            keyboard.with_xkb_state(state, |mut context| {
                context.set_modifier_mask(mods_depressed, mods_latched, mods_locked, group);
            });
        }
        // Keys the device still held were released as ordinary key events
        // before this one. What remains is the seat keymap: if the device
        // installed one, put the compositor's layout back, which also rebuilds
        // the xkb state and clears the masks it applied through `modifiers`.
        //
        // A device that sent `no_keymap` never replaced the seat keymap, and
        // its masks are left alone: they are absolute and shared with the
        // physical keyboard, so clearing them would drop modifiers the virtual
        // keyboard did not set.
        InputEvent::DeviceRemoved { device } if device.keymap().is_some() => {
            state.restore_keyboard_layout();
        }
        _ => {}
    }
}

/// Activate the keymap a virtual keyboard sent so its keycodes resolve.
fn activate_virtual_keyboard_keymap(state: &mut WaylandState, device: &VirtualKeyboardDevice) {
    // `no_keymap` means the client sends keycodes for the seat keymap, so there
    // is nothing to activate.
    let Some(keymap) = device.keymap() else {
        return;
    };
    let keyboard = state.native.keyboard.clone();
    if let Err(err) = keyboard.set_keymap_from_string(state, keymap.to_string()) {
        log::warn!("Failed to activate virtual keyboard keymap: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::keysyms::*;
    use smithay::input::keyboard::ModifiersState;

    #[test]
    fn shortcut_suppression_requires_an_explicit_input_policy() {
        assert_eq!(
            select_shortcut_suppression(ShortcutContext::default()),
            None
        );
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                exclusive_layer: true,
                ..ShortcutContext::default()
            }),
            Some(ShortcutSuppression::ExclusiveLayer)
        );
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                focused_surface_inhibited: true,
                ..ShortcutContext::default()
            }),
            Some(ShortcutSuppression::SurfaceInhibitor)
        );
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                keyboard_grabbed: true,
                ..ShortcutContext::default()
            }),
            Some(ShortcutSuppression::KeyboardGrab)
        );
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                session_locked: true,
                ..ShortcutContext::default()
            }),
            Some(ShortcutSuppression::SessionLocked)
        );
    }

    #[test]
    fn input_method_grab_alone_keeps_compositor_shortcuts_available() {
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                keyboard_grabbed: true,
                input_method_grabbed: true,
                ..ShortcutContext::default()
            }),
            None
        );
    }

    #[test]
    fn security_and_protocol_reasons_have_stable_precedence() {
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                session_locked: true,
                keyboard_grabbed: true,
                input_method_grabbed: true,
                focused_surface_inhibited: true,
                exclusive_layer: true,
            }),
            Some(ShortcutSuppression::SessionLocked)
        );
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                keyboard_grabbed: true,
                focused_surface_inhibited: true,
                exclusive_layer: true,
                ..ShortcutContext::default()
            }),
            Some(ShortcutSuppression::KeyboardGrab)
        );
        assert_eq!(
            select_shortcut_suppression(ShortcutContext {
                focused_surface_inhibited: true,
                exclusive_layer: true,
                ..ShortcutContext::default()
            }),
            Some(ShortcutSuppression::SurfaceInhibitor)
        );
    }

    #[test]
    fn test_vt_switch_direct_keysyms() {
        let empty_mods = ModifiersState::default();
        assert_eq!(vt_switch_target(XF86XK_SWITCH_VT_1, &empty_mods), Some(1));
        assert_eq!(vt_switch_target(XF86XK_SWITCH_VT_3, &empty_mods), Some(3));
        assert_eq!(vt_switch_target(XF86XK_SWITCH_VT_7, &empty_mods), Some(7));
        assert_eq!(vt_switch_target(XF86XK_SWITCH_VT_12, &empty_mods), Some(12));
    }

    #[test]
    fn test_vt_switch_ctrl_alt_function_keys() {
        let mods = ModifiersState {
            ctrl: true,
            alt: true,
            ..ModifiersState::default()
        };

        assert_eq!(vt_switch_target(XK_F1, &mods), Some(1));
        assert_eq!(vt_switch_target(XK_F2, &mods), Some(2));
        assert_eq!(vt_switch_target(XK_F3, &mods), Some(3));
        assert_eq!(vt_switch_target(XK_F7, &mods), Some(7));
        assert_eq!(vt_switch_target(XK_F12, &mods), Some(12));

        // Missing ctrl
        let no_ctrl = ModifiersState {
            alt: true,
            ..ModifiersState::default()
        };
        assert_eq!(vt_switch_target(XK_F3, &no_ctrl), None);

        // Missing alt
        let no_alt = ModifiersState {
            ctrl: true,
            ..ModifiersState::default()
        };
        assert_eq!(vt_switch_target(XK_F3, &no_alt), None);

        // Different key
        assert_eq!(vt_switch_target(XK_ESCAPE, &mods), None);
        assert_eq!(vt_switch_target(XK_A, &mods), None);
    }

    #[test]
    fn recovery_chord_is_exact_and_deliberate() {
        let chord = ModifiersState {
            logo: true,
            shift: true,
            ..ModifiersState::default()
        };
        assert!(is_shortcut_recovery_chord(XK_ESCAPE, &chord));
        assert!(!is_shortcut_recovery_chord(XK_Q, &chord));
        assert!(!is_shortcut_recovery_chord(
            XK_ESCAPE,
            &ModifiersState {
                logo: true,
                ..ModifiersState::default()
            }
        ));
        assert!(!is_shortcut_recovery_chord(
            XK_ESCAPE,
            &ModifiersState {
                ctrl: true,
                ..chord
            }
        ));
    }
}
