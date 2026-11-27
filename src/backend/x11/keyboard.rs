//! X11-specific keyboard helpers: key grabbing, numlock detection.

use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::config::keysyms::XK_NUM_LOCK;
use crate::contexts::WmCtxX11;
use crate::types::{Key, Keysym, ModMask, Modifier};
use std::collections::{HashMap, HashSet};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
// The X11 wire type, named apart from `crate::types::ModMask` so the boundary
// conversion is visible at the call site rather than hidden by a shared name.
use x11rb::protocol::xproto::ModMask as XModMask;

pub(crate) fn apply_layout(
    layout: &str,
    variant: &str,
    options: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    let status = layout_command(layout, variant, options, model)
        .status()
        .map_err(|e| format!("failed to run setxkbmap: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("setxkbmap exited with {status}"))
    }
}

fn layout_command(
    layout: &str,
    variant: &str,
    options: Option<&str>,
    model: Option<&str>,
) -> std::process::Command {
    let mut command = std::process::Command::new("setxkbmap");
    // Explicitly clear the previous variant and options even when the new
    // configuration leaves them empty (e.g. disabling caps:swapescape).
    command.args(["-layout", layout, "-variant", variant, "-option", ""]);
    if let Some(options) = options.filter(|value| !value.is_empty()) {
        command.args(["-option", options]);
    }
    if let Some(model) = model.filter(|value| !value.is_empty()) {
        command.args(["-model", model]);
    }
    command
}

type NativeGrab = (u8, u16);

/// Confirmed server grabs and reusable plans for the two default-mode scopes.
#[derive(Default)]
pub(crate) struct KeyGrabs {
    installed: HashSet<NativeGrab>,
    default_plans: [Option<GrabPlan>; 2],
}

impl KeyGrabs {
    pub(crate) fn invalidate_plans(&mut self) {
        self.default_plans = Default::default();
    }
}

#[derive(Default)]
struct GrabPlan {
    wm: HashSet<NativeGrab>,
    portal: HashMap<crate::shortcuts::Trigger, HashSet<NativeGrab>>,
    desired: HashSet<NativeGrab>,
}

fn plan_grabs(
    mapping: &crate::backend::x11::X11KeyboardMapping,
    numlock: ModMask,
    bindings: &[&Key],
    triggers: &[crate::shortcuts::Trigger],
) -> GrabPlan {
    let caps = ModMask::from_modifier(Modifier::CapsLock);
    let locks = [ModMask::NONE, caps, numlock, numlock | caps];
    let mut plan = GrabPlan::default();
    // Keep empty entries: a valid keysym absent from the keyboard mapping is
    // an unavailable shortcut, rather than a successful registration.
    for trigger in triggers {
        plan.portal.insert(*trigger, Default::default());
    }
    if mapping.keysyms_per_keycode == 0 {
        return plan;
    }
    for (offset, symbols) in mapping
        .keysyms
        .chunks(usize::from(mapping.keysyms_per_keycode))
        .enumerate()
    {
        let Some(code) = usize::from(mapping.min_keycode)
            .checked_add(offset)
            .and_then(|code| u8::try_from(code).ok())
        else {
            break;
        };
        let key = symbols[0].for_binding();
        if key == Keysym::NONE {
            continue;
        }
        for binding in bindings.iter().filter(|binding| binding.keysym == key) {
            for lock in locks {
                plan.wm.insert((code, (binding.mod_mask | lock).bits()));
            }
        }
        for (trigger, grabs) in &mut plan.portal {
            if trigger.key == key {
                for lock in locks {
                    grabs.insert((code, (trigger.mods | lock).bits()));
                }
            }
        }
    }
    plan.desired = plan
        .wm
        .iter()
        .chain(plan.portal.values().flatten())
        .copied()
        .collect();
    plan
}

/// Reconcile passive grabs, requesting only chords the server hasn't confirmed.
pub fn grab_keys(
    state: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
) {
    if x11_runtime.keyboard_mapping.keysyms.is_empty() {
        return;
    }

    let mode = &state.behavior.current_mode;
    let selected = state.model.selected_win();
    let default_scope = matches!(mode, crate::core_state::ActiveWmMode::Default)
        .then(|| usize::from(crate::keyboard::desktop_bindings_enabled(selected, mode)));
    let mut grabs = x11_runtime.key_grabs.borrow_mut();
    let plan = default_scope
        .and_then(|scope| grabs.default_plans[scope].take())
        .unwrap_or_else(|| {
            let bindings =
                crate::keyboard::passive_bindings(&state.config.bindings, selected, mode);
            let triggers = if default_scope.is_some() {
                state.shortcuts.triggers()
            } else {
                Vec::new()
            };
            plan_grabs(
                &x11_runtime.keyboard_mapping,
                x11_runtime.numlockmask,
                &bindings,
                &triggers,
            )
        });

    let conn = x11.conn;
    let root = x11_runtime.root;
    let mut changed = false;
    grabs.installed.retain(|&(code, mods)| {
        let keep = plan.desired.contains(&(code, mods));
        if !keep {
            let _ = conn.ungrab_key(code, root, XModMask::from(mods));
            changed = true;
        }
        keep
    });

    let mut pending = Vec::new();
    for &(code, mods) in plan.desired.difference(&grabs.installed) {
        if let Ok(cookie) = conn.grab_key(
            false,
            root,
            XModMask::from(mods),
            code,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        ) {
            changed = true;
            pending.push(((code, mods), cookie));
        }
    }
    // Checking from the last request batches new grabs into one round trip.
    // A contested WM chord stays absent and can be retried on the next refresh.
    for (chord, cookie) in pending.into_iter().rev() {
        if cookie.check().is_ok() {
            grabs.installed.insert(chord);
        }
    }

    let mut rejected = Vec::new();
    for (&trigger, chords) in &plan.portal {
        if chords.is_empty() || !chords.is_subset(&grabs.installed) {
            log::warn!("global shortcuts: X11 could not grab {}", trigger.text());
            state.shortcuts.reject_trigger(trigger);
            rejected.push(trigger);
        }
    }
    if rejected.is_empty() {
        if let Some(scope) = default_scope {
            grabs.default_plans[scope] = Some(plan);
        }
    } else {
        grabs.invalidate_plans();
        // A portal trigger is all-or-nothing. Release its partial grabs
        // unless a WM binding or another successful trigger uses them.
        let retained: HashSet<_> = plan
            .wm
            .iter()
            .chain(
                plan.portal
                    .iter()
                    .filter(|(trigger, _)| !rejected.contains(trigger))
                    .flat_map(|(_, chords)| chords),
            )
            .copied()
            .collect();
        grabs.installed.retain(|&(code, mods)| {
            let keep = retained.contains(&(code, mods));
            if !keep {
                let _ = conn.ungrab_key(code, root, XModMask::from(mods));
                changed = true;
            }
            keep
        });
    }

    if changed {
        let _ = conn.flush();
    }
}

/// Own all keyboard input for a short compositor modal interaction.
pub fn grab_modal_keyboard(x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) -> bool {
    x11.conn
        .grab_keyboard(
            false,
            x11_runtime.root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.status == GrabStatus::SUCCESS)
}

pub fn ungrab_modal_keyboard(x11: &X11BackendRef) {
    let _ = x11.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
    let _ = x11.conn.flush();
}

/// Show or hide the hollow manual-tree insertion preview.
pub fn update_layout_preview(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    rect: Option<crate::types::Rect>,
    style: crate::types::InteractionOutlineStyle,
    target: Option<crate::types::WindowId>,
    animate: bool,
    duration: std::time::Duration,
) {
    x11_runtime.layout_preview_style = style;
    x11_runtime.layout_preview_target = target;
    let displayed = x11_runtime.layout_preview_animation.set_target(
        rect,
        animate,
        duration,
        std::time::Instant::now(),
    );
    render_layout_preview(x11, x11_runtime, displayed);
}

pub(crate) fn tick_layout_preview(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    now: std::time::Instant,
) {
    let displayed = x11_runtime.layout_preview_animation.tick(now);
    render_layout_preview(x11, x11_runtime, displayed);
}

fn render_layout_preview(
    x11: &X11BackendRef,
    x11_runtime: &mut X11RuntimeConfig,
    rect: Option<crate::types::Rect>,
) {
    let conn = x11.conn;
    if rect.is_some() && x11_runtime.layout_preview_windows.is_none() {
        let ids: [Option<Window>; 4] = std::array::from_fn(|_| conn.generate_id().ok());
        let Some(ids) = ids.into_iter().collect::<Option<Vec<_>>>() else {
            return;
        };
        let windows: [Window; 4] = ids.try_into().expect("exactly four preview windows");
        let color = match x11_runtime.layout_preview_style {
            crate::types::InteractionOutlineStyle::Layout => {
                x11_runtime.border_scheme.snap.background.pixel()
            }
            crate::types::InteractionOutlineStyle::Close => {
                x11_runtime.border_scheme.close.background.pixel()
            }
        };
        let aux = CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(color);
        for window in windows {
            if conn
                .create_window(
                    x11rb::COPY_FROM_PARENT as u8,
                    window,
                    x11_runtime.root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    &aux,
                )
                .is_err()
            {
                for created in windows {
                    let _ = conn.destroy_window(created);
                }
                return;
            }
        }
        x11_runtime.layout_preview_windows = Some(windows);
    }

    let Some(windows) = x11_runtime.layout_preview_windows else {
        return;
    };
    if let Some(rect) = rect {
        let color = match x11_runtime.layout_preview_style {
            crate::types::InteractionOutlineStyle::Layout => {
                x11_runtime.border_scheme.snap.background.pixel()
            }
            crate::types::InteractionOutlineStyle::Close => {
                x11_runtime.border_scheme.close.background.pixel()
            }
        };
        for (window, side) in windows
            .into_iter()
            .zip(crate::layouts::geometry::outline_rectangles(
                rect,
                crate::layouts::geometry::LAYOUT_PREVIEW_BORDER_WIDTH,
            ))
        {
            let _ = conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().background_pixel(color),
            );
            let mut configure = ConfigureWindowAux::new()
                .x(side.x)
                .y(side.y)
                .width(side.w.max(1) as u32)
                .height(side.h.max(1) as u32)
                .stack_mode(StackMode::ABOVE);
            if x11_runtime.layout_preview_style == crate::types::InteractionOutlineStyle::Close
                && let Some(target) = x11_runtime.layout_preview_target
            {
                configure = configure.sibling(u32::from(target));
            }
            x11.configure_stacking_window(x11_runtime, window, &configure);
            let _ = conn.map_window(window);
        }
    } else {
        for window in windows {
            let _ = conn.unmap_window(window);
        }
    }
    let _ = conn.flush();
}

