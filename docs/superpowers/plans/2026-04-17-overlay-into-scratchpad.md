# Merge Overlay Into Scratchpad — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fold the overlay system into the scratchpad system by adding an optional `EdgeDirection` field to scratchpads, then delete the overlay module entirely.

**Architecture:** The overlay becomes a named scratchpad (`"instantwm_overlay_scratch"`) with `scratchpad_direction: Some(EdgeDirection)`. All overlay-specific state is removed from `Monitor`. Animation/positioning logic moves from `overlay.rs` into `scratchpad.rs`. Arrow-key direction changes move from bare arrows to Super+Ctrl+Arrow.

**Tech Stack:** Rust, existing instantWM codebase.

---

## File Structure

### Create
- (none — all changes are modifications to existing files)

### Modify
- `src/types/input.rs` — rename `OverlayMode` to `EdgeDirection`
- `src/types/client.rs` — add `scratchpad_direction` field
- `src/types/monitor.rs` — remove `overlaystatus`, `overlaymode`, `overlay` fields
- `src/types/core.rs` — remove unused `OVERLAY_ACTIVATION_ZONE`, `OVERLAY_KEEP_ZONE_X`, `OVERLAY_KEEP_ZONE_Y`
- `src/floating/scratchpad.rs` — absorb overlay animation/positioning, add direction-aware make/show/hide/unmake
- `src/floating/mod.rs` — remove overlay module and exports, add new scratchpad exports
- `src/floating/state.rs` — replace `mon.overlay` check with `scratchpad_direction` check
- `src/floating/movement.rs` — replace `mon.overlay` check with `scratchpad_direction` check
- `src/actions/named.rs` — replace `SetOverlay`/`CreateOverlay` with `OverlayCreate`/`OverlayToggle`
- `src/actions/dispatch.rs` — replace overlay button actions with scratchpad operations
- `src/config/keybindings.rs` — update bindings: Super+W → OverlayToggle, Super+Ctrl+W → OverlayCreate, Super+Ctrl+Arrow → direction changes
- `src/ipc_types.rs` — add `direction` to `ScratchpadCommand::Create` and `ScratchpadInfo`
- `src/ipc/scratchpad.rs` — pass direction to `scratchpad_make`
- `src/layouts/manager.rs` — remove `place_overlay()`
- `src/layouts/algo/overview.rs` — check `scratchpad_direction` instead of `mon.overlay`
- `src/keyboard.rs` — remove bare arrow overlay handling, add Super+Ctrl+Arrow
- `src/tags/shift.rs` — remove overlay-to-set_overlay_mode redirection
- `src/client/visibility.rs` — no change needed (already routes by scratchpad name)
- `src/mouse/drag/move_drop.rs` — check `scratchpad_direction` instead of `mon.overlay`
- `src/globals.rs` — check `scratchpad_direction` for color scheme
- `src/monitor.rs` — remove overlay-related transfer logic if any
- `src/backend/x11/lifecycle.rs` — remove overlay cleanup from `unmanage`
- `src/constants/overlay.rs` — delete
- `src/constants/mod.rs` — remove `pub mod overlay;`
- `src/bin/ctl/commands.rs` — add `--direction` option to scratchpad create
- `src/types/color.rs` — check for overlay scheme references (likely no change needed)

### Delete
- `src/floating/overlay.rs`
- `src/constants/overlay.rs`

---

## Task 1: Rename OverlayMode to EdgeDirection

**Files:**
- Modify: `src/types/input.rs:307-326`
- Modify: `src/constants/animation.rs:49` (comment reference)

This is the foundation — all subsequent tasks reference `EdgeDirection`.

- [ ] **Step 1: Rename the enum in `src/types/input.rs`**

In `src/types/input.rs`, rename the enum and update the doc comment:

```rust
/// The screen edge where an edge-anchored scratchpad slides in/out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeDirection {
    /// Slides down from the top edge (default).
    #[default]
    Top,
    /// Slides in from the right edge.
    Right,
    /// Slides up from the bottom edge.
    Bottom,
    /// Slides in from the left edge.
    Left,
}

impl EdgeDirection {
    /// Returns `true` for modes where the window is sized along the vertical axis.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}
```

- [ ] **Step 2: Run a project-wide find-and-replace**

The rename from `OverlayMode` to `EdgeDirection` affects every file that references it. Run:

```bash
grep -rl "OverlayMode" src/ --include="*.rs"
```

Replace every occurrence of `OverlayMode` with `EdgeDirection` in all Rust source files. This is a mechanical rename — no logic changes. Files will include:
- `src/types/input.rs` (already done)
- `src/floating/overlay.rs`
- `src/types/monitor.rs`
- `src/keyboard.rs`
- `src/tags/shift.rs`

- [ ] **Step 3: Verify compilation**

Run: `cargo check 2>&1 | head -50`
Expected: errors about `OverlayMode` should be gone. Remaining errors are expected (other files still reference overlay module).

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor: rename OverlayMode to EdgeDirection"
```

---

## Task 2: Add scratchpad_direction field to Client and remove overlay fields from Monitor

**Files:**
- Modify: `src/types/client.rs:64-67`
- Modify: `src/types/monitor.rs:123-154`

- [ ] **Step 1: Add `scratchpad_direction` to Client**

In `src/types/client.rs`, add after `scratchpad_restore_tags` (after line 67):

```rust
    /// Edge direction for edge-anchored scratchpads (None for regular scratchpads).
    pub scratchpad_direction: Option<crate::types::input::EdgeDirection>,
