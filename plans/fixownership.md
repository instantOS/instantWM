# Renderer Ownership Fix Plan

## Current State

The codebase is **broken** — it does not compile. There are 7 errors and 2 warnings,
all stemming from an incomplete migration between two renderer ownership models.
There are also two deeper design problems with unsafe code.

### Problem 1: `Rc<RefCell<GlesRenderer>>` vs `GlesRenderer` mismatch

`WaylandState.renderer` is declared as `Rc<RefCell<GlesRenderer>>` (state.rs:138),
but all call sites treat it as a plain `GlesRenderer`:

| File | Line | Issue |
|------|------|-------|
| `wayland/runtime/drm.rs` | 69 | Passes plain `GlesRenderer` to `WaylandState::new()` which expects `Rc<RefCell<>>` |
| `wayland/runtime/drm.rs` | 77 | Calls `.dmabuf_formats()` on `Rc<RefCell<>>` (method not found) |
| `wayland/runtime/drm.rs` | 86 | Passes `&mut Rc<RefCell<GlesRenderer>>` where `&mut GlesRenderer` expected |
| `wayland/runtime/drm.rs` | 339 | Same as above, in `render_outputs` |
| `wayland/runtime/winit.rs` | 51 | Calls `.clone()` on `&mut GlesRenderer` returned by `backend.renderer()` |
| `backend/wayland/compositor/handlers.rs` | 135 | Calls `.import_dmabuf()` on `Rc<RefCell<>>` |
| `wayland/common.rs` | 290 | References `data.state.display_handle` — no field `state` on `WaylandState` |

### Problem 2: `WaylandBackend` uses `NonNull` + unsafe

`WaylandBackend` (backend/wayland/mod.rs:73-75) stores a `RefCell<Option<NonNull<WaylandState>>>`
and dereferences it unsafely in `with_state()` (line 145). This exists because:

- `Wm` owns `Backend` (which contains `WaylandBackend`)
- `WaylandState` owns `Wm`
- WM logic needs to call back into `WaylandState` through the backend

This is a **self-referential struct** pattern worked around with raw pointers.

### Problem 3: `attach_state` uses unsafe pointer cast

Both runtime/drm.rs:70 and runtime/winit.rs:52 use:
```rust
if let WmBackend::Wayland(data) = unsafe { &mut *(&mut state.wm.backend as *mut _) } {
    data.backend.attach_state(&mut state);
}
```
This casts away the borrow checker to get `&mut` to a nested field while also
holding `&mut state`.

## Solution: Direct Ownership — No Rc, No RefCell, No NonNull

The core insight is that the `Rc<RefCell<GlesRenderer>>` wrapper is **completely
unnecessary**. Looking at how the renderer is actually used:

- **DRM path**: The renderer is created in `init_gpu()`, stored in state, and then
  passed as `&mut GlesRenderer` to `build_output_surfaces()` and `render_drm_output()`.
  It is never shared — it is always used from the single-threaded calloop event
  loop via `&mut WaylandState`.

- **Winit path**: The renderer is obtained from `backend.bind()` which returns
  `(&mut GlesRenderer, Framebuffer)`. The state doesn't even need to store it —
  winit owns it.

- **Dmabuf handler**: Needs `&mut GlesRenderer` for `import_dmabuf()`.

**There is exactly one owner and one user at any point in time.** The Rc<RefCell>
wrapping adds runtime overhead for zero benefit.

### Step 1: Change `WaylandState.renderer` to `Option<GlesRenderer>`

**File: `src/backend/wayland/compositor/state.rs`**

```rust
// Before:
pub renderer: Rc<RefCell<GlesRenderer>>,

// After:
pub renderer: Option<GlesRenderer>,
```

Update the constructor signature to `renderer: Option<GlesRenderer>`.

Remove unused imports `RefCell` and `Rc` from this file.

### Step 2: Fix DRM runtime — use `.take()`/reassign for split borrows

**File: `src/wayland/runtime/drm.rs`**

`render_outputs` takes both `state: &mut WaylandState` and
`renderer: &mut GlesRenderer`, but the renderer lives inside state. Rust
forbids borrowing both simultaneously. Since `GlesRenderer` doesn't implement
`Default`, `std::mem::take` won't work — but `Option::take()` does:

```rust
let mut renderer = state.renderer.take().unwrap();
render_outputs(state, &mut renderer, ...);
state.renderer = Some(renderer);
```

Other call sites (lines 77, 85-86) use `state.renderer.as_mut().unwrap()`
for non-conflicting borrows like `.dmabuf_formats()` and `build_output_surfaces()`.

### Step 3: Fix winit runtime — pass `None`, skip dmabuf global

**File: `src/wayland/runtime/winit.rs`**

The winit backend's `WinitGraphicsBackend` owns the `GlesRenderer` — we cannot
move it out. The renderer is obtained each frame from `backend.bind()`.

Pass `None` to the constructor:
```rust
let mut state = WaylandState::new(display, &loop_handle, *wm, None);
```

