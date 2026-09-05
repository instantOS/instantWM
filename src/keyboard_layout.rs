//! XKB keyboard layout management.
//!
//! Provides functions to switch between configured keyboard layouts (e.g.
//! QWERTY, QWERTZ, Dvorak) through the active backend. Layouts are configured in the
//! TOML config under `[keyboard]` and can be switched at runtime via
//! keybindings or IPC.

use crate::contexts::WmCtx;
use crate::types::KeyboardLayout;
use crate::types::input::StackDirection;
use std::process::Command;

/// Apply one configured layout through the active backend.
fn apply_layout(ctx: &mut WmCtx, index: usize) -> Result<(), String> {
    let state = &ctx.core().interaction().keyboard_layout;
    let layout = state
        .layout(index)
        .ok_or_else(|| format!("layout index {index} out of range"))?
        .clone();
    let variant = layout.variant.as_deref().unwrap_or("").to_owned();
    let mut options = state.options.clone();
    let model = state.model.clone();

    if state.swap_escape {
        if let Some(ref mut opts) = options {
            if !opts.is_empty() {
                opts.push_str(",caps:swapescape");
            } else {
                *opts = "caps:swapescape".to_string();
            }
        } else {
            options = Some("caps:swapescape".to_string());
        }
    }

    ctx.apply_keyboard_layout(&layout.name, &variant, options.as_deref(), model.as_deref())?;

    ctx.core_mut().interaction_mut().keyboard_layout.current = index;
    Ok(())
}

/// Switch to a specific keyboard layout by index (0-based).
pub fn set_keyboard_layout(ctx: &mut WmCtx, index: usize) {
    if ctx.core().interaction().keyboard_layout.is_empty() {
        return;
    }
    if let Err(e) = apply_layout(ctx, index) {
        eprintln!("instantwm: {e}");
    }
}

/// Switch to a keyboard layout by name.
///
/// If the name matches one of the configured layouts, switch to it.
/// Returns `true` if the layout was found and applied.
pub fn set_keyboard_layout_by_name(ctx: &mut WmCtx, name: &str) -> bool {
    let index = ctx
        .core()
        .interaction()
        .keyboard_layout
        .find_layout_index(name);
    match index {
        Some(idx) => {
            set_keyboard_layout(ctx, idx);
            true
        }
        None => false,
    }
}

/// Cycle to the next or previous keyboard layout.
/// Returns the status string of the new layout, or an empty string if no layouts are configured.
pub fn cycle_keyboard_layout(ctx: &mut WmCtx, direction: StackDirection) -> String {
    let state = &ctx.core().interaction().keyboard_layout;
    if state.is_empty() {
        return String::new();
    }
    let len = state.len();
    let current = state.current;
    let next = if direction.is_forward() {
        (current + 1) % len
    } else if current == 0 {
        len - 1
    } else {
        current - 1
    };
    set_keyboard_layout(ctx, next);
    ctx.core().interaction().keyboard_layout.status()
}

/// Replace the configured keyboard layouts at runtime.
///
/// This allows IPC clients to reconfigure layouts without editing the TOML file.
pub fn set_keyboard_layouts(ctx: &mut WmCtx, layouts: Vec<KeyboardLayout>) {
    ctx.core_mut()
        .state_mut()
        .interaction
        .keyboard_layout
        .reset_layouts(layouts);
    if !ctx.core().interaction().keyboard_layout.is_empty() {
        set_keyboard_layout(ctx, 0);
    }
}

pub fn set_swapescape(ctx: &mut WmCtx, enabled: bool) {
    let current = ctx.core().interaction().keyboard_layout.current;
    ctx.core_mut().interaction_mut().keyboard_layout.swap_escape = enabled;
    if !ctx.core().interaction().keyboard_layout.is_empty() {
        set_keyboard_layout(ctx, current);
    }
}

/// Apply the initially configured keyboard layout (called during startup).
pub fn init_keyboard_layout(ctx: &mut WmCtx) {
    if !ctx.core().interaction().keyboard_layout.is_empty() {
        set_keyboard_layout(ctx, 0);
    }
}

