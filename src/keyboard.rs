use crate::actions::{KeyAction, execute_key_action};
use crate::config::ModeConfig;
use crate::contexts::WmCtx;
use crate::core_state::ActiveWmMode;
use crate::floating::change_snap;
use crate::focus::{direction_focus, focus_stack};

use crate::types::*;
use crate::types::{Direction, StackDirection, VerticalDirection};
use std::collections::HashMap;

fn normalize_binding_keysym(keysym: u32) -> u32 {
    if (b'A' as u32..=b'Z' as u32).contains(&keysym) {
        keysym + u32::from(b'a' - b'A')
    } else {
        keysym
    }
}

fn is_modifier_keysym(keysym: u32) -> bool {
    use crate::config::keysyms::*;
    matches!(
        keysym,
        XK_SHIFT_L | XK_SHIFT_R | XK_CONTROL_L | XK_CONTROL_R | XK_SUPER_L | XK_SUPER_R
    )
}

pub fn handle_keysym(ctx: &mut WmCtx, keysym: u32, mod_mask: u32) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = crate::util::clean_mask(mod_mask, numlockmask) as u16;
    let placement_active = matches!(ctx.current_mode(), ActiveWmMode::TreePlacement(_));
    // Super may still be held after the chord that entered placement. Treat it
    // as an entry modifier, not part of commands within the mode.
    let binding_mask = if placement_active {
        cleaned & !(crate::config::keybindings::MODKEY as u16)
    } else {
        cleaned
    };
    let binding_keysym = normalize_binding_keysym(keysym);

    // Super + Escape always resets to default mode
    if !matches!(ctx.current_mode(), ActiveWmMode::Default)
        && keysym == crate::config::keysyms::XK_ESCAPE
        && cleaned
            == crate::util::clean_mask(crate::config::keybindings::MODKEY, numlockmask) as u16
    {
        ctx.reset_mode();
        return true;
    }

    let (action, transient) = resolve_key_action(
        ctx.core().config().bindings.keys.as_slice(),
        ctx.core()
            .state()
            .config
            .bindings
            .desktop_keybinds
            .as_slice(),
        &ctx.core().config().bindings.modes,
        ctx.core().model().selected_win(),
        ctx.current_mode(),
        binding_keysym,
        binding_mask,
        numlockmask,
    )
    .map(|resolution| (Some(resolution.action), resolution.transient))
    .unwrap_or((None, false));

    if let Some(action) = action {
        execute_key_action(ctx, &action);
        if transient {
            ctx.reset_mode();
        }
        true
    } else if placement_active {
        // Modifier presses are part of forming the next chord. Every other
        // unbound key cancels and is consumed so it cannot leak to a client.
        if !is_modifier_keysym(keysym) {
            crate::layouts::finish_keyboard_tree_placement(ctx, false);
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
    mode: &ActiveWmMode,
) -> bool {
    !matches!(mode, ActiveWmMode::Default) || selected_client.is_none()
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
    modes: &HashMap<String, ModeConfig>,
    selected_client: Option<WindowId>,
    mode: &ActiveWmMode,
    keysym: u32,
    cleaned: u16,
    numlockmask: u32,
) -> Option<KeyResolution> {
    let find = |binds: &[Key]| find_matching_action(binds, keysym, cleaned, numlockmask);

    match mode {
        ActiveWmMode::TreePlacement(_) => modes
            .get(crate::core_state::TREE_PLACEMENT_MODE_NAME)
            .and_then(|mode| find(&mode.keybinds))
            .map(|action| KeyResolution {
                action,
                transient: false,
            }),
        ActiveWmMode::Named(name) => {
            let mode_cfg = modes.get(name.as_str());
            let transient = mode_cfg.is_some_and(|m| m.transient);
            let action = mode_cfg
                .and_then(|m| find(&m.keybinds))
                .or_else(|| find(keys))
                .or_else(|| find(desktop_keybinds));
            action.map(|action| KeyResolution { action, transient })
        }
        _ => {
            // Default & Overview: global bindings → desktop bindings (if enabled)
            find(keys)
                .or_else(|| {
                    if desktop_bindings_enabled(selected_client, mode) {
                        find(desktop_keybinds)
                    } else {
                        None
                    }
                })
                .map(|action| KeyResolution {
                    action,
                    transient: false,
                })
        }
    }
}

pub fn up_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = ctx.core().model().is_overview_active();

    if is_overview {
        direction_focus(ctx, VerticalDirection::Up.into());
        return;
    }

    let has_tiling = ctx.core().model().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.core().model().selected_win() {
            if let WmCtx::X11(x11_ctx) = ctx {
                crate::backend::x11::focus::refresh_border_color(
                    x11_ctx.core.state(),
                    &x11_ctx.x11,
                    x11_ctx.x11_runtime,
                    win,
                    false,
                );
            }
            change_snap(ctx, win, Direction::Up);
        }
        return;
    }

    focus_stack(ctx, direction);
}

