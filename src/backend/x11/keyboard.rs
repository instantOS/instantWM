//! X11-specific keyboard helpers: key grabbing, numlock detection.

use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::config::keysyms::XK_NUM_LOCK;
use crate::contexts::WmCtxX11;
use crate::types::{Key, Keysym, ModMask, Modifier};
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

/// Install a keyboard layout without waiting for `setxkbmap`.
///
/// Startup has no user input yet and reacts to the resulting `MappingNotify`
/// by refreshing the cached mapping and passive grabs, so the command may
/// finish while the rest of the session initialization runs. Runtime layout
/// changes keep the synchronous path so an immediate key press observes the
/// requested layout.
pub(crate) fn spawn_layout(
    layout: &str,
    variant: &str,
    options: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    let child = layout_command(layout, variant, options, model)
        .spawn()
        .map_err(|e| format!("failed to run setxkbmap: {e}"))?;
    crate::util::reap_child_async(child);
    Ok(())
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

/// Number of applied reconciliations between full re-grabs of the WM chords.
///
/// WM grabs are issued unchecked because X11 reports conflicts asynchronously.
/// Re-grabbing every chord on every scope change retried contested ones, but
/// costs one request per binding. Applying only the delta keeps the common
/// desktop/window focus transition to a handful of requests; the periodic full
/// pass still recovers a chord that another client held during installation.
const GRAB_FULL_REFRESH_INTERVAL: u32 = 32;

/// Successful application grabs are retained across focus changes. WM grabs
/// track their scope but remain unchecked; the periodic full pass in
/// [`GRAB_FULL_REFRESH_INTERVAL`] re-issues them to recover contested chords.
#[derive(Default)]
pub(crate) struct KeyGrabs {
    installed: std::collections::HashSet<NativeGrab>,
    portal: std::collections::HashSet<NativeGrab>,
    /// Cached plan for desktop-scope and client-scope focus. The plans depend
    /// on bindings, keyboard mapping and portal triggers, all of which are
    /// explicitly invalidated on change; toggling focus only picks a slot.
    plans: [Option<GrabPlan>; 2],
    /// Desired set of the last applied plan. Unchanged scope needs no requests.
    applied: Option<std::collections::HashSet<NativeGrab>>,
    reconciliations: u32,
}

impl KeyGrabs {
    /// Drop cached plans and force the next application to be a full refresh.
    ///
    /// Called for every input that `plan_grabs` reads beyond focus scope:
    /// configuration reloads, keyboard mapping changes, mode switches and
    /// portal trigger updates.
    pub(crate) fn invalidate(&mut self) {
        self.plans = Default::default();
        self.reconciliations = 0;
    }

    /// Whether the desired grab set still needs to be projected onto the server.
    fn needs_refresh(&self, desired: &std::collections::HashSet<NativeGrab>) -> bool {
        self.reconciliations == 0
            || self.reconciliations >= GRAB_FULL_REFRESH_INTERVAL
            || self.applied.as_ref() != Some(desired)
    }
}

#[derive(Default)]
struct GrabPlan {
    wm: std::collections::HashSet<NativeGrab>,
    portal:
        std::collections::HashMap<crate::shortcuts::Trigger, std::collections::HashSet<NativeGrab>>,
    desired: std::collections::HashSet<NativeGrab>,
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
    plan
}

/// Update only changed X11 keybindings.
///
/// The plan for the current focus scope is cached until an input beyond that
/// scope changes; a scope toggle therefore projects the focused/desktop delta
/// instead of every WM chord. Unchanged scopes issue no requests at all.
pub fn grab_keys(
    state: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
) {
    let conn = x11.conn;
    let root = x11_runtime.root;
    if x11_runtime.keyboard_mapping.keysyms.is_empty() {
        return;
    }
    let desktop_scope = usize::from(crate::keyboard::desktop_bindings_enabled(
        state.model.selected_win(),
        &state.behavior.current_mode,
    ));
    let mut grabs = x11_runtime.key_grabs.borrow_mut();
    let mut plan = match grabs.plans[desktop_scope].take() {
        Some(plan) => plan,
        None => {
            let bindings = crate::keyboard::passive_bindings(
                &state.config.bindings,
                state.model.selected_win(),
                &state.behavior.current_mode,
            );
            let triggers = if matches!(
                state.behavior.current_mode,
                crate::core_state::ActiveWmMode::Default
            ) {
                state.shortcuts.triggers()
            } else {
                Vec::new()
            };
            let mut plan = plan_grabs(
                &x11_runtime.keyboard_mapping,
                x11_runtime.numlockmask,
                &bindings,
                &triggers,
            );
            plan.desired = plan
                .wm
                .iter()
                .copied()
                .chain(plan.portal.values().flatten().copied())
                .collect();
            plan
        }
    };

    if !grabs.needs_refresh(&plan.desired) {
        grabs.plans[desktop_scope] = Some(plan);
        return;
    }
    let full_refresh =
        grabs.reconciliations == 0 || grabs.reconciliations >= GRAB_FULL_REFRESH_INTERVAL;
    for &(code, mods) in grabs.installed.difference(&plan.desired) {
        let _ = conn.ungrab_key(code, root, XModMask::from(mods));
    }
    // Only chords the server is not known to have grabbed need a request. A
    // periodic full pass retries unchecked WM grabs that may have lost a
    // contested chord.
    for &(code, mods) in &plan.wm {
        if full_refresh || !grabs.installed.contains(&(code, mods)) {
            let _ = conn.grab_key(
                false,
                root,
                XModMask::from(mods),
                code,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            );
        }
    }
    let mut pending = Vec::new();
    let mut failed = std::collections::HashSet::new();
    for (&trigger, trigger_grabs) in &plan.portal {
        if trigger_grabs.is_empty() {
            failed.insert(trigger);
        }
        for &(code, mods) in trigger_grabs.difference(&grabs.portal) {
            match conn.grab_key(
                false,
                root,
                XModMask::from(mods),
                code,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            ) {
                Ok(cookie) => pending.push((trigger, cookie)),
                Err(_) => {
                    failed.insert(trigger);
                }
            }
        }
    }
    // Checking the last request first batches all new grabs into one round trip.
    for (trigger, cookie) in pending.into_iter().rev() {
        if cookie.check().is_err() {
            failed.insert(trigger);
        }
    }
    let mut portal = std::collections::HashSet::new();
    for (trigger, trigger_grabs) in std::mem::take(&mut plan.portal) {
        if failed.contains(&trigger) {
            for (code, mods) in trigger_grabs.difference(&plan.wm) {
                let _ = conn.ungrab_key(*code, root, XModMask::from(*mods));
            }
            log::warn!("global shortcuts: X11 could not grab {}", trigger.text());
            state.shortcuts.reject_trigger(trigger);
        } else {
            portal.extend(trigger_grabs);
        }
    }
    grabs.installed = std::mem::take(&mut plan.wm);
    grabs.installed.extend(&portal);
    grabs.portal = portal;
    grabs.applied = Some(plan.desired.clone());
    grabs.reconciliations = if full_refresh {
        1
    } else {
        grabs.reconciliations + 1
    };
    grabs.plans[desktop_scope] = Some(plan);
    let _ = conn.flush();
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
            let _ = conn.configure_window(window, &configure);
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
        x11_runtime.key_grabs.get_mut().invalidate();
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
    fn grab_refresh_skips_unchanged_sets_until_full_pass() {
        let mut grabs = super::KeyGrabs::default();
        let desired = std::collections::HashSet::from([(8u8, 0u16), (9u8, 4u16)]);
        // First projection always reaches the server.
        assert!(grabs.needs_refresh(&desired));
        grabs.applied = Some(desired.clone());
        grabs.reconciliations = 1;
        // Unchanged scope between full passes must not issue requests.
        assert!(!grabs.needs_refresh(&desired));
        // A changed set is projected immediately.
        assert!(grabs.needs_refresh(&std::collections::HashSet::from([(8u8, 0u16)])));
        // The periodic full pass retries unchecked WM grabs.
        grabs.reconciliations = super::GRAB_FULL_REFRESH_INTERVAL;
        assert!(grabs.needs_refresh(&desired));
        // A config or mapping change produces a full pass regardless of count.
        grabs.reconciliations = 1;
        grabs.invalidate();
        assert!(grabs.needs_refresh(&desired));
        assert!(grabs.plans.iter().all(Option::is_none));
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