```

Since `Client` derives `Default`, `Option` fields default to `None` — no other changes needed.

- [ ] **Step 2: Remove overlay fields from Monitor**

In `src/types/monitor.rs`, remove these three fields:

```rust
    // Remove:
    pub overlaystatus: i32,           // line ~124
    pub overlaymode: EdgeDirection,    // line ~126
    pub overlay: Option<WindowId>,     // line ~154
```

Also remove any `EdgeDirection` import that becomes unused.

- [ ] **Step 3: Remove unused overlay constants from `src/types/core.rs`**

Remove lines ~100-110:

```rust
// Remove these:
pub const OVERLAY_ACTIVATION_ZONE: i32 = 20;
pub const OVERLAY_KEEP_ZONE_X: i32 = 40;
pub const OVERLAY_KEEP_ZONE_Y: i32 = 30;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check 2>&1 | head -80`
Expected: many errors from files still reading `mon.overlay`, `mon.overlaystatus`, `mon.overlaymode`. That's expected — we'll fix them in subsequent tasks.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor: add scratchpad_direction to Client, remove overlay fields from Monitor"
```

---

## Task 3: Absorb overlay animation/positioning into scratchpad module

**Files:**
- Modify: `src/floating/scratchpad.rs`
- Reference: `src/floating/overlay.rs` (copy logic, then delete in Task 8)
- Reference: `src/constants/overlay.rs` (inline constants)
- Reference: `src/constants/animation.rs:49`

This is the largest task — moving all positioning and animation logic from overlay.rs into scratchpad.rs and making it direction-aware.

- [ ] **Step 1: Add constants and position structs to `scratchpad.rs`**

Add at the top of `src/floating/scratchpad.rs`, after the existing imports:

```rust
use crate::backend::BackendOps;
use crate::client::save_border_width;
use crate::constants::animation::OVERLAY_ANIMATION_FRAMES;
use crate::geometry::MoveResizeOptions;
use crate::types::input::EdgeDirection;
use crate::types::Rect;

/// Horizontal margin from screen edge for edge-anchored scratchpads.
const EDGE_MARGIN_X: i32 = 20;
/// Vertical margin from screen edge for edge-anchored scratchpads.
const EDGE_MARGIN_Y: i32 = 40;
/// Horizontal inset (full-width minus this = overlay width).
const EDGE_INSET_X: i32 = 40;
/// Vertical inset (full-height minus this = overlay height).
const EDGE_INSET_Y: i32 = 80;

const OVERLAY_NAME: &str = "instantwm_overlay_scratch";

/// Information needed to position an edge-anchored scratchpad.
#[derive(Debug, Clone, Copy)]
struct EdgePositionInfo {
    direction: EdgeDirection,
    monitor_rect: Rect,
    work_width: i32,
    yoffset: i32,
    client_size: Rect,
}

fn get_initial_edge_rect(info: &EdgePositionInfo) -> Rect {
    let EdgePositionInfo {
        direction,
        monitor_rect,
        work_width,
        yoffset,
        client_size,
    } = *info;

    match direction {
        EdgeDirection::Top => Rect {
            x: monitor_rect.x + EDGE_MARGIN_X,
            y: monitor_rect.y + yoffset - client_size.h,
            w: work_width - EDGE_INSET_X,
            h: client_size.h,
        },
        EdgeDirection::Right => Rect {
            x: monitor_rect.x + monitor_rect.w - EDGE_MARGIN_X,
            y: monitor_rect.y + EDGE_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - EDGE_INSET_Y,
        },
        EdgeDirection::Bottom => Rect {
            x: monitor_rect.x + EDGE_MARGIN_X,
            y: monitor_rect.y + monitor_rect.h,
            w: work_width - EDGE_INSET_X,
            h: client_size.h,
        },
        EdgeDirection::Left => Rect {
            x: monitor_rect.x - client_size.w + EDGE_MARGIN_X,
            y: monitor_rect.y + EDGE_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - EDGE_INSET_Y,
        },
    }
}

fn get_target_edge_rect(info: &EdgePositionInfo) -> Rect {
    let EdgePositionInfo {
        direction,
        monitor_rect,
        work_width,
        yoffset,
        client_size,
    } = *info;

    match direction {
        EdgeDirection::Top => Rect {
            x: monitor_rect.x + EDGE_MARGIN_X,
            y: monitor_rect.y + yoffset,
            w: work_width - EDGE_INSET_X,
            h: client_size.h,
        },
        EdgeDirection::Right => Rect {
            x: monitor_rect.x + monitor_rect.w - client_size.w,
            y: monitor_rect.y + EDGE_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - EDGE_INSET_Y,
        },
        EdgeDirection::Bottom => Rect {
            x: monitor_rect.x + EDGE_MARGIN_X,
            y: monitor_rect.y + monitor_rect.h - client_size.h,
            w: work_width - EDGE_INSET_X,
            h: client_size.h,
        },
        EdgeDirection::Left => Rect {
            x: monitor_rect.x,
            y: monitor_rect.y + EDGE_MARGIN_Y,
            w: client_size.w,
            h: monitor_rect.h - EDGE_INSET_Y,
        },
    }
}

#[derive(Debug, Clone, Copy)]
struct HideAnimationInfo {
    direction: EdgeDirection,
    monitor_rect: Rect,
    client_x: i32,
    client_size: Rect,
}

fn get_hide_animation_rect(info: &HideAnimationInfo) -> Rect {
    let HideAnimationInfo {
        direction,
        monitor_rect,
        client_x,
        client_size,
    } = *info;

    match direction {
        EdgeDirection::Top => Rect {
            x: client_x,
            y: -client_size.h,
            w: 0,
            h: 0,
        },
        EdgeDirection::Right => Rect {
            x: monitor_rect.x + monitor_rect.w,
            y: monitor_rect.y + EDGE_MARGIN_Y,
            w: 0,
            h: 0,
        },
        EdgeDirection::Bottom => Rect {
            x: client_x,
            y: monitor_rect.y + monitor_rect.h,
            w: 0,
            h: 0,
        },
        EdgeDirection::Left => Rect {
            x: monitor_rect.x - client_size.w,
            y: EDGE_MARGIN_Y,
            w: 0,
            h: 0,
        },
    }
}
```