/// Get all available XKB layouts from the system.
///
/// Runs `localectl list-x11-keymap-layouts` to get the list.
/// Returns an empty list if the command fails.
pub fn get_all_keyboard_layouts() -> Vec<String> {
    let output = Command::new("localectl")
        .arg("list-x11-keymap-layouts")
        .output();

    match output {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                Vec::new()
            }
        }
        Err(_) => Vec::new(),
    }
}

/// Add a keyboard layout to the active list.
///
/// If the layout already exists, returns an error.
/// Switches to the newly added layout.
pub fn add_keyboard_layout(ctx: &mut WmCtx, layout: KeyboardLayout) -> Result<(), String> {
    let new_index = ctx
        .core_mut()
        .state_mut()
        .interaction
        .keyboard_layout
        .add_layout(layout)?;

    // Switch to the new layout
    set_keyboard_layout(ctx, new_index);
    Ok(())
}

/// Remove a keyboard layout from the active list.
///
/// The `layout` argument can be:
/// - A layout name (e.g., "de")
/// - An index prefixed with # (e.g., "#1")
///
/// Returns an error if the layout doesn't exist or if it's the last layout.
pub fn remove_keyboard_layout(ctx: &mut WmCtx, layout: &str) -> Result<(), String> {
    let state = &ctx.core().interaction().keyboard_layout;

    // Parse the layout argument
    let index = if let Some(stripped) = layout.strip_prefix('#') {
        // Index format: #1, #2, etc.
        let idx = stripped
            .parse::<usize>()
            .map_err(|_| format!("invalid index '{}'", layout))?;
        // Convert to 0-based
        if idx == 0 || idx > state.layouts.len() {
            return Err(format!(
                "index {} out of range (1-{})",
                idx,
                state.layouts.len()
            ));
        }
        Some(idx - 1)
    } else {
        // Name format: find by name
        state.layouts.iter().position(|l| l.name == layout)
    };

    let index = index.ok_or_else(|| format!("layout '{}' not found", layout))?;

    ctx.core_mut()
        .state_mut()
        .interaction
        .keyboard_layout
        .remove_layout(index)?;

    let current = ctx.core().interaction().keyboard_layout.current;
    set_keyboard_layout(ctx, current);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_wayland_layout_switch_does_not_require_x11_utilities() {
        const CHILD_ENV: &str = "INSTANTWM_KEYMAP_TEST_CHILD";
        if std::env::var_os(CHILD_ENV).is_none() {
            // Keep PATH and display changes local to a subprocess: other
            // tests can run concurrently without touching the host keymap.
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "keyboard_layout::tests::native_wayland_layout_switch_does_not_require_x11_utilities",
                    "--nocapture",
                ])
                .env(CHILD_ENV, "1")
                .env("PATH", "")
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        use crate::backend::{Backend, wayland::WaylandBackend};
        let (_event_loop, mut state) =
            crate::backend::wayland::compositor::new_event_loop_and_state();
        let backend = WaylandBackend::new();
        backend.attach_state(&mut state);
        let mut wm = crate::wm::Wm::new(Backend::new_wayland(backend));
        wm.core.interaction.keyboard_layout.layouts =
            vec![KeyboardLayout::new("us"), KeyboardLayout::new("de")];

        apply_layout(&mut wm.ctx(), 1).unwrap();
        assert_eq!(wm.core.interaction.keyboard_layout.current, 1);
        let symbol = state
            .keyboard
            .clone()
            .with_xkb_state(&mut state, |context| {
                let xkb = context.xkb().lock().unwrap();
                // 29 is the xkb keycode of the Y key (evdev 21 + 8): 'y' on the
                // US layout, 'z' once the German keymap is active.
                xkb.raw_syms_for_key_in_layout(29u16.into(), xkb.active_layout())
                    .first()
                    .and_then(|symbol| symbol.key_char())
            });
        assert_eq!(
            symbol,
            Some('z'),
            "German layout must replace the US keymap"
        );
    }
}