impl crate::backend::LayoutInteractionOps for crate::contexts::WmCtxX11<'_> {
    fn begin_modal_keyboard(&mut self) -> bool {
        grab_modal_keyboard(&self.x11, self.x11_runtime)
    }

    fn end_modal_keyboard(&mut self) {
        ungrab_modal_keyboard(&self.x11);
    }

    fn layout_preview_changed(
        &mut self,
        rect: Option<crate::types::Rect>,
        style: crate::types::InteractionOutlineStyle,
        target: Option<crate::types::WindowId>,
        animate: bool,
        duration: std::time::Duration,
    ) {
        update_layout_preview(
            &self.x11,
            self.x11_runtime,
            rect,
            style,
            target,
            animate,
            duration,
        );
    }
}

/// Refresh keyboard state after startup or `MappingNotify`.
///
/// Both requests are issued before either reply is awaited, so this costs one
/// server round trip and keeps key-event handling entirely local afterwards.
pub fn refresh_keyboard_mapping(x11: &X11BackendRef, x11_runtime: &mut X11RuntimeConfig) -> bool {
    let conn = x11.conn;
    let (keycode_min, keycode_max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let mapping_cookie = conn.get_keyboard_mapping(keycode_min, keycode_max - keycode_min + 1);
    let modifier_cookie = conn.get_modifier_mapping();

    let mut mapping_refreshed = false;
    if let Some(mapping) = mapping_cookie.ok().and_then(|cookie| cookie.reply().ok())
        && !mapping.keysyms.is_empty()
    {
        x11_runtime.keyboard_mapping = crate::backend::x11::X11KeyboardMapping {
            min_keycode: keycode_min,
            keysyms_per_keycode: mapping.keysyms_per_keycode,
            keysyms: mapping.keysyms.into_iter().map(Keysym::new).collect(),
        };
        mapping_refreshed = true;
    }

    if let Some(reply) = modifier_cookie.ok().and_then(|cookie| cookie.reply().ok()) {
        // Ask the server which modifier position Num Lock occupies rather than
        // assuming Mod2, then trust that answer everywhere downstream.
        let mut new_numlockmask = ModMask::NONE;
        for (i, keycode) in reply.keycodes.iter().enumerate() {
            if x11_runtime.keyboard_mapping.keysym(*keycode, 0) == XK_NUM_LOCK {
                let mod_index = i / reply.keycodes_per_modifier() as usize;
                if mod_index < 8 {
                    new_numlockmask = ModMask::new(1 << mod_index);
                }
            }
        }
        x11_runtime.numlockmask = new_numlockmask;
    }

    if mapping_refreshed {
        x11_runtime.key_grabs.get_mut().invalidate_plans();
    }
    mapping_refreshed
}

/// Handle an X11 `KeyPress` event: convert the keycode to a keysym and dispatch
/// to the backend‑agnostic key handler.
pub fn key_press(ctx: &mut WmCtxX11, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;
    let keysym = ctx.x11_runtime.keyboard_mapping.keysym(keycode, 0);
    let mods = ModMask::new(state.bits()).cleaned(ctx.x11_runtime.numlockmask);
    if matches!(
        ctx.core.state.behavior.current_mode,
        crate::core_state::ActiveWmMode::Default
    ) && ctx
        .core
        .state
        .shortcuts
        .press(u32::from(keycode), keysym, mods)
    {
        return;
    }
    let mut wm_ctx = ctx.wm_ctx();
    // The event's state is already an X11 `ModMask`, so it needs no conversion.
    let _ = crate::keyboard::handle_keysym(&mut wm_ctx, keysym, ModMask::new(state.bits()));
}

/// Portal activation must not toggle on the artificial release/press pairs
/// generated by legacy X11 autorepeat. XKB provides real release events.
pub(crate) fn enable_detectable_repeat(conn: &impl Connection) -> bool {
    use x11rb::protocol::xkb::{self, BoolCtrl, PerClientFlag};
    let extension = xkb::use_extension(conn, 1, 0)
        .ok()
        .and_then(|cookie| cookie.reply().ok());
    if !extension.is_some_and(|reply| reply.supported) {
        return false;
    }
    let flag = PerClientFlag::DETECTABLE_AUTO_REPEAT;
    xkb::per_client_flags(
        conn,
        0x100,
        flag,
        flag,
        BoolCtrl::default(),
        BoolCtrl::default(),
        BoolCtrl::default(),
    )
    .ok()
    .and_then(|cookie| cookie.reply().ok())
    .is_some_and(|reply| reply.value.contains(flag))
}

#[cfg(test)]
mod mapping_tests {
    use super::plan_grabs;
    use crate::backend::x11::X11KeyboardMapping;
    use crate::shortcuts::Trigger;
    use crate::types::{Keysym, ModMask, Modifier};

    #[test]
    fn grab_plan_keeps_unavailable_symbols_for_native_rejection() {
        let mapping = X11KeyboardMapping {
            min_keycode: 8,
            keysyms_per_keycode: 2,
            keysyms: vec![
                Keysym::from_name("equal").unwrap(),
                Keysym::from_name("plus").unwrap(),
            ],
        };
        let unavailable = Trigger::parse("CTRL+plus").unwrap();
        let available = Trigger::parse("CTRL+equal").unwrap();
        let plan = plan_grabs(&mapping, ModMask::NONE, &[], &[unavailable, available]);
        assert!(plan.portal[&unavailable].is_empty());
        assert_eq!(plan.portal[&available].len(), 2);
        assert!(plan.portal[&available].contains(&(8, available.mods.bits())));
    }

    #[test]
    fn grab_plan_handles_multiple_keycodes_and_sticky_locks() {
        let key = Keysym::from_name("space").unwrap();
        let mapping = X11KeyboardMapping {
            min_keycode: 8,
            keysyms_per_keycode: 1,
            keysyms: vec![key, Keysym::NONE, key],
        };
        let trigger = Trigger::parse("CTRL+space").unwrap();
        let numlock = ModMask::new(16);
        let plan = plan_grabs(&mapping, numlock, &[], &[trigger]);
        let grabs = &plan.portal[&trigger];
        assert_eq!(grabs.len(), 8);
        for code in [8, 10] {
            for locks in [
                ModMask::NONE,
                ModMask::from_modifier(Modifier::CapsLock),
                numlock,
                numlock | ModMask::from_modifier(Modifier::CapsLock),
            ] {
                assert!(grabs.contains(&(code, (trigger.mods | locks).bits())));
            }
        }
    }

    #[test]
    fn layout_command_clears_previous_variant_and_options() {
        let command = super::layout_command("us", "", None, None);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["-layout", "us", "-variant", "", "-option", ""]);
    }

    #[test]
    fn layout_command_resets_options_before_applying_the_new_configuration() {
        let command = super::layout_command("us", "dvorak", Some("caps:swapescape"), Some("pc105"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "-layout",
                "us",
                "-variant",
                "dvorak",
                "-option",
                "",
                "-option",
                "caps:swapescape",
                "-model",
                "pc105",
            ]
        );
    }

    #[test]
    fn cached_mapping_resolves_columns_without_server_access() {
        let mapping = X11KeyboardMapping {
            min_keycode: 8,
            keysyms_per_keycode: 2,
            keysyms: vec![10, 11, 20, 21].into_iter().map(Keysym::new).collect(),
        };
        assert_eq!(mapping.keysym(8, 0), Keysym::new(10));
        assert_eq!(mapping.keysym(8, 1), Keysym::new(11));
        assert_eq!(mapping.keysym(9, 0), Keysym::new(20));
        assert_eq!(mapping.keysym(9, 1), Keysym::new(21));
    }

    #[test]
    fn cached_mapping_rejects_out_of_range_keycodes_and_columns() {
        let mapping = X11KeyboardMapping {
            min_keycode: 8,
            keysyms_per_keycode: 1,
            keysyms: vec![42, 84].into_iter().map(Keysym::new).collect(),
        };
        // `Keysym::NONE` is how the server says "this key produces no symbol",
        // which the grab scan and the Num Lock probe both have to detect.
        assert_eq!(mapping.keysym(7, 0), Keysym::NONE);
        assert_eq!(mapping.keysym(8, 1), Keysym::NONE);
        assert_eq!(mapping.keysym(9, 0), Keysym::new(84));
        assert_eq!(mapping.keysym(10, 0), Keysym::NONE);
    }
}