- [ ] **Step 2: Modify `scratchpad_make` to accept direction**

Replace the existing `scratchpad_make` function:

```rust
pub fn scratchpad_make(
    ctx: &mut WmCtx,
    name: &str,
    window_id: Option<WindowId>,
    direction: Option<EdgeDirection>,
    status: ScratchpadInitialStatus,
) {
    if name.is_empty() {
        return;
    }

    let target = selected_or_explicit_window(ctx, window_id);
    let Some(selected_window) = target else {
        return;
    };

    if scratchpad_find(ctx.core().globals(), name).is_some() {
        return;
    }

    let Some(client) = ctx.client_mut(selected_window) else {
        return;
    };

    let was_scratchpad = client.is_scratchpad();
    let old_tags = if was_scratchpad {
        crate::types::TagMask::EMPTY
    } else {
        client.tags
    };

    client.scratchpad_name = name.to_string();
    client.scratchpad_direction = direction;

    if !was_scratchpad {
        client.scratchpad_restore_tags = old_tags;
    }

    client.set_tag_mask(crate::types::TagMask::SCRATCHPAD);
    client.is_sticky = false;

    if !client.is_floating {
        client.is_floating = true;
    }

    if let Some(dir) = direction {
        let (mon_ww, mon_wh) = {
            let mon = ctx.core().globals().selected_monitor();
            (mon.work_rect.w, mon.work_rect.h)
        };
        if dir.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
        save_border_width(client);
        client.border_width = 0;
        client.is_locked = true;
    }

    crate::client::hide(ctx, selected_window);

    if matches!(status, ScratchpadInitialStatus::Shown) {
        let _ = scratchpad_show_name(ctx, name);
    }
}
```

- [ ] **Step 3: Modify `scratchpad_show_name` to animate edge-anchored scratchpads**

Replace the existing `scratchpad_show_name` function:

