use crate::actions::{KeyAction, execute_key_action};
use crate::config::ModeConfig;
use crate::contexts::WmCtx;
use crate::core_state::ActiveWmMode;
use crate::floating::change_snap;
use crate::focus::{direction_focus, focus_stack};

use crate::types::*;
use crate::types::{Direction, StackDirection, VerticalDirection};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreePlacementCommand {
    Navigate(crate::layouts::tree::Side),
    Swap(crate::layouts::tree::Side),
    Resize(crate::layouts::tree::Side),
    Cycle { backwards: bool },
    Center,
    Apply,
    Cancel,
    Modifier,
}

fn tree_placement_command(keysym: u32, cleaned: u16) -> TreePlacementCommand {
    use crate::config::keybindings::{CONTROL, MODKEY, SHIFT};
    use crate::config::keysyms::*;
    use crate::layouts::tree::Side;

    if matches!(
        keysym,
        XK_SHIFT_L | XK_SHIFT_R | XK_CONTROL_L | XK_CONTROL_R | XK_SUPER_L | XK_SUPER_R
    ) {
        return TreePlacementCommand::Modifier;
    }

    // Super may still be held from the chord that entered placement. It is
    // optional inside the mode; every other modifier is part of the command.
    let modifiers = u32::from(cleaned) & !MODKEY;
    let direction = match keysym {
        XK_LEFT | XK_H | XK_H_UPPER => Some(Side::Left),
        XK_RIGHT | XK_L | XK_L_UPPER => Some(Side::Right),
        XK_UP | XK_K | XK_K_UPPER => Some(Side::Top),
        XK_DOWN | XK_J | XK_J_UPPER => Some(Side::Bottom),
        _ => None,
    };
    if let Some(side) = direction {
        return match modifiers {
            0 => TreePlacementCommand::Navigate(side),
            SHIFT => TreePlacementCommand::Swap(side),
            CONTROL => TreePlacementCommand::Resize(side),
            _ => TreePlacementCommand::Cancel,
        };
    }

    match (keysym, modifiers) {
        (XK_TAB, 0) => TreePlacementCommand::Cycle { backwards: false },
        (XK_TAB, SHIFT) => TreePlacementCommand::Cycle { backwards: true },
        (XK_SPACE, 0) => TreePlacementCommand::Center,
        (XK_RETURN, 0) => TreePlacementCommand::Apply,
        // Escape and every unrelated key cancel and are consumed. This avoids
        // accidentally typing the cancelling key into the focused client.
        _ => TreePlacementCommand::Cancel,
    }
}

pub fn handle_keysym(ctx: &mut WmCtx, keysym: u32, mod_mask: u32) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = crate::util::clean_mask(mod_mask, numlockmask) as u16;

    if ctx.core().state().tree_placement.is_some() {
        return match tree_placement_command(keysym, cleaned) {
            TreePlacementCommand::Navigate(side) => {
                crate::layouts::step_keyboard_tree_placement(ctx, side)
            }
            TreePlacementCommand::Swap(side) => {
                crate::layouts::swap_keyboard_tree_placement(ctx, side)
            }
            TreePlacementCommand::Resize(side) => {
                crate::layouts::resize_keyboard_tree_placement(ctx, side)
            }
            TreePlacementCommand::Cycle { backwards } => {
                crate::layouts::cycle_keyboard_tree_placement(ctx, backwards)
            }
            TreePlacementCommand::Center => crate::layouts::center_keyboard_tree_placement(ctx),
            TreePlacementCommand::Apply => {
                crate::layouts::finish_keyboard_tree_placement(ctx, true)
            }
            TreePlacementCommand::Cancel => {
                crate::layouts::finish_keyboard_tree_placement(ctx, false)
            }
            TreePlacementCommand::Modifier => true,
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
    fn tree_placement_accepts_arrows_and_vim_directions() {
        use crate::config::keysyms::*;
        use crate::layouts::tree::Side;

        for (keysym, side) in [
            (XK_LEFT, Side::Left),
            (XK_H, Side::Left),
            (XK_DOWN, Side::Bottom),
            (XK_J, Side::Bottom),
            (XK_UP, Side::Top),
            (XK_K, Side::Top),
            (XK_RIGHT, Side::Right),
            (XK_L, Side::Right),
        ] {
            assert_eq!(
                tree_placement_command(keysym, 0),
                TreePlacementCommand::Navigate(side)
            );
        }
    }

    #[test]
    fn tree_placement_modifiers_select_swap_and_resize() {
        use crate::config::keybindings::{CONTROL, MODKEY, SHIFT};
        use crate::config::keysyms::*;
        use crate::layouts::tree::Side;

        assert_eq!(
            tree_placement_command(XK_H_UPPER, SHIFT as u16),
            TreePlacementCommand::Swap(Side::Left)
        );
        assert_eq!(
            tree_placement_command(XK_RIGHT, (CONTROL | MODKEY) as u16),
            TreePlacementCommand::Resize(Side::Right)
        );
        assert_eq!(
            tree_placement_command(XK_J, (SHIFT | CONTROL) as u16),
            TreePlacementCommand::Cancel
        );
    }

    #[test]
    fn unrelated_tree_placement_keys_cancel_but_chord_modifiers_do_not() {
        use crate::config::keysyms::*;

        assert_eq!(
            tree_placement_command(XK_SHIFT_L, 0),
            TreePlacementCommand::Modifier
        );
        assert_eq!(
            tree_placement_command(XK_CONTROL_R, 0),
            TreePlacementCommand::Modifier
        );
        assert_eq!(
            tree_placement_command(XK_Q, 0),
            TreePlacementCommand::Cancel
        );
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