Skip `init_dmabuf_global` on the winit path entirely. Winit's host compositor
already handles dmabuf for nested clients. The `DmabufHandler::dmabuf_imported`
impl checks `self.renderer.is_some()` before importing; when `None`, it reports
failure via the notifier (clients fall back to shm).

Remove unused `use std::cell::RefCell` and `use std::rc::Rc`.

### Step 4: Fix the `WaylandBackend` NonNull problem

The `WaylandBackend` exists solely so that `Wm` (which is owned by `WaylandState`)
can call back into `WaylandState`. It currently uses `NonNull` + unsafe.

**The circular call path**:
```
WaylandState (event loop)
  → state.wm.ctx()           (creates WmCtxWayland with &WaylandBackend)
    → backend.resize_window() (BackendOps impl)
      → with_state(|state| state.resize_window())  (unsafe NonNull deref!)
        → WaylandState.resize_window()  (back to start!)
```

**Fix**: Replace the `NonNull` back-pointer with a **command queue**.
`WaylandBackend` becomes a stub that collects pending operations into a
`Vec<WmCommand>`, which are flushed after WM logic returns:

```rust
pub enum WmCommand {
    ResizeWindow(WindowId, Rect),
    RaiseWindow(WindowId),
    Restack(Vec<WindowId>),
    SetFocus(WindowId),
    MapWindow(WindowId),
    UnmapWindow(WindowId),
    Flush,
    WarpPointer(f64, f64),
}

pub struct WaylandBackend {
    pending_ops: RefCell<Vec<WmCommand>>,
}

impl BackendOps for WaylandBackend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        self.pending_ops.borrow_mut().push(WmCommand::ResizeWindow(window, rect));
    }
    // ... etc for each BackendOps method
}
```

Then in the event loop, after WM logic runs:
```rust
let ops = state.wm.backend.wayland_data().unwrap().backend.drain_ops();
for op in ops {
    state.execute_command(op);
}
```

This is safe (just a `Vec::push` per operation), removes all `NonNull`/`unsafe`
from the backend bridge, and batches operations which avoids re-entrant borrows.

### Step 5: Fix `attach_state` unsafe pointer casts

**Files: `wayland/runtime/drm.rs:70`, `wayland/runtime/winit.rs:52`**

With the command-queue approach from Step 4, `attach_state` is eliminated
entirely. There's no back-pointer to set up.

### Step 6: Fix `wayland/common.rs:290` — `data.state` field doesn't exist

This line references `data.state.display_handle` but `WaylandState` has no
`state` field. This should be `data.display_handle`:

```rust
// Before:
let _ = data.state.display_handle.insert_client(...);

// After:
let _ = data.display_handle.insert_client(...);
```

### Step 7: Remove unused imports

- `src/wayland/runtime/winit.rs`: Remove `use std::cell::RefCell` (line 6)
  and `use std::rc::Rc` (line 8).
- `src/backend/wayland/compositor/state.rs`: Remove `use std::cell::RefCell`
  and `use std::rc::Rc` if no longer needed.
- `src/backend/wayland/mod.rs`: Remove `use std::ptr::NonNull` and
  `use std::cell::RefCell`.

## Summary of Changes

| # | What | Files | Complexity |
|---|------|-------|-----------|
| 1 | `renderer: Rc<RefCell<GlesRenderer>>` → `renderer: Option<GlesRenderer>` | state.rs | Low |
| 2 | Fix DRM runtime — pass renderer directly, use `.take()`/reassign for split borrows | runtime/drm.rs | Low |
| 3 | Fix winit runtime — don't store renderer (winit owns it), pass `None` | runtime/winit.rs | Low |
| 4 | Replace `WaylandBackend` NonNull back-pointer with command queue | backend/wayland/mod.rs, runtime/*.rs | Medium |
| 5 | Remove `attach_state` unsafe casts | runtime/drm.rs, runtime/winit.rs | Low (deleted) |
| 6 | Fix `data.state.display_handle` → `data.display_handle` | wayland/common.rs | Trivial |
| 7 | Clean up dead imports | Multiple | Trivial |

## Execution Order

1. Step 6 first (trivial bug fix, independent)
2. Step 1 + 2 + 3 together (renderer type change + all call sites)
3. Step 4 + 5 together (backend bridge redesign)
4. Step 7 last (cleanup)

Steps 1-3 fix compilation. Step 4-5 remove all unsafe code from the backend
bridge. The entire change eliminates `Rc`, `RefCell`, `NonNull`, and `unsafe`
from the renderer/backend ownership path.

## Design Principles

- **No `Rc<RefCell<_>>`**: The renderer has exactly one owner at all times.
  `Option<GlesRenderer>` with temporary extraction is sufficient.
- **No `unsafe`**: The command-queue pattern eliminates the need for raw
  pointers in the backend bridge. Operations are deferred and executed safely.
- **No runtime panics**: `Option::take()` + reassign is infallible when
  the invariant (always `Some` outside render calls) is maintained.
- **Zero overhead**: `Option<GlesRenderer>` has the same size as `GlesRenderer`
  if it's not zero-sized (it isn't). The command queue is a `Vec::push` per
  operation, cheaper than the current `NonNull` deref + RefCell borrow.