```rust
pub fn scratchpad_show_name(ctx: &mut WmCtx, name: &str) -> Result<String, String> {
    let Some(found) = scratchpad_find(ctx.core().globals(), name) else {
        return Err(format!("scratchpad '{}' not found", name));
    };

    let was_sticky = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .is_some_and(|c| c.is_sticky);

    if was_sticky {
        return Ok(format!("scratchpad '{}' is already visible", name));
    }

    let direction = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .and_then(|c| c.scratchpad_direction);

    let current_mon = ctx.core().globals().selected_monitor_id();
    let target_mon = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .map(|c| c.monitor_id)
        .unwrap_or(current_mon);

    if let Some(client) = ctx.client_mut(found) {
        client.is_sticky = true;
        client.is_floating = true;
    }

    if target_mon != current_mon {
        move_client_to_monitor(ctx.core_mut().globals_mut(), found, current_mon);
    }

    let focusfollowsmouse = ctx.core().globals().behavior.focus_follows_mouse;

    if let Some(dir) = direction {
        // Edge-anchored scratchpad: detach, animate in from edge
        ctx.core_mut().globals_mut().detach(found);
        ctx.core_mut().globals_mut().detach_z_order(found);

        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) {
            client.monitor_id = current_mon;
        }

        ctx.core_mut().globals_mut().attach(found);
        ctx.core_mut().globals_mut().attach_z_order_top(found);

        let tags = ctx
            .core_mut()
            .globals_mut()
            .selected_monitor()
            .selected_tags();

        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&found) {
            client.border_width = 0;
            client.set_tag_mask(tags);
        }

        // Calculate y offset (bar height + fullscreen check)
        let yoffset = {
            let mon = ctx.core().globals().selected_monitor();
            let showbar = mon.showbar_for_mask(tags);
            let bar_height = ctx.core().globals().cfg.bar_height;
            let mut offset = if showbar { bar_height } else { 0 };
            for (_win, c) in mon.iter_clients(ctx.core().globals().clients.map()) {
                if c.tags.intersects(tags) && c.is_true_fullscreen() {
                    offset = 0;
                    break;
                }
            }
            offset
        };

        let (mon_rect, mon_ww, client_w, client_h) = {
            let mon = ctx.core().globals().monitor(current_mon).unwrap();
            let client = ctx.client(found).unwrap();
            (
                mon.monitor_rect,
                mon.work_rect.w,
                client.geo.w,
                client.geo.h,
            )
        };

        let pos_info = EdgePositionInfo {
            direction: dir,
            monitor_rect: mon_rect,
            work_width: mon_ww,
            yoffset,
            client_size: Rect {
                x: 0,
                y: 0,
                w: client_w,
                h: client_h,
            },
        };

        // Start off-screen
        let initial_rect = get_initial_edge_rect(&pos_info);
        ctx.move_resize(found, initial_rect, MoveResizeOptions::hinted_immediate(true));

        // Animate to target
        ctx.backend().raise_window_visual_only(found);
        let target_rect = get_target_edge_rect(&pos_info);
        ctx.move_resize(found, target_rect, MoveResizeOptions::animate_to(OVERLAY_ANIMATION_FRAMES));
    } else {
        let is_hidden = ctx
            .core()
            .globals()
            .clients
            .get(&found)
            .map(|c| c.is_hidden)
            .unwrap_or(false);
        if is_hidden {
            crate::client::show_window(ctx, found);
        } else {
            let mid = ctx.core().globals().selected_monitor_id();
            crate::focus::focus_soft(ctx, Some(found));
            arrange(ctx, Some(mid));
            crate::layouts::sync_monitor_z_order(ctx, mid);
        }
    }

    crate::focus::focus_soft(ctx, Some(found));
    ctx.backend().raise_window_visual_only(found);

    if focusfollowsmouse {
        ctx.warp_cursor_to_client(found);
    }

    Ok(format!("shown scratchpad '{}'", name))
}
```

- [ ] **Step 4: Modify `scratchpad_hide_name` to animate edge-anchored scratchpads**

Replace the existing `scratchpad_hide_name` function:

```rust
pub fn scratchpad_hide_name(ctx: &mut WmCtx, name: &str) {
    let Some(found) = scratchpad_find(ctx.core().globals(), name) else {
        return;
    };

    let direction = ctx
        .core()
        .globals()
        .clients
        .get(&found)
        .and_then(|c| c.scratchpad_direction);

    let Some(client) = ctx.client_mut(found) else {
        return;
    };
    if !client.is_sticky {
        return;
    }

    client.is_sticky = false;
    client.set_tag_mask(crate::types::TagMask::SCRATCHPAD);

    if let Some(dir) = direction {
        let (mon_rect, client_x, client_w, client_h) = {
            let client = ctx.client(found).unwrap();
            let mon = ctx.core().globals().selected_monitor();
            (
                mon.monitor_rect,
                client.geo.x,
                client.geo.w,
                client.geo.h,
            )
        };

        let hide_info = HideAnimationInfo {
            direction: dir,
            monitor_rect: mon_rect,
            client_x,
            client_size: Rect {
                x: 0,
                y: 0,
                w: client_w,
                h: client_h,
            },
        };

        let hide_rect = get_hide_animation_rect(&hide_info);
        ctx.move_resize(found, hide_rect, MoveResizeOptions::animate_to(OVERLAY_ANIMATION_FRAMES));
    }

    crate::client::hide(ctx, found);
}
```

- [ ] **Step 5: Modify `scratchpad_unmake` to restore edge-anchored state**

Replace the existing `scratchpad_unmake` function:

```rust
pub fn scratchpad_unmake(ctx: &mut WmCtx, window_id: Option<WindowId>) {
    let target = selected_or_explicit_window(ctx, window_id);
    let Some(selected_window) = target else {
        return;
    };

    let monitor_tags = ctx.core().globals().selected_monitor().selected_tags();

    let Some(client) = ctx.client(selected_window) else {
        return;
    };
    if !client.is_scratchpad() {
        return;
    }
    let restore_tags = client.scratchpad_restore_tags;
    let monitor_id = client.monitor_id;
    let had_direction = client.scratchpad_direction.is_some();

    let mut was_hidden = false;
    if let Some(client) = ctx.client_mut(selected_window) {
        was_hidden = client.is_hidden;
        client.set_tag_mask(if !restore_tags.is_empty() {
            restore_tags
        } else {
            monitor_tags
        });

        if had_direction {
            client.border_width = client.old_border_width;
            client.is_locked = false;
            client.scratchpad_direction = None;
        }
    }

    if was_hidden {
        crate::client::show_window(ctx, selected_window);
    } else {
        arrange(ctx, Some(monitor_id));
    }
}
```

- [ ] **Step 6: Add `set_scratchpad_direction` and overlay scratchpad helpers**

Add these new public functions to `scratchpad.rs`:

```rust
/// Change the edge direction of an existing edge-anchored scratchpad.
pub fn set_scratchpad_direction(ctx: &mut WmCtx, win: WindowId, direction: EdgeDirection) {
    let was_sticky = ctx.client(win).is_some_and(|c| c.is_sticky);

    let (mon_ww, mon_wh) = {
        let mon = ctx.core().globals().selected_monitor();
        (mon.work_rect.w, mon.work_rect.h)
    };

    if let Some(client) = ctx.client_mut(win) {
        client.scratchpad_direction = Some(direction);
        if direction.is_vertical() {
            client.geo.h = mon_wh / 3;
        } else {
            client.geo.w = mon_ww / 3;
        }
    }

    if was_sticky {
        let name = ctx.client(win).map(|c| c.scratchpad_name.clone()).unwrap_or_default();
        if !name.is_empty() {
            scratchpad_hide_name(ctx, &name);
            let _ = scratchpad_show_name(ctx, &name);
        }
    }
}

/// Create or replace the overlay scratchpad from the focused window.
pub fn overlay_create(ctx: &mut WmCtx) {
    // If one exists, unmake it first
    if let Some(existing) = scratchpad_find(ctx.core().globals(), OVERLAY_NAME) {
        scratchpad_unmake(ctx, Some(existing));
    }

    let Some(selected) = ctx.selected_client() else {
        return;
    };

    // Exit fullscreen if needed
    let is_fullscreen = ctx.client(selected).is_some_and(|c| c.is_true_fullscreen());
    if is_fullscreen {
        crate::floating::toggle_maximized(ctx);
    }

    scratchpad_make(
        ctx,
        OVERLAY_NAME,
        None,
        Some(EdgeDirection::Top),
        ScratchpadInitialStatus::Shown,
    );
}

/// Toggle the overlay scratchpad visibility.
pub fn overlay_toggle(ctx: &mut WmCtx) {
    scratchpad_toggle(ctx, Some(OVERLAY_NAME));
}
```

- [ ] **Step 7: Update `ScratchpadInfo` to include direction**

Update the `ScratchpadInfo` struct and `collect_scratchpad_info`:

```rust
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct ScratchpadInfo {
    pub name: String,
    pub visible: bool,
    pub window_id: Option<u32>,
    pub monitor: Option<usize>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub floating: bool,
    pub fullscreen: bool,
    pub direction: Option<String>,
}

// In collect_scratchpad_info, update the ScratchpadInfo construction:
pub fn collect_scratchpad_info(g: &Globals) -> Vec<ScratchpadInfo> {
    let mut scratchpads = Vec::new();

    for c in g.clients.values() {
        if c.is_scratchpad() {
            scratchpads.push(ScratchpadInfo {
                name: c.scratchpad_name.clone(),
                visible: c.is_sticky,
                window_id: Some(c.win.0),
                monitor: Some(c.monitor_id.index()),
                x: Some(c.geo.x),
                y: Some(c.geo.y),
                width: Some(c.geo.w),
                height: Some(c.geo.h),
                floating: c.is_floating,
                fullscreen: c.is_fullscreen,
                direction: c.scratchpad_direction.map(|d| format!("{:?}", d).to_lowercase()),
            });
        }
    }

    scratchpads
}
```

- [ ] **Step 8: Verify compilation**

Run: `cargo check 2>&1 | head -80`
Expected: errors only from files still referencing overlay module/imports (fixed in later tasks).

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "feat: absorb overlay positioning into scratchpad module with direction support"
```

---

## Task 4: Update floating module exports

**Files:**
- Modify: `src/floating/mod.rs`

- [ ] **Step 1: Remove overlay module, add new exports**

Replace the full contents of `src/floating/mod.rs`:

```rust
//! Floating window management.
//!
//! - [`snap`]    — snap positions, the navigation matrix, apply/change/reset snap
//! - [`state`]   — save/restore float geometry & border width; set_window_mode;
//!                  toggle/set/change floating state; toggle_maximized
//! - [`movement`] — keyboard move, resize, center window, scale client
//! - [`batch`]   — save/restore all floating positions, distribute clients
//! - [`helpers`] — check_floating, visible_client, has_tiling_layout, apply_size
//! - [`scratchpad`] — named floating windows that can be toggled visible/hidden,
//!                     with optional edge-anchored positioning and slide animation

mod batch;
mod helpers;
mod movement;
pub mod scratchpad;
mod snap;
mod state;

// -- snap --
pub use snap::{change_snap, reset_snap};

// -- movement --
pub use movement::{center_window, key_resize};

// -- batch --
pub use batch::{distribute_clients, restore_all_floating, save_all_floating};

// -- state --
pub use state::{
    WindowMode, save_floating_geometry, set_window_mode, toggle_floating, toggle_maximized,
};

// -- scratchpad --
pub use scratchpad::{
    overlay_create, overlay_toggle, scratchpad_find, scratchpad_hide_name, scratchpad_make,
    scratchpad_show_name, scratchpad_toggle, scratchpad_unmake, set_scratchpad_direction,
    unhide_one,
};
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check 2>&1 | head -80`

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "refactor: remove overlay module, export new scratchpad functions"
```

---

## Task 5: Update actions (NamedAction variants and dispatch)

