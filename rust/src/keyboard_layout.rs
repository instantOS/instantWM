//! XKB keyboard layout management.
//!
//! Provides functions to switch between configured keyboard layouts (e.g.
//! QWERTY, QWERTZ, Dvorak) via `setxkbmap`. Layouts are configured in the
//! TOML config under `[keyboard]` and can be switched at runtime via
//! keybindings or IPC.

use crate::backend::BackendOps;
use crate::contexts::WmCtx;
use std::process::Command;

/// Apply the keyboard layout at the given index using `setxkbmap`.
///
/// This sets a single layout active (not the full list), which is the
/// simplest and most portable approach across X11 and XWayland.
fn apply_layout(ctx: &mut WmCtx, index: usize) -> Result<(), String> {
    let state = &ctx.g().keyboard_layout;
    let layout = state
        .layouts
        .get(index)
        .ok_or_else(|| format!("layout index {index} out of range"))?;
    let variant = state.variants.get(index).map(|s| s.as_str()).unwrap_or("");
    let options = state.options.clone();
    let model = state.model.clone();

    let mut cmd = Command::new("setxkbmap");
    cmd.arg("-layout").arg(layout);
    if !variant.is_empty() {
        cmd.arg("-variant").arg(variant);
    }
    if let Some(ref opts) = options {
        if !opts.is_empty() {
            cmd.arg("-option").arg("").arg("-option").arg(opts);
        }
    }
    if let Some(ref m) = model {
        if !m.is_empty() {
            cmd.arg("-model").arg(m);
        }
    }

    cmd.spawn()
        .map_err(|e| format!("failed to run setxkbmap: {e}"))?;

    // Also apply via the backend abstraction (for Smithay wayland native layout)
    ctx.backend()
        .set_keyboard_layout(layout, variant, options.as_deref(), model.as_deref());

    ctx.g_mut().keyboard_layout.current = index;
    Ok(())
}

/// Switch to a specific keyboard layout by index (0-based).
pub fn set_keyboard_layout(ctx: &mut WmCtx, index: usize) {
    if ctx.g().keyboard_layout.layouts.is_empty() {
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
        .g()
        .keyboard_layout
        .layouts
        .iter()
        .position(|l| l == name);
    match index {
        Some(idx) => {
            set_keyboard_layout(ctx, idx);
            true
        }
        None => false,
    }
}

/// Cycle to the next keyboard layout.
pub fn cycle_keyboard_layout(ctx: &mut WmCtx, forward: bool) {
    let state = &ctx.g().keyboard_layout;
    if state.layouts.is_empty() {
        return;
    }
    let len = state.layouts.len();
    let current = state.current;
    let next = if forward {
        (current + 1) % len
    } else if current == 0 {
        len - 1
    } else {
        current - 1
    };
    set_keyboard_layout(ctx, next);
}

/// Get the current keyboard layout status as a formatted string.
pub fn keyboard_layout_status(ctx: &WmCtx) -> String {
    let state = &ctx.g().keyboard_layout;
    if state.layouts.is_empty() {
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
        state.layouts.len(),
        current_name,
        variant_str
    )
}

/// Get the list of configured keyboard layouts as a formatted string.
pub fn keyboard_layout_list(ctx: &WmCtx) -> String {
    let state = &ctx.g().keyboard_layout;
    if state.layouts.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (i, layout) in state.layouts.iter().enumerate() {
        let variant = state.variants.get(i).map(|s| s.as_str()).unwrap_or("");
        let marker = if i == state.current { "* " } else { "  " };
        if variant.is_empty() {
            out.push_str(&format!("{}{}\n", marker, layout));
        } else {
            out.push_str(&format!("{}{} ({})\n", marker, layout, variant));
        }
    }
    out
}

/// Replace the configured keyboard layouts at runtime.
///
/// This allows IPC clients to reconfigure layouts without editing the TOML file.
pub fn set_keyboard_layouts(ctx: &mut WmCtx, layouts: Vec<String>, variants: Vec<String>) {
    ctx.g_mut().keyboard_layout.layouts = layouts;
    ctx.g_mut().keyboard_layout.variants = variants;
    ctx.g_mut().keyboard_layout.current = 0;
    // Apply the first layout
    if !ctx.g().keyboard_layout.layouts.is_empty() {
        set_keyboard_layout(ctx, 0);
    }
}

/// Apply the initially configured keyboard layout (called during startup).
pub fn init_keyboard_layout(ctx: &mut WmCtx) {
    if !ctx.g().keyboard_layout.layouts.is_empty() {
        set_keyboard_layout(ctx, 0);
    } else {
        // Fallback to environment variables (standard Wayland convention)
        let layout = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
        if !layout.is_empty() {
            let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
            let options = std::env::var("XKB_DEFAULT_OPTIONS").ok();
            let model = std::env::var("XKB_DEFAULT_MODEL").ok();
            log::info!(
                "Initializing keyboard layout from env: layout={}, variant={}, options={:?}, model={:?}",
                layout,
                variant,
                options,
                model
            );
            ctx.backend().set_keyboard_layout(
                &layout,
                &variant,
                options.as_deref(),
                model.as_deref(),
            );
        } else {
            // Last resort: standard US layout
            ctx.backend().set_keyboard_layout("us", "", None, None);
        }
    }
}