pub fn down_key(ctx: &mut WmCtx, direction: StackDirection) {
    let is_overview = ctx.core().model().is_overview_active();

    if is_overview {
        direction_focus(ctx, VerticalDirection::Down.into());
        return;
    }

    let has_tiling = ctx.core().model().selected_monitor().is_tiling_layout();

    if !has_tiling {
        if let Some(win) = ctx.core().model().selected_win() {
            change_snap(ctx, win, Direction::Down);
        }
        return;
    }

    focus_stack(ctx, direction);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::NamedAction;
    use crate::core_state::ActiveWmMode;

    fn placement_mode() -> ActiveWmMode {
        let state = crate::core_state::KeyboardTreePlacement::new(
            WindowId(1),
            MonitorId::default(),
            TagMask::EMPTY,
            vec![crate::layouts::tree::PlacementTarget {
                target: WindowId(2),
                side: None,
                candidate_index: 0,
                position: Point::new(0, 0),
            }],
            0,
        )
        .expect("valid placement test state");
        ActiveWmMode::TreePlacement(state)
    }

    #[test]
    fn key_normalization_handles_shifted_letters_and_modifier_keys() {
        use crate::config::keysyms::*;

        assert_eq!(normalize_binding_keysym(XK_H_UPPER), XK_H);
        assert!(is_modifier_keysym(XK_SHIFT_L));
        assert!(is_modifier_keysym(XK_CONTROL_R));
        assert!(!is_modifier_keysym(XK_Q));
    }

    #[test]
    fn resolve_key_action_prefers_mode_binding_and_marks_transient() {
        let mode_key = Key {
            mod_mask: 1,
            keysym: 42,
            action: KeyAction::named(NamedAction::FocusNext),
        };
        let global_key = Key {
            mod_mask: 1,
            keysym: 42,
            action: KeyAction::named(NamedAction::FocusPrev),
        };
        let mut modes = HashMap::new();
        modes.insert(
            "resize".to_string(),
            ModeConfig {
                description: None,
                transient: true,
                keybinds: vec![mode_key],
            },
        );

        let resolved = resolve_key_action(
            &[global_key],
            &[],
            &modes,
            None,
            &ActiveWmMode::Named("resize".to_string()),
            42,
            1,
            0,
        )
        .expect("expected action");

        match resolved.action {
            KeyAction::Named { action, .. } => assert_eq!(action, NamedAction::FocusNext),
            _ => panic!("unexpected action kind"),
        }
        assert!(resolved.transient);
    }

    #[test]
    fn placement_resolves_only_its_configured_mode_actions() {
        let placement_key = Key {
            mod_mask: 0,
            keysym: 42,
            action: KeyAction::named(NamedAction::PlacementLeft),
        };
        let global_key = Key {
            mod_mask: 0,
            keysym: 43,
            action: KeyAction::named(NamedAction::FocusNext),
        };
        let global_keys = [global_key];
        let mut modes = HashMap::new();
        modes.insert(
            crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string(),
            ModeConfig {
                description: None,
                transient: true,
                keybinds: vec![placement_key],
            },
        );
        let mode = placement_mode();

        let resolved = resolve_key_action(&global_keys, &[], &modes, None, &mode, 42, 0, 0)
            .expect("configured placement action");
        assert!(matches!(
            resolved.action,
            KeyAction::Named {
                action: NamedAction::PlacementLeft,
                ..
            }
        ));
        assert!(
            !resolved.transient,
            "placement is intrinsically non-transient"
        );
        assert!(resolve_key_action(&global_keys, &[], &modes, None, &mode, 43, 0, 0).is_none());
    }

    #[test]
    fn resolve_key_action_uses_desktop_bindings_only_without_selected_client() {
        let desktop_key = Key {
            mod_mask: 0,
            keysym: 9,
            action: KeyAction::named(NamedAction::ToggleLayout),
        };

        let resolved = resolve_key_action(
            &[],
            &[desktop_key],
            &HashMap::new(),
            None,
            &ActiveWmMode::Default,
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
                action: KeyAction::named(NamedAction::ToggleLayout),
            }],
            &HashMap::new(),
            Some(WindowId(1)),
            &ActiveWmMode::Default,
            9,
            0,
            0,
        );
        assert!(blocked.is_none());
    }

    #[test]
    fn resolve_key_action_overview_ignores_configured_overview_mode() {
        // A user-configured mode whose name collides with the built-in overview.
        // It must NOT be consulted while the WM is in Overview mode.
        let overview_mode_key = Key {
            mod_mask: 1,
            keysym: 42,
            action: KeyAction::named(NamedAction::FocusPrev),
        };
        let global_key = Key {
            mod_mask: 1,
            keysym: 42,
            action: KeyAction::named(NamedAction::FocusNext),
        };
        let mut modes = HashMap::new();
        modes.insert(
            crate::overview::OVERVIEW_MODE_NAME.to_string(),
            ModeConfig {
                description: None,
                transient: false,
                keybinds: vec![overview_mode_key],
            },
        );

        // The global binding wins; the configured "overview" mode is ignored.
        let resolved = resolve_key_action(
            &[global_key],
            &[],
            &modes,
            None,
            &ActiveWmMode::Overview,
            42,
            1,
            0,
        )
        .expect("expected global action in overview");
        match resolved.action {
            KeyAction::Named { action, .. } => assert_eq!(action, NamedAction::FocusNext),
            _ => panic!("unexpected action kind"),
        }
        assert!(!resolved.transient);
    }

    #[test]
    fn desktop_bindings_enabled_in_non_default_mode_even_with_selection() {
        assert!(desktop_bindings_enabled(
            Some(WindowId(1)),
            &ActiveWmMode::Named("resize".to_string())
        ));
        // Overview is a built-in non-default mode: desktop bindings stay enabled.
        assert!(desktop_bindings_enabled(
            Some(WindowId(1)),
            &ActiveWmMode::Overview
        ));
        assert!(!desktop_bindings_enabled(
            Some(WindowId(1)),
            &ActiveWmMode::Default
        ));
        assert!(desktop_bindings_enabled(None, &ActiveWmMode::Default));
    }
}