**Files:**
- Modify: `src/actions/named.rs:141-142,86-177`
- Modify: `src/actions/dispatch.rs:4,184-185`

- [ ] **Step 1: Replace NamedAction variants in `src/actions/named.rs`**

Replace the two overlay variants (lines 141-142):

```rust
    // Remove:
    //   SetOverlay | "set_overlay" | set overlay
    //   CreateOverlay | "create_overlay" | create overlay from focused client
    // Add:
    OverlayToggle | "overlay_toggle" | toggle overlay scratchpad visibility
    OverlayCreate | "overlay_create" | create overlay scratchpad from focused window
```

Find the dispatch block for `SetOverlay` and `CreateOverlay` and replace with:

```rust
OverlayCreate => {
    crate::floating::overlay_create(ctx);
}
OverlayToggle => {
    crate::floating::overlay_toggle(ctx);
}
```

Also update `ScratchpadToggle` to pass `None` for the new `direction` parameter:

```rust
ScratchpadToggle => {
    const DEFAULT_NAME: &str = "instantwm_scratchpad";
    if scratchpad_find(ctx.core().globals(), DEFAULT_NAME).is_some() {
        scratchpad_toggle(ctx, Some(DEFAULT_NAME));
    } else {
        scratchpad_make(ctx, DEFAULT_NAME, None, None, ScratchpadInitialStatus::Shown);
    }
}
```

- [ ] **Step 2: Update button action dispatch in `src/actions/dispatch.rs`**

Remove the overlay import on line 4:

```rust
// Change:
use crate::floating::{hide_overlay, show_overlay, toggle_floating};
// To:
use crate::floating::toggle_floating;
```

Replace lines 184-185:

```rust
        // Remove:
        //   ButtonAction::HideOverlay => hide_overlay(ctx),
        //   ButtonAction::ShowOverlay => show_overlay(ctx),
        // Add:
        ButtonAction::HideOverlay => {
            crate::floating::scratchpad_hide_name(ctx, "instantwm_overlay_scratch");
        }
        ButtonAction::ShowOverlay => {
            let _ = crate::floating::scratchpad_show_name(ctx, "instantwm_overlay_scratch");
        }
```

Note: `ButtonAction::HideOverlay` and `ButtonAction::ShowOverlay` variants still exist in the enum definition elsewhere. If we want to fully clean up, those could be renamed later, but they work functionally as-is.

- [ ] **Step 3: Verify compilation**

Run: `cargo check 2>&1 | head -80`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor: replace overlay actions with scratchpad-based overlay actions"
```

---

## Task 6: Update keybindings

**Files:**
- Modify: `src/config/keybindings.rs:102-103`
- Modify: `src/keyboard.rs:289-357`
- Modify: `src/tags/shift.rs:36-45`

- [ ] **Step 1: Update keybindings in `src/config/keybindings.rs`**

Replace the overlay keybindings (lines 102-103):

```rust
    // Remove:
    //   key!(MODKEY, XK_W => named(NamedAction::SetOverlay)),
    //   key!(MODKEY | CONTROL, XK_W => named(NamedAction::CreateOverlay)),
    // Add:
    key!(MODKEY, XK_W => named(NamedAction::OverlayToggle)),
    key!(MODKEY | CONTROL, XK_W => named(NamedAction::OverlayCreate)),
    key!(MODKEY | CONTROL, XK_Up => named(NamedAction::None)),
    key!(MODKEY | CONTROL, XK_Down => named(NamedAction::None)),
    key!(MODKEY | CONTROL, XK_Left => named(NamedAction::None)),
    key!(MODKEY | CONTROL, XK_Right => named(NamedAction::None)),
```

Wait — the Super+Ctrl+Arrow keybindings should dispatch the direction change on the overlay scratchpad. Since there's no existing `NamedAction` for per-direction changes, we need to add four new variants or handle them in the keyboard module. The cleanest approach is to add a single `NamedAction::OverlayDirection(arg)` that takes a direction string, matching the pattern of other arg-taking actions.

Actually, looking at the `key!` macro, some actions take args via `key!(MOD, KEY => named_with_arg(NamedAction::Foo, "arg"))`. Let's add four variants:

In `src/actions/named.rs`, add these variants:

```rust
    OverlayDirectionUp | "overlay_direction_up" | set overlay direction to top
    OverlayDirectionDown | "overlay_direction_down" | set overlay direction to bottom
    OverlayDirectionLeft | "overlay_direction_left" | set overlay direction to left
    OverlayDirectionRight | "overlay_direction_right" | set overlay direction to right
