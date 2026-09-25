use crate::actions::{KeyAction, execute_key_action};
use crate::config::keybindings::MODKEY;
use crate::contexts::WmCtx;
use crate::core_state::{ActiveWmMode, BindingConfig};
use crate::floating::change_snap;
use crate::focus::focus_stack;

use crate::types::*;

/// Dispatch one key press against the active binding tables.
///
/// Returns whether the WM consumed the key. The chord arrives already in
/// instantWM's X11 modifier convention from whichever backend produced it; this
/// is the single place both backends funnel through, so binding matching, the
/// sticky-lock cleanup and the placement-mode chord rules all live in one spot
/// rather than being re-implemented per backend.
pub fn handle_keysym(ctx: &mut WmCtx, keysym: Keysym, mod_mask: ModMask) -> bool {
    let numlockmask = ctx.numlock_mask();
    let cleaned = mod_mask.cleaned(numlockmask);
    let placement_active = matches!(ctx.current_mode(), ActiveWmMode::TreePlacement(_));
    // Super may still be held after the chord that entered placement. Treat it
    // as an entry modifier, not part of commands within the mode.
    let binding_mask = if placement_active {
        cleaned.without(Modifier::Super)
    } else {
        cleaned
    };
    let binding_keysym = keysym.for_binding();

    // Super + Escape always resets to default mode
    if !matches!(ctx.current_mode(), ActiveWmMode::Default)
        && keysym == crate::config::keysyms::XK_ESCAPE
        && cleaned == MODKEY.cleaned(numlockmask)
    {
        ctx.reset_mode();
        return true;
    }

    let resolved = resolve_key_action(
        &ctx.core().config().bindings,
        ctx.core().model().selected_win(),
        ctx.current_mode(),
        binding_keysym,
        binding_mask,
        numlockmask,
    )
    .map(|(action, transient)| (action.clone(), transient));

    if let Some((action, transient)) = resolved {
        execute_key_action(ctx, &action);
        if transient {
            ctx.reset_mode();
        }
        true
    } else if placement_active {
        // Modifier presses are part of forming the next chord. Every other
        // unbound key cancels and is consumed so it cannot leak to a client.
        if !keysym.is_modifier() {
            crate::layouts::finish_keyboard_tree_placement(ctx, false);
        }
        true
    } else {
        false
    }
}

pub(crate) fn desktop_bindings_enabled(
    selected_client: Option<WindowId>,
    mode: &ActiveWmMode,
) -> bool {
    !matches!(mode, ActiveWmMode::Default) || selected_client.is_none()
}

/// Binding tables consulted for `mode`, in priority order, and whether the
/// mode is transient.
///
/// This is the single definition of binding scope for both dispatch and
/// passive grabs: a backend must not grab bindings from inactive modes, since
/// unlike a compositor, X11 cannot forward an unmatched passively-grabbed key
/// to the focused client after the fact.
fn binding_scopes<'a>(
    bindings: &'a BindingConfig,
    selected_client: Option<WindowId>,
    mode: &ActiveWmMode,
) -> ([&'a [Key]; 3], bool) {
    let mode_keys = |name: &str| {
        bindings
            .modes
            .get(name)
            .map_or(&[][..], |mode| mode.keybinds.as_slice())
    };
    match mode {
        ActiveWmMode::TreePlacement(_) => (
            [
                mode_keys(crate::core_state::TREE_PLACEMENT_MODE_NAME),
                &[],
                &[],
            ],
            false,
        ),
        ActiveWmMode::Named(name) => (
            [mode_keys(name), &bindings.keys, &bindings.desktop_keybinds],
            bindings.modes.get(name).is_some_and(|mode| mode.transient),
        ),
        _ => {
            let desktop: &[Key] = if desktop_bindings_enabled(selected_client, mode) {
                &bindings.desktop_keybinds
            } else {
                &[]
            };
            ([&bindings.keys, desktop, &[]], false)
        }
    }
}

/// Bindings that a backend with passive/global grabs must currently own.
pub(crate) fn passive_bindings<'a>(
    bindings: &'a BindingConfig,
    selected_client: Option<WindowId>,
    mode: &ActiveWmMode,
) -> Vec<&'a Key> {
    binding_scopes(bindings, selected_client, mode)
        .0
        .into_iter()
        .flatten()
        .collect()
}

/// The action bound to a chord in the current mode, and whether the mode is
/// transient.
///
/// `binding_mask` is expected to be already cleaned of sticky locks; the
/// configured masks are cleaned the same way here so the two sides of the
/// comparison are normalized identically.
fn resolve_key_action<'a>(
    bindings: &'a BindingConfig,
    selected_client: Option<WindowId>,
    mode: &ActiveWmMode,
    keysym: Keysym,
    binding_mask: ModMask,
    numlockmask: ModMask,
) -> Option<(&'a KeyAction, bool)> {
    let (scopes, transient) = binding_scopes(bindings, selected_client, mode);
    scopes
        .into_iter()
        .flatten()
        .find(|key| keysym == key.keysym && key.mod_mask.cleaned(numlockmask) == binding_mask)
        .map(|key| (&key.action, transient))
}

