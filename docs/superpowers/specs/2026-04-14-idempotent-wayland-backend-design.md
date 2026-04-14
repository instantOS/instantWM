# Idempotent Wayland Backend Methods

## Problem

`WaylandState::unmap_window` bundles backend work (unmap from space, drop
animation, clear focus) with WM orchestration (queue layout, request space
sync). When called from `arrange() -> apply_visibility_wayland()`, the layout
queueing is redundant — arrange already handles layout scheduling for the
correct monitors. This creates a feedback pattern:

```
arrange()
  -> apply_visibility_wayland()
       -> WaylandState::unmap_window(win)
            -> queue_layout_for_client(win)   // redundant — arrange already scheduled layout
  -> next tick: process_pending_work()
       -> arrange() again (redundant pass)
```

The second arrange is a no-op (the window is already unmapped, the early-return
guard fires), but it still pays for the full arrange + visibility + restack
cycle on every monitor.

## Fix

Remove `queue_layout_for_client` from `WaylandState::unmap_window`. The layout
queueing is an orchestration concern that belongs in callers, not in the
backend method. All existing callers already handle layout scheduling
themselves:

- `apply_visibility_wayland` — called from `arrange()`, which already runs the
  layout and queues nothing extra.
- `hide()` — already calls `queue_layout_for_monitor_urgent` at
  `visibility.rs:243`.

`request_space_sync()` remains in `unmap_window` — it is idempotent (sets a
bool) and needed for render correctness between ticks.

## Changes

### `src/backend/wayland/compositor/window/lifecycle.rs`

**`unmap_window`**: Remove `g.queue_layout_for_client(window)` call.
Add `log::debug!` for no-op calls (already unmapped, window not found).

**`map_window`**: Add `log::debug!` for no-op calls (already mapped).

### `src/backend/wayland/compositor/state.rs`

**`request_render`**: Always set `render_dirty = true`. Skip the render ping
only when `render_dirty` was already true (avoid spurious event-loop wake).
Add `log::debug!` for redundant render requests.

**`request_space_sync`**: Add `log::debug!` for redundant sync requests
(already pending).

## Debug counters

All debug logging uses `log::debug!`, which is compiled out at warn/error
level. The counters log:

- `unmap_window({id}): no-op, already unmapped`
- `map_window({id}): no-op, already mapped`
- `request_render: ping skipped, already dirty`
- `request_space_sync: already pending`

These make redundant-operation patterns visible without runtime overhead in
production.

## What does NOT change

- `BackendOps` trait — no changes. The fix is entirely internal to WaylandState.
- `apply_visibility_wayland` — the only removed side effect (layout queueing)
  is handled by arrange.
- `hide()` — already queues layout itself.
- `remove_window_tracking` — doesn't go through `unmap_window`, uses
  `space.unmap_elem` directly.
- X11 backend — not touched.