```

And their dispatch:

```rust
OverlayDirectionUp => overlay_set_direction(ctx, EdgeDirection::Top),
OverlayDirectionDown => overlay_set_direction(ctx, EdgeDirection::Bottom),
OverlayDirectionLeft => overlay_set_direction(ctx, EdgeDirection::Left),
OverlayDirectionRight => overlay_set_direction(ctx, EdgeDirection::Right),
```

Add a helper in `named.rs`:

```rust
fn overlay_set_direction(ctx: &mut WmCtx, dir: EdgeDirection) {
    use crate::floating::scratchpad::{scratchpad_find, set_scratchpad_direction};
    const NAME: &str = "instantwm_overlay_scratch";
    if let Some(win) = scratchpad_find(ctx.core().globals(), NAME) {
        set_scratchpad_direction(ctx, win, dir);
    }
}
```

Then in `keybindings.rs`:

```rust
    key!(MODKEY, XK_W => named(NamedAction::OverlayToggle)),
    key!(MODKEY | CONTROL, XK_W => named(NamedAction::OverlayCreate)),
    key!(MODKEY | CONTROL, XK_Up => named(NamedAction::OverlayDirectionUp)),
    key!(MODKEY | CONTROL, XK_Down => named(NamedAction::OverlayDirectionDown)),
    key!(MODKEY | CONTROL, XK_Left => named(NamedAction::OverlayDirectionLeft)),
    key!(MODKEY | CONTROL, XK_Right => named(NamedAction::OverlayDirectionRight)),
```

- [ ] **Step 2: Remove bare arrow key overlay handling in `src/keyboard.rs`**

In `up_press` (around lines 302-305), remove the overlay check:

```rust
    // Remove:
    //   if Some(win) == overlay_win {
    //       crate::floating::set_overlay_mode(ctx, EdgeDirection::Top);
    //       return;
    //   }
```

Also remove the `overlay_win` variable binding that reads from `mon.overlay`.

In `down_press` (around lines 349-352), remove the overlay check:

```rust
    // Remove:
    //   if Some(win) == overlay_win {
    //       crate::floating::set_overlay_mode(ctx, EdgeDirection::Bottom);
    //       return;
    //   }
```

Also remove the `overlay_win` variable binding.

- [ ] **Step 3: Remove overlay redirection in `src/tags/shift.rs`**

Remove lines 36-45:

```rust
    // Remove the entire block:
    //   if Some(win) == overlay_win {
    //       let mode = match dir { ... };
    //       crate::floating::set_overlay_mode(ctx, mode);
    //       return;
    //   }
