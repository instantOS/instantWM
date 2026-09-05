//! X11-specific keyboard helpers: key grabbing, numlock detection.

use crate::backend::x11::{X11BackendRef, X11RuntimeConfig};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::Key;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub(crate) fn apply_layout(
    layout: &str,
    variant: &str,
    options: Option<&str>,
    model: Option<&str>,
) -> Result<(), String> {
    layout_command(layout, variant, options, model)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to run setxkbmap: {e}"))
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
        globals.config.bindings.keys.as_slice(),
        globals.config.bindings.desktop_keybinds.as_slice(),
        &globals.config.bindings.modes,
        globals.model.selected_win(),
        &globals.behavior.current_mode,
    );

    // Never discard working passive grabs when a refresh failed and no
    // replacement mapping is available.
    if x11_runtime.keyboard_mapping.keysyms.is_empty() {
        return;
    }

    let _ = ungrab_key(conn, 0, root, ModMask::ANY);

    let (keycode_min, keycode_max): (u8, u8) = (conn.setup().min_keycode, conn.setup().max_keycode);

    let modifiers: [u16; 4] = [
        0,
        ModMask::LOCK.bits(),
        numlockmask as u16,
        (numlockmask as u16) | ModMask::LOCK.bits(),
    ];

    for keycode in keycode_min..=keycode_max {
        let keysym = x11_runtime.keyboard_mapping.keysym(keycode, 0);
        if keysym == 0 {
            continue;
        }

        for key in &bindings {
            if keysym == key.keysym {
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
                x11_runtime.border_scheme.snap.bg.pixel()
            }
            crate::types::InteractionOutlineStyle::Close => {
                x11_runtime.border_scheme.close.bg.pixel()
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
                x11_runtime.border_scheme.snap.bg.pixel()
            }
            crate::types::InteractionOutlineStyle::Close => {
                x11_runtime.border_scheme.close.bg.pixel()
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
            keysyms: mapping.keysyms,
        };
        mapping_refreshed = true;
    }

    if let Some(reply) = modifier_cookie.ok().and_then(|cookie| cookie.reply().ok()) {
        let mut new_numlockmask: u32 = 0;
        for (i, keycode) in reply.keycodes.iter().enumerate() {
            if x11_runtime.keyboard_mapping.keysym(*keycode, 0) == 0xff7f {
                let mod_index = i / reply.keycodes_per_modifier() as usize;
                if mod_index < 8 {
                    new_numlockmask = 1 << mod_index;
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
    let _ = crate::keyboard::handle_keysym(&mut wm_ctx, keysym, state.bits() as u32);
}

/// Handle an X11 `KeyRelease` event (currently a no‑op).
pub fn key_release() {}

#[cfg(test)]
mod mapping_tests {
    use crate::backend::x11::X11KeyboardMapping;

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
            keysyms: vec![10, 11, 20, 21],
        };
        assert_eq!(mapping.keysym(8, 0), 10);
        assert_eq!(mapping.keysym(8, 1), 11);
        assert_eq!(mapping.keysym(9, 0), 20);
        assert_eq!(mapping.keysym(9, 1), 21);
    }

    #[test]
    fn cached_mapping_rejects_out_of_range_keycodes_and_columns() {
        let mapping = X11KeyboardMapping {
            min_keycode: 8,
            keysyms_per_keycode: 1,
            keysyms: vec![42, 84],
        };
        assert_eq!(mapping.keysym(7, 0), 0);
        assert_eq!(mapping.keysym(8, 1), 0);
        assert_eq!(mapping.keysym(9, 0), 84);
        assert_eq!(mapping.keysym(10, 0), 0);
    }
}
