//! XKB keyboard layout management.
//!
//! Provides functions to switch between configured keyboard layouts (e.g.
//! QWERTY, QWERTZ, Dvorak) via `setxkbmap`. Layouts are configured in the
//! TOML config under `[keyboard]` and can be switched at runtime via
//! keybindings or IPC.

use crate::backend::BackendOps;
use crate::contexts::WmCtx;
use crate::globals::KeyboardLayout;
use std::process::Command;

/// Apply the keyboard layout at the given index using `setxkbmap`.
///
/// This sets a single layout active (not the full list), which is the
/// simplest and most portable approach across X11 and XWayland.
fn apply_layout(ctx: &mut WmCtx, index: usize) -> Result<(), String> {
    let state = &ctx.core().globals().keyboard_layout;
    let layout = state
        .layout(index)
        .ok_or_else(|| format!("layout index {index} out of range"))?;
    let variant = layout.variant.as_deref().unwrap_or("");
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

    let mut cmd = Command::new("setxkbmap");
    cmd.arg("-layout").arg(&layout.name);
    if !variant.is_empty() {
        cmd.arg("-variant").arg(variant);
    }
    if let Some(ref opts) = options
        && !opts.is_empty()
    {
        cmd.arg("-option").arg("").arg("-option").arg(opts);
    }
    if let Some(ref m) = model
        && !m.is_empty()
    {
        cmd.arg("-model").arg(m);
    }

    cmd.spawn()
        .map_err(|e| format!("failed to run setxkbmap: {e}"))?;

    // Also apply via the backend abstraction (for Smithay wayland native layout)
    ctx.backend()
        .set_keyboard_layout(&layout.name, variant, options.as_deref(), model.as_deref());

    ctx.core_mut().globals_mut().keyboard_layout.current = index;
    Ok(())
}

/// Switch to a specific keyboard layout by index (0-based).
pub fn set_keyboard_layout(ctx: &mut WmCtx, index: usize) {
    if ctx.core().globals().keyboard_layout.is_empty() {
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
    let index = ctx.core().globals().keyboard_layout.find_layout_index(name);
    match index {
        Some(idx) => {
            set_keyboard_layout(ctx, idx);
            true
        }
        None => false,
    }
}

/// Cycle to the next keyboard layout.
/// Returns the status string of the new layout, or an empty string if no layouts are configured.
pub fn cycle_keyboard_layout(ctx: &mut WmCtx, forward: bool) -> String {
    let state = &ctx.core().globals().keyboard_layout;
    if state.is_empty() {
        return String::new();
    }
    let len = state.len();
    let current = state.current;
    let next = if forward {
        (current + 1) % len
    } else if current == 0 {
        len - 1
    } else {
        current - 1
    };
    set_keyboard_layout(ctx, next);
    keyboard_layout_status(ctx)
}

/// Get the current keyboard layout status as a formatted string.
pub fn keyboard_layout_status(ctx: &WmCtx) -> String {
    let state = &ctx.core().globals().keyboard_layout;
    if state.is_empty() {
        return "no layouts configured".to_string();
    }
    let current_name = state.current_layout().unwrap_or("unknown");
    let current_variant = state.current_variant();
    let variant_str = if current_variant.is_empty() {
        String::new()
    } else {
        format!(" ({})", current_variant)
    };
    format!(
        "{}/{}: {}{}",
        state.current + 1,
        state.len(),
        current_name,
        variant_str
    )
}

/// Get the list of configured keyboard layouts as a formatted string.
pub fn keyboard_layout_list(ctx: &WmCtx) -> String {
    let state = &ctx.core().globals().keyboard_layout;
    if state.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, layout) in state.layouts.iter().enumerate() {
        let variant = layout.variant.as_deref().unwrap_or("");
        let marker = if i == state.current { "* " } else { "  " };
        if variant.is_empty() {
            out.push_str(&format!("{}{}\n", marker, layout.name));
        } else {
            out.push_str(&format!("{}{} ({})\n", marker, layout.name, variant));
        }
    }
    out
}

/// Replace the configured keyboard layouts at runtime.
///
/// This allows IPC clients to reconfigure layouts without editing the TOML file.
pub fn set_keyboard_layouts(ctx: &mut WmCtx, layouts: Vec<KeyboardLayout>) {
    ctx.core_mut()
        .globals_mut()
        .keyboard_layout
        .reset_layouts(layouts);
    if !ctx.core().globals().keyboard_layout.is_empty() {
        set_keyboard_layout(ctx, 0);
    }
}

pub fn set_swapescape(ctx: &mut WmCtx, enabled: bool) {
    let current = ctx.core().globals().keyboard_layout.current;
    ctx.core_mut().globals_mut().keyboard_layout.swap_escape = enabled;
    if !ctx.core().globals().keyboard_layout.is_empty() {
        set_keyboard_layout(ctx, current);
    }
}

/// Apply the initially configured keyboard layout (called during startup).
pub fn init_keyboard_layout(ctx: &mut WmCtx) {
    if !ctx.core().globals().keyboard_layout.is_empty() {
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
        .globals_mut()
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
    let state = &ctx.core().globals().keyboard_layout;

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
        .globals_mut()
        .keyboard_layout
        .remove_layout(index)?;

    let current = ctx.core().globals().keyboard_layout.current;
    set_keyboard_layout(ctx, current);
    Ok(())
}
