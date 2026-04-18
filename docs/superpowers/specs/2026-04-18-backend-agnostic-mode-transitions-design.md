# Backend-Agnostic Mode Transitions

## Problem

The `ClientMode` enum (Tiling, Floating, TrueFullscreen, FakeFullscreen, Maximized) was introduced to replace individual booleans, but the maximize and fullscreen state transitions are independently implemented in three places — X11, XDG shell, and XWayland — each directly manipulating client state instead of calling a shared function. This caused the X11-side `toggle_maximized` to diverge from the Wayland compositor handlers (it didn't set `c.mode` at all).

## Scope

Extract two backend-agnostic state transition functions:

1. `set_maximized(core, win, enter: bool)` — maximize/unmaximize
2. `set_fullscreen(core, win, enter: bool)` — fullscreen/unfullscreen

These handle only state management (mode, float_geo, border_width, mon.maximized). Backend handlers call them, then perform backend-specific I/O (configure_window, send_configure, move_resize).

## New file: `src/client/mode.rs`

### `set_maximized`

```rust
pub enum MaximizedOutcome {
    Entered { base: BaseClientMode },
    Exited { base: BaseClientMode },
}

pub fn set_maximized(core: &mut CoreCtx, win: WindowId, enter: bool) -> Option<MaximizedOutcome>
```

**Enter** (maximize):
1. Save `float_geo = geo` if the window is not already floating
2. Set `c.mode = c.mode.as_maximized()`
3. Set `mon.maximized = Some(win)` on the window's monitor
4. Return `Entered { base }` where `base` is the pre-maximize base mode

**Exit** (unmaximize):
1. Record `base = c.mode.base_mode()`
2. Set `c.mode = c.mode.restored()`
3. Clear `mon.maximized` (via `clear_maximized_for`)
4. Return `Exited { base }`

Callers use the returned base mode to decide whether to restore float geometry, apply X11 size hints, etc.

### `set_fullscreen`

```rust
pub enum FullscreenOutcome {
    Entered { was_floating: bool },
    Exited,
}

pub fn set_fullscreen(core: &mut CoreCtx, win: WindowId, enter: bool) -> Option<FullscreenOutcome>
```

**Enter** (fullscreen):
1. Record `was_floating = c.mode.is_floating()`
2. Save border width via `c.save_border_width()`
3. Set `c.mode = c.mode.as_fullscreen()`
4. Set `c.border_width = 0`
5. Return `Entered { was_floating }`

**Exit** (unfullscreen):
1. Set `c.mode = c.mode.restored()`
2. Restore border width via `restore_border_width()`
3. Return `Exited`

Callers use `was_floating` to decide whether to animate the expansion and whether to reposition the window.

## Caller changes

### `src/floating/state.rs` — `toggle_maximized`

Replace direct state manipulation with a call to `set_maximized`. Keep the animation suppression, arrange pass, and raise logic.

### `src/client/fullscreen.rs` — `set_fullscreen_x11`

Replace direct state manipulation with a call to `set_fullscreen`. Keep only X11-specific I/O: `_NET_WM_STATE` atom writes, `configure_window` calls, animation via `move_resize`.

### `src/backend/wayland/compositor/xdg_shell.rs`

- `fullscreen_request` / `unfullscreen_request`: call `set_fullscreen`, then do `surface.with_pending_state()` + `send_configure()`.
- `maximize_request` / `unmaximize_request`: call `set_maximized`, then do `surface.with_pending_state()` + `send_configure()`.

### `src/backend/wayland/compositor/xwayland.rs`

- `fullscreen_request` / `unfullscreen_request`: call `set_fullscreen`, then do `window.set_fullscreen()`.
- `maximize_request` / `unmaximize_request`: call `set_maximized`, then do `move_resize` to work_rect or float_geo.

### `src/mouse/drag/move_drop.rs`

Already uses `c.mode.is_maximized()` from the previous fix. No further changes needed.

## What stays backend-specific

- X11: `_NET_WM_STATE` atom writes, `configure_window`, border width X11 protocol
- Wayland XDG: `surface.with_pending_state()`, `send_configure()`
- Wayland XWayland: `window.set_fullscreen()`, `move_resize` to work_rect
- Animation (X11-only for now)
- Arrange passes and raise calls (these use `WmCtx`, not `CoreCtx`)

## Not in scope

- Client initialization (manage/adopt flow)
- Floating policy detection
- Urgent state
- Border width configuration beyond save/restore