/// Alt-tab style navigation: overview focus, snapping for floating layouts,
/// otherwise the focus stack.
pub fn alt_tab_key(ctx: &mut WmCtx, direction: VerticalDirection) {
    if ctx.core().model().is_overview_active() {
        crate::overview::focus_direction(ctx, direction.into());
        return;
    }

    if ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout()
    {
        focus_stack(ctx, direction.into());
    } else if let Some(win) = ctx.core().model().selected_win() {
        change_snap(ctx, win, direction.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::NamedAction;
    use crate::config::ModeConfig;
    use crate::core_state::ActiveWmMode;
    use std::collections::HashMap;

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

        assert_eq!(XK_H_UPPER.for_binding(), XK_H);
        assert_eq!(XK_H.for_binding(), XK_H);
        // A non-letter is already its own base keysym and must not be touched.
        assert_eq!(XK_RETURN.for_binding(), XK_RETURN);
        assert_eq!(XK_F1.for_binding(), XK_F1);

        // Every modifier a chord could use must be recognized, not just the
        // ones the compiled defaults happen to bind: a user-configured chord
        // with Alt or AltGr would otherwise be cancelled mid-composition.
        for modifier_keysym in [
            XK_SHIFT_L,
            XK_SHIFT_R,
            XK_CONTROL_L,
            XK_CONTROL_R,
            XK_ALT_L,
            XK_ALT_R,
            XK_SUPER_L,
            XK_SUPER_R,
            XK_ISO_LEVEL3_SHIFT,
            XK_ISO_LEVEL5_SHIFT,
            XK_MODE_SWITCH,
        ] {
            assert!(modifier_keysym.is_modifier(), "{modifier_keysym:?}");
        }
        for ordinary in [XK_Q, XK_RETURN, XK_F1, XK_SPACE, XK_ESCAPE] {
            assert!(!ordinary.is_modifier(), "{ordinary:?}");
        }
    }

    #[test]
    fn resolve_key_action_prefers_mode_binding_and_marks_transient() {
        let mode_key = Key {
            mod_mask: ModMask::from_modifier(Modifier::Shift),
            keysym: Keysym::new(42),
            action: KeyAction::named(NamedAction::FocusNext),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let global_key = Key {
            mod_mask: ModMask::from_modifier(Modifier::Shift),
            keysym: Keysym::new(42),
            action: KeyAction::named(NamedAction::FocusPrev),
            origin: crate::types::KeybindOrigin::CompiledDefault,
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

        let bindings = BindingConfig {
            keys: vec![global_key],
            modes,
            ..BindingConfig::default()
        };
        let resolved = resolve_key_action(
            &bindings,
            None,
            &ActiveWmMode::Named("resize".to_string()),
            Keysym::new(42),
            ModMask::from_modifier(Modifier::Shift),
            ModMask::NONE,
        )
        .expect("expected action");

        assert!(matches!(
            resolved.0,
            KeyAction::Named(NamedAction::FocusNext)
        ));
        assert!(resolved.1);
    }

    #[test]
    fn placement_resolves_only_its_configured_mode_actions() {
        let placement_key = Key {
            mod_mask: ModMask::NONE,
            keysym: Keysym::new(42),
            action: KeyAction::named(NamedAction::PlacementLeft),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let global_key = Key {
            mod_mask: ModMask::NONE,
            keysym: Keysym::new(43),
            action: KeyAction::named(NamedAction::FocusNext),
            origin: crate::types::KeybindOrigin::CompiledDefault,
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
        let bindings = BindingConfig {
            keys: global_keys.to_vec(),
            modes,
            ..BindingConfig::default()
        };

        let resolved = resolve_key_action(
            &bindings,
            None,
            &mode,
            Keysym::new(42),
            ModMask::NONE,
            ModMask::NONE,
        )
        .expect("configured placement action");
        assert!(matches!(
            resolved.0,
            KeyAction::Named(NamedAction::PlacementLeft)
        ));
        assert!(!resolved.1, "placement is intrinsically non-transient");
        assert!(
            resolve_key_action(
                &bindings,
                None,
                &mode,
                Keysym::new(43),
                ModMask::NONE,
                ModMask::NONE,
            )
            .is_none()
        );
    }

    #[test]
    fn resolve_key_action_uses_desktop_bindings_only_without_selected_client() {
        let desktop_key = Key {
            mod_mask: ModMask::NONE,
            keysym: Keysym::new(9),
            action: KeyAction::named(NamedAction::ToggleBar),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };

        let bindings = BindingConfig {
            desktop_keybinds: vec![desktop_key],
            ..BindingConfig::default()
        };
        let resolved = resolve_key_action(
            &bindings,
            None,
            &ActiveWmMode::Default,
            Keysym::new(9),
            ModMask::NONE,
            ModMask::NONE,
        )
        .expect("expected desktop action");

        assert!(matches!(
            resolved.0,
            KeyAction::Named(NamedAction::ToggleBar)
        ));

        let blocked_bindings = BindingConfig {
            desktop_keybinds: vec![Key {
                mod_mask: ModMask::NONE,
                keysym: Keysym::new(9),
                action: KeyAction::named(NamedAction::ToggleBar),
                origin: crate::types::KeybindOrigin::CompiledDefault,
            }],
            ..BindingConfig::default()
        };
        let blocked = resolve_key_action(
            &blocked_bindings,
            Some(WindowId(1)),
            &ActiveWmMode::Default,
            Keysym::new(9),
            ModMask::NONE,
            ModMask::NONE,
        );
        assert!(blocked.is_none());
    }

    #[test]
    fn resolve_key_action_overview_ignores_configured_overview_mode() {
        // A user-configured mode whose name collides with the built-in overview.
        // It must NOT be consulted while the WM is in Overview mode.
        let overview_mode_key = Key {
            mod_mask: ModMask::from_modifier(Modifier::Shift),
            keysym: Keysym::new(42),
            action: KeyAction::named(NamedAction::FocusPrev),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let global_key = Key {
            mod_mask: ModMask::from_modifier(Modifier::Shift),
            keysym: Keysym::new(42),
            action: KeyAction::named(NamedAction::FocusNext),
            origin: crate::types::KeybindOrigin::CompiledDefault,
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
        let bindings = BindingConfig {
            keys: vec![global_key],
            modes,
            ..BindingConfig::default()
        };
        let resolved = resolve_key_action(
            &bindings,
            None,
            &ActiveWmMode::Overview,
            Keysym::new(42),
            ModMask::from_modifier(Modifier::Shift),
            ModMask::NONE,
        )
        .expect("expected global action in overview");
        assert!(matches!(
            resolved.0,
            KeyAction::Named(NamedAction::FocusNext)
        ));
        assert!(!resolved.1);
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

    #[test]
    fn passive_grabs_match_only_bindings_the_dispatcher_can_use() {
        let global = Key {
            mod_mask: crate::config::keybindings::MODKEY,
            keysym: Keysym::new(1),
            action: KeyAction::named(NamedAction::FocusNext),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let desktop = Key {
            mod_mask: ModMask::NONE,
            keysym: crate::config::keysyms::XK_L,
            action: KeyAction::named(NamedAction::ScrollRight),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let inactive_mode = Key {
            mod_mask: ModMask::NONE,
            keysym: crate::config::keysyms::XK_SPACE,
            action: KeyAction::named(NamedAction::PlacementCenter),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let mut modes = HashMap::new();
        modes.insert(
            crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string(),
            ModeConfig {
                keybinds: vec![inactive_mode],
                ..ModeConfig::default()
            },
        );

        let bindings = BindingConfig {
            keys: vec![global.clone()],
            desktop_keybinds: vec![desktop.clone()],
            modes,
            ..BindingConfig::default()
        };
        let grabbed = passive_bindings(&bindings, Some(WindowId(1)), &ActiveWmMode::Default);

        assert_eq!(grabbed.len(), 1);
        assert_eq!(grabbed[0].keysym, global.keysym);
        assert!(grabbed.iter().all(|key| key.keysym != desktop.keysym));
        assert!(
            grabbed
                .iter()
                .all(|key| key.keysym != crate::config::keysyms::XK_SPACE)
        );
    }

    #[test]
    fn active_named_mode_passively_grabs_its_complete_resolution_scope() {
        let global = Key {
            mod_mask: ModMask::from_modifier(Modifier::Shift),
            keysym: Keysym::new(1),
            action: KeyAction::named(NamedAction::FocusNext),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let desktop = Key {
            mod_mask: ModMask::NONE,
            keysym: Keysym::new(2),
            action: KeyAction::named(NamedAction::ScrollRight),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let mode_key = Key {
            mod_mask: ModMask::NONE,
            keysym: Keysym::new(3),
            action: KeyAction::named(NamedAction::FocusPrev),
            origin: crate::types::KeybindOrigin::CompiledDefault,
        };
        let mut modes = HashMap::new();
        modes.insert(
            "resize".to_string(),
            ModeConfig {
                keybinds: vec![mode_key],
                ..ModeConfig::default()
            },
        );

        let bindings = BindingConfig {
            keys: vec![global],
            desktop_keybinds: vec![desktop],
            modes,
            ..BindingConfig::default()
        };
        let grabbed = passive_bindings(
            &bindings,
            Some(WindowId(1)),
            &ActiveWmMode::Named("resize".to_string()),
        );
        let keysyms = grabbed.iter().map(|key| key.keysym).collect::<Vec<_>>();

        assert_eq!(keysyms, [Keysym::new(3), Keysym::new(1), Keysym::new(2)]);
    }
}