```

Also remove the `overlay_win` variable binding that reads from `mon.overlay`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check 2>&1 | head -80`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor: update keybindings for merged overlay/scratchpad"
```

---

## Task 7: Update all remaining overlay references

**Files:**
- Modify: `src/layouts/manager.rs:89,164-187`
- Modify: `src/layouts/algo/overview.rs:79`
- Modify: `src/floating/state.rs:132`
- Modify: `src/floating/movement.rs:79-82`
- Modify: `src/mouse/drag/move_drop.rs:116-132`
- Modify: `src/globals.rs:900-913`
- Modify: `src/backend/x11/lifecycle.rs:521-525`
- Modify: `src/monitor.rs` (remove overlay-specific transfer logic if any)

- [ ] **Step 1: Remove `place_overlay` from layout manager**

In `src/layouts/manager.rs`, remove the entire `place_overlay` function (lines 164-187) and its call site (around line 89).

- [ ] **Step 2: Update overview layout in `src/layouts/algo/overview.rs`**

Replace line 79:

```rust
    // Remove:
    //   let is_overlay = m.overlay == Some(win);
    // Add:
    let is_edge_scratchpad = client.scratchpad_direction.is_some();

    // Update line 81:
    //   if is_hidden || is_overlay {
    // To:
    if is_hidden || is_edge_scratchpad {
```

- [ ] **Step 3: Update `toggle_floating` guard in `src/floating/state.rs`**

Replace line 132:

```rust
    // Remove:
    //   Some(sel) if Some(sel) != mon.overlay => {
    // Add:
    Some(sel) if !ctx.client(sel).is_some_and(|c| c.scratchpad_direction.is_some()) => {
```

- [ ] **Step 4: Update `center_window` guard in `src/floating/movement.rs`**

Replace lines 79-82:

```rust
    // Remove:
    //   let is_overlay = ctx.core().globals().selected_monitor().overlay == Some(win);
    //   if is_overlay { return; }
    // Add:
    let is_edge_scratchpad = ctx.client(win).is_some_and(|c| c.scratchpad_direction.is_some());
    if is_edge_scratchpad {
        return;
    }
```

- [ ] **Step 5: Update drag guard in `src/mouse/drag/move_drop.rs`**

Replace lines 116-132:

```rust
    let (sel, fullscreen) = {
        let g = ctx.core_mut().globals_mut();
        let mon = g.selected_monitor();
        let sel = mon.sel?;
        (sel, mon.fullscreen)
    };
    let c = ctx.core().client(sel)?;
    let is_true_fullscreen = c.is_true_fullscreen();
    let is_edge_scratchpad = c.scratchpad_direction.is_some();
    let is_fullscreen = Some(sel) == fullscreen;

    if is_true_fullscreen {
        return None;
    }
    if is_edge_scratchpad {
        return None;
    }
```

Remove the `overlay` variable and `is_overlay` check.

- [ ] **Step 6: Update color scheme in `src/globals.rs`**

Replace the overlay detection (around line 900):

```rust
    // Remove:
    //   let is_overlay = selmon.overlay == Some(c.win);
    // Add:
    let is_overlay = c.scratchpad_direction.is_some();
```

- [ ] **Step 7: Remove overlay cleanup from X11 unmanage**

In `src/backend/x11/lifecycle.rs`, remove lines 521-525:

```rust
    // Remove:
    //   if mon.overlay == Some(win) {
    //       mon.overlay = None;
    //   }
```

The overlay is now a scratchpad — when the window is destroyed, the scratchpad metadata on the client will be cleaned up naturally through the normal unmanage flow. However, if the scratchpad is hidden, we may need to ensure the destroyed window's scratchpad name doesn't linger. Check if the existing unmanage flow removes the client from the clients map (it should — if the client is removed, `scratchpad_find` will no longer find it).

- [ ] **Step 8: Verify compilation**

Run: `cargo check 2>&1 | head -80`

- [ ] **Step 9: Commit**

```bash
git add -A && git commit -m "refactor: replace all mon.overlay checks with scratchpad_direction"
```

---

## Task 8: Delete overlay module and constants

**Files:**
- Delete: `src/floating/overlay.rs`
- Delete: `src/constants/overlay.rs`
- Modify: `src/constants/mod.rs:2`

- [ ] **Step 1: Delete overlay files**

```bash
rm src/floating/overlay.rs src/constants/overlay.rs
```

- [ ] **Step 2: Remove overlay module declaration from constants**

In `src/constants/mod.rs`, remove:

```rust
pub mod overlay;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check 2>&1 | head -80`
Expected: clean compilation, or only remaining errors in Wayland backend files that use "overlay" for unrelated concepts (layer shell rendering).

If there are remaining compilation errors from the Wayland backend files, they are likely about the `mon.overlay` field being removed from `Monitor`. Check and fix each one:

```bash
cargo check 2>&1 | grep "error" | head -20
```

Each remaining reference to `mon.overlay`, `mon.overlaystatus`, or `mon.overlaymode` must be removed or replaced.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor: delete overlay module and constants"
```

---

## Task 9: Update IPC and CLI for direction parameter

**Files:**
- Modify: `src/ipc_types.rs:187-203` (ScratchpadCommand::Create)
- Modify: `src/ipc/scratchpad.rs:52-59` (create dispatch)
- Modify: `src/bin/ctl/commands.rs:132-140` (ScratchpadAction::Create)

- [ ] **Step 1: Add direction to ScratchpadCommand::Create**

In `src/ipc_types.rs`, update `ScratchpadCommand::Create`:

```rust
        Create {
            name: String,
            window_id: Option<u32>,
            status: ScratchpadInitialStatus,
            direction: Option<String>,
        },
```

- [ ] **Step 2: Update IPC dispatch to pass direction**

In `src/ipc/scratchpad.rs`, update the `Create` branch:

```rust
        ScratchpadCommand::Create { name, window_id, status, direction } => {
            let dir = direction.as_deref().and_then(|d| match d.to_lowercase().as_str() {
                "top" => Some(EdgeDirection::Top),
                "right" => Some(EdgeDirection::Right),
                "bottom" => Some(EdgeDirection::Bottom),
                "left" => Some(EdgeDirection::Left),
                _ => None,
            });
            scratchpad_make(&mut wm.ctx(), &name, window_id.map(WindowId::from), dir, status);
            Response::ok()
        }
```

Add the import:

```rust
use crate::types::input::EdgeDirection;
```

- [ ] **Step 3: Update CLI**

In `src/bin/ctl/commands.rs`, add `--direction` to `ScratchpadAction::Create`:

```rust
      #[command(alias = "make")]
      Create {
          #[arg(default_value = "instantwm_scratchpad")]
          name: String,
          #[arg(long, short = 'w')]
          window_id: Option<u32>,
          #[arg(long, default_value = "hidden")]
          status: ScratchpadInitialStatus,
          #[arg(long)]
          direction: Option<String>,
      },
```

In the `command_to_ipc` function, update the `ScratchpadAction::Create` mapping to pass `direction`:

```rust
      ScratchpadAction::Create { name, window_id, status, direction } => {
          IpcCommand::Scratchpad(ScratchpadCommand::Create {
              name,
              window_id,
              status,
              direction,
          })
      }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check 2>&1 | head -80`

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add direction parameter to scratchpad IPC/CLI"
```

---

## Task 10: Final cleanup and verification

**Files:**
- All remaining files with overlay references

- [ ] **Step 1: Search for remaining overlay references**

```bash
grep -rn "overlay" src/ --include="*.rs" | grep -v "overlay_windows_for_render\|is_unmanaged_x11_overlay\|is_overlay.*false\|OverlayFocus\|Overlay.*Scheme\|WindowType::Overlay\|x11_overlay\|// overlay\|overlay.*comment\|should_suppress"
```

Fix any remaining compilation errors. The Wayland backend files (`src/backend/wayland/`, `src/wayland/`) use "overlay" for layer-shell/X11-override concepts that are unrelated to the overlay scratchpad feature and should not need changes.

- [ ] **Step 2: Full build**

```bash
cargo build 2>&1 | tail -20
```

Expected: clean build.

- [ ] **Step 3: Run tests**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 4: Final commit**

```bash
git add -A && git commit -m "refactor: final cleanup for overlay-into-scratchpad merge"
```
