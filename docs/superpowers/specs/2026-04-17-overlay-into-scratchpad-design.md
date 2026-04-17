# Merge Overlay System Into Scratchpad System

**Date:** 2026-04-17
**Status:** Draft

## Motivation

The overlay system and scratchpad system are conceptually the same thing: a floating window that can be shown/hidden on demand. The overlay adds edge-anchored positioning with slide animation, but carries significant tech debt:

- `overlaystatus: i32` used as a boolean
- Same `WindowId` stored on every monitor
- `place_overlay()` conflicts with overlay's own positioning
- No cleanup when the overlay window is destroyed externally
- `reset_overlay` doesn't clear `mon.overlay`

The scratchpad system is clean and well-structured. Folding the overlay into scratchpads eliminates all overlay-specific state from `Monitor` and fixes these bugs by removing the duplicated system entirely.

## Data Model

### Rename

`OverlayMode` in `src/types/input.rs` is renamed to `EdgeDirection`. Same variants (Top, Right, Bottom, Left), same `is_vertical()` helper.

### Client (`src/types/client.rs`)

Add one field:

```rust
pub scratchpad_direction: Option<EdgeDirection>,
```

`None` for regular scratchpads, `Some(Top)` etc. for edge-anchored ones. The overlay is now just "a scratchpad with a direction."

### Monitor (`src/types/monitor.rs`)

Remove three fields:

- `overlaystatus: i32`
- `overlaymode: OverlayMode`
- `overlay: Option<WindowId>`

No more global overlay state per monitor. The overlay is looked up by name (`"instantwm_overlay_scratch"`).

### IPC Types

`ScratchpadInfo` gains `direction: Option<EdgeDirection>`.

`ScratchpadCommand::Create` gains an optional `direction` parameter.

## Scratchpad Lifecycle

### `scratchpad_make`

New signature: `scratchpad_make(ctx, name, window_id, direction, status)`.

When `direction` is `Some`:
- Resize to 1/3 of monitor (height for Top/Bottom, width for Left/Right)
- Save old border width, set `border_width = 0`
- Set `is_locked = true`
- Store direction in `client.scratchpad_direction`

When `direction` is `None`: behave exactly as before (original size, normal border).

### `scratchpad_show_name`

When `scratchpad_direction` is `Some`:
- Compute initial rect (off-screen at the configured edge)
- Compute target rect (anchored at the edge with margins)
- Teleport to initial rect (`MoveResizeOptions::hinted_immediate`)
- Animate to target rect over `OVERLAY_ANIMATION_FRAMES` frames
- Positioning functions (`get_initial_overlay_rect`, `get_target_overlay_rect`) move from `overlay.rs`

When `scratchpad_direction` is `None`: behave exactly as before (instant show, no animation).

### `scratchpad_hide_name`

When `scratchpad_direction` is `Some`:
- Animate to off-screen rect (the `get_hide_animation_rect` logic from `overlay.rs`)
- Then hide as normal

When `scratchpad_direction` is `None`: behave exactly as before.

### `scratchpad_unmake`

When `scratchpad_direction` was `Some`:
- Restore `border_width` from `old_border_width`
- Set `is_locked = false`
- Clear `scratchpad_direction = None`

### `scratchpad_find`

Unchanged. Still looks up by name.

### `set_scratchpad_direction`

New function (replaces `set_overlay_mode`). Changes the direction on an existing edge-anchored scratchpad. If the scratchpad is currently visible, hides then re-shows to trigger the slide animation from the new edge. Also updates geometry (1/3 sizing based on new direction).

## Keybindings and Actions

### Super+Ctrl+W — `NamedAction::OverlayCreate`

Fixed name `"instantwm_overlay_scratch"`, direction defaults to `Top`.

- If `"instantwm_overlay_scratch"` already exists: unmake the existing one (restore to normal window), then create a new overlay scratchpad from the focused window.
- If it doesn't exist: create from focused window.

### Super+W — `NamedAction::OverlayToggle`

Calls `scratchpad_toggle(ctx, Some("instantwm_overlay_scratch"))`. Same guard as regular scratchpad toggle (skip in overview layout).

### Super+Ctrl+Arrow — Change overlay direction

When the overlay scratchpad is focused:
- Super+Ctrl+Up → `EdgeDirection::Top`
- Super+Ctrl+Down → `EdgeDirection::Bottom`
- Super+Ctrl+Left → `EdgeDirection::Left`
- Super+Ctrl+Right → `EdgeDirection::Right`

Calls `set_scratchpad_direction` on the overlay scratchpad.

### Super+S — Unchanged

Regular scratchpad create-or-toggle with name `"instantwm_scratchpad"`.

### Removed Actions

- `NamedAction::SetOverlay` and `NamedAction::CreateOverlay` are replaced by `OverlayCreate` and `OverlayToggle`.
- Button actions `HideOverlay`/`ShowOverlay` are folded into scratchpad operations.

## Files

### Delete

- `src/floating/overlay.rs`
- `src/constants/overlay.rs` — constants (margins, insets) move inline into `scratchpad.rs`

### Major Modifications

- `src/types/client.rs` — add `scratchpad_direction` field
- `src/types/input.rs` — rename `OverlayMode` to `EdgeDirection`
- `src/types/monitor.rs` — remove `overlaystatus`, `overlaymode`, `overlay` fields
- `src/floating/scratchpad.rs` — absorb animation/positioning logic, add direction-aware show/hide/make/unmake
- `src/floating/mod.rs` — remove overlay re-exports, add new scratchpad function exports
- `src/actions/named.rs` — replace overlay actions with `OverlayCreate`/`OverlayToggle`
- `src/config/keybindings.rs` — add Super+W, Super+Ctrl+W, Super+Ctrl+Arrow bindings
- `src/ipc/scratchpad.rs` — add direction parameter to create command dispatch
- `src/ipc_types.rs` — add direction to `ScratchpadCommand`/`ScratchpadInfo`

### Minor Updates

Remove overlay-specific guards, check `scratchpad_direction` instead:

- `src/layouts/manager.rs` — remove `place_overlay()`
- `src/layouts/algo/overview.rs` — check `scratchpad_direction` instead of overlay
- `src/keyboard.rs` — update for Super+Ctrl+Arrow overlay direction changes
- `src/tags/shift.rs` — remove overlay-to-set_overlay_mode redirection
- `src/client/visibility.rs` — update hide routing for edge-anchored scratchpads
- `src/mouse/drag/move_drop.rs` — block drag for `scratchpad_direction.is_some()` windows
- `src/floating/movement.rs` — block `center_window` for `scratchpad_direction.is_some()` windows
- `src/globals.rs` — update color scheme: `scratchpad_direction.is_some()` gets Overlay/OverlayFocus scheme
- `src/monitor.rs` — simplify scratchpad transfer (direction travels with the client already)
- `src/bin/ctl/commands.rs` — add direction option to scratchpad CLI create command

## What This Fixes

- Overlay state no longer duplicated across all monitors
- `overlaystatus: i32` replaced by scratchpad's existing sticky/hidden state
- `place_overlay()` layout conflict eliminated (no more overlay-specific layout pass)
- Overlay window destruction is handled by scratchpad cleanup (unmake on destroy)
- `reset_overlay` not clearing `mon.overlay` bug eliminated (no more `mon.overlay`)
