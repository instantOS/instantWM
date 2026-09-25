//! X11-specific keyboard helpers: key grabbing, numlock detection.

use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::config::keysyms::XK_NUM_LOCK;
use crate::contexts::{WmCtx, WmCtxX11};
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

fn grab_keys_for_key<C: Connection>(
    conn: &C,
    root: Window,
    modifiers: &[ModMask],
    key: &Key,
    keycode: u8,
) {
    for &lock_variation in modifiers {
        let _ = grab_key(
            conn,
            false,
            root,
            XModMask::from((key.mod_mask | lock_variation).bits()),
            keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        );
    }
}

/// Grab all X11 keybindings for the current config.
pub fn grab_keys(
    globals: &crate::core_state::CoreState,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
) {
    let conn = x11.conn;
    let root = x11_runtime.root;
    let numlockmask = x11_runtime.numlockmask;
    let bindings = crate::keyboard::passive_bindings(
        &globals.config.bindings,
        globals.model.selected_win(),
        &globals.behavior.current_mode,
    );

    // Never discard working passive grabs when a refresh failed and no
    // replacement mapping is available.
    if x11_runtime.keyboard_mapping.keysyms.is_empty() {
        return;
    }

    let _ = ungrab_key(conn, 0, root, XModMask::ANY);

    let (keycode_min, keycode_max): (u8, u8) = (conn.setup().min_keycode, conn.setup().max_keycode);

    // A passive grab is keyed on an exact modifier state, so a binding that
    // should also fire with Caps Lock or Num Lock held needs one grab per
    // combination of those sticky modifiers. `Modifier::CapsLock` is bit 1, the
    // same position as x11rb's `ModMask::LOCK`.
    let caps_lock = ModMask::from_modifier(Modifier::CapsLock);
    let modifiers: [ModMask; 4] = [
        ModMask::NONE,
        caps_lock,
        numlockmask,
        numlockmask | caps_lock,
    ];

    for keycode in keycode_min..=keycode_max {
        let keysym = x11_runtime.keyboard_mapping.keysym(keycode, 0);
        if keysym == Keysym::NONE {
            continue;
        }

        for key in &bindings {
            if keysym.for_binding() == key.keysym {
                grab_keys_for_key(conn, root, &modifiers, key, keycode);
            }
        }
    }

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

/// Show or hide the hollow manual-tree placement preview.
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
        for (window, side) in
            windows
                .into_iter()
                .zip(crate::layouts::placement::outline_rectangles(
                    rect,
                    crate::layouts::placement::LAYOUT_PREVIEW_BORDER_WIDTH,
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

    mapping_refreshed
}

/// Handle an X11 `KeyPress` event: convert the keycode to a keysym and dispatch
/// to the backend‑agnostic key handler.
pub fn key_press(ctx: &mut WmCtxX11, e: &KeyPressEvent) {
    let keycode = e.detail;
    let state = e.state;
    let keysym = ctx.x11_runtime.keyboard_mapping.keysym(keycode, 0);
    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    // The event's state is already an X11 `ModMask`, so it needs no conversion.
    let _ = crate::keyboard::handle_keysym(&mut wm_ctx, keysym, ModMask::new(state.bits()));
}

#[cfg(test)]
mod mapping_tests {
    use crate::backend::x11::X11KeyboardMapping;
    use crate::types::Keysym;

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
