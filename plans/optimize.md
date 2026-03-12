# DRM event loop performance optimizations

Remaining items from the input-latency investigation. The critical bug
(double `libinput_context.dispatch()`) and the per-event mutex thrashing are
already fixed. These are the next-priority items.

## 1. Eliminate the mpsc channel hop (medium)

The libinput calloop source pushes events into an `mpsc::channel`, then the
main loop callback drains them. This adds one queue boundary and means events
are never processed until the *next* callback invocation after the source
fires.  Eliminating the channel and processing events directly in the calloop
source callback removes that boundary entirely.

**Blocker:** the source callback only receives `&mut WaylandState`, but input
dispatch also needs `&mut Wm` and `&Arc<Mutex<SharedDrmState>>`.  Fix by
storing those references (or the needed subset) inside `WaylandState` so the
callback has access.

## 2. Add a layout-dirty flag to avoid per-frame `arrange()` (medium)

`arrange_layout()` runs every frame (~60 Hz) even when nothing changed.
`arrange()` is not free — it iterates all clients, computes borders, runs the
layout algorithm, restacks, and flushes.  Add a `layout_dirty: bool` flag to
`Globals`, set it from the places that actually change layout state
(keybindings, IPC, client map/unmap, tag changes, monitor changes), and only
call `arrange()` when the flag is set.

## 3. Throttle `sync_space_from_globals()` (small)

Called every frame in `process_animations()`. It iterates every window in the
Smithay space, looks up the WM client, and potentially sends a
`send_pending_configure` to each.  Should only run when client geometries have
actually changed — can be gated behind the same layout-dirty flag or a
separate `space_dirty` flag.

## 4. Keyboard state: suppress release for intercepted press (small)

When a WM shortcut is intercepted on press, the release still forwards to
`keyboard_handle.input()` with `FilterResult::Forward`.  Smithay handles
this correctly internally (the key was never tracked as pressed for the
client), so this is not a bug, but it is unnecessary work.  Track intercepted
keycodes and return `FilterResult::Intercept(())` for their releases too.
