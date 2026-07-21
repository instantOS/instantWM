use crate::actions::{KeyAction, execute_key_action};
use crate::config::ModeConfig;
use crate::contexts::WmCtx;
use crate::core_state::ActiveWmMode;
use crate::floating::change_snap;
use crate::focus::{direction_focus, focus_stack};

use crate::types::*;
use crate::types::{Direction, StackDirection, VerticalDirection};
use std::collections::HashMap;

pub fn handle_keysym(ctx: &mut WmCtx, keysym: u32, mod_mask: u32) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = crate::util::clean_mask(mod_mask, numlockmask) as u16;

    if ctx.core().state().tree_placement.is_some() {
        use crate::config::keysyms::*;
        use crate::layouts::tree::Side;
        return match keysym {
            XK_LEFT => crate::layouts::step_keyboard_tree_placement(ctx, Side::Left),
            XK_RIGHT => crate::layouts::step_keyboard_tree_placement(ctx, Side::Right),
            XK_UP => crate::layouts::step_keyboard_tree_placement(ctx, Side::Top),
            XK_DOWN => crate::layouts::step_keyboard_tree_placement(ctx, Side::Bottom),
            XK_TAB => crate::layouts::cycle_keyboard_tree_placement(
                ctx,
                cleaned & crate::config::keybindings::SHIFT as u16 != 0,
            ),
            XK_SPACE => crate::layouts::center_keyboard_tree_placement(ctx),
            XK_RETURN => crate::layouts::finish_keyboard_tree_placement(ctx, true),
            XK_ESCAPE => crate::layouts::finish_keyboard_tree_placement(ctx, false),
            // Placement is modal: do not leak unrelated keys to a client while
            // a compositor command is awaiting confirmation.
            _ => true,
        };
    }

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
