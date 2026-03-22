# Renderer Ownership Fix Plan

## Current State

The codebase is **broken** — it does not compile. There are 5 errors and 3 warnings,
all stemming from an incomplete migration between two renderer ownership models.
There are also two deeper design problems with unsafe code.

### Problem 1: `Rc<RefCell<GlesRenderer>>` vs `GlesRenderer` mismatch

`WaylandState.renderer` is declared as `Rc<RefCell<GlesRenderer>>` (state.rs:138),
but all call sites treat it as a plain `GlesRenderer`:

| File | Line | Issue |
|------|------|-------|
| `wayland/runtime/drm.rs` | 71 | Passes `Rc::new(RefCell::new(renderer))` — over-wraps a plain `GlesRenderer` |
| `wayland/runtime/drm.rs` | 79 | `state.renderer.borrow().dmabuf_formats()` causes E0502 (mutable+immutable borrow conflict on `state`) |
| `wayland/runtime/drm.rs` | 338-341 | `state.renderer.borrow_mut()` then pass `state` to `render_outputs` causes E0502 |
| `wayland/runtime/winit.rs` | 51 | Calls `.clone()` on `&mut GlesRenderer` returned by `backend.renderer()` — E0599 |
| `backend/wayland/compositor/handlers.rs` | 135 | Calls `.import_dmabuf()` on `Rc<RefCell<>>` — E0599 (method not found) |
| `wayland/common.rs` | 290 | References `data.state.display_handle` — E0609 (no field `state` on `WaylandState`) |

### Problem 2: `WaylandBackend` uses `NonNull` + unsafe

`WaylandBackend` (backend/wayland/mod.rs:73-75) stores a `RefCell<Option<NonNull<WaylandState>>>`
and dereferences it unsafely in `with_state()` (line 145). This exists because:

- `Wm` owns `Backend` (which contains `WaylandBackend`)
- `WaylandState` owns `Wm`
- WM logic needs to call back into `WaylandState` through the backend

This is a **self-referential struct** pattern worked around with raw pointers.

### Problem 3: `attach_state` uses unsafe pointer cast

Both runtime/drm.rs:72 and runtime/winit.rs:52 use:
```rust
if let WmBackend::Wayland(data) = unsafe { &mut *(&mut state.wm.backend as *mut _) } {
    data.backend.attach_state(&mut state);
}
```
This casts away the borrow checker to get `&mut` to a nested field while also
holding `&mut state`.

## Solution

### Phase 1: Fix compilation — Direct Renderer Ownership (no Rc, no RefCell)

The `Rc<RefCell<GlesRenderer>>` wrapper is unnecessary. The renderer has exactly
one owner and one user at any point in time:

- **DRM path**: The renderer is created in `init_gpu()`, stored in state, and then
  passed as `&mut GlesRenderer` to `build_output_surfaces()` and `render_drm_output()`.
  It is never shared — it is always used from the single-threaded calloop event
  loop via `&mut WaylandState`.

- **Winit path**: The renderer is obtained from `backend.bind()` which returns
  `(&mut GlesRenderer, Framebuffer)`. The state doesn't even need to store it —
  winit owns it.

- **Dmabuf handler**: Needs `&mut GlesRenderer` for `import_dmabuf()`.

### Phase 2 (separate patch): Remove NonNull/unsafe backend bridge

The command-queue redesign (Steps 4–5) should be done as a **separate patch**
after compilation is fixed. It is a larger architecture change with different
risks. See the Phase 2 section below.

### Step 1: Change `WaylandState.renderer` to `Option<GlesRenderer>`

**File: `src/backend/wayland/compositor/state.rs`**

Change the field (line 138):
```rust
// Before:
pub renderer: Rc<RefCell<GlesRenderer>>,

// After:
pub renderer: Option<GlesRenderer>,
```

Change the constructor signature (line 178):
```rust
// Before:
renderer: Rc<RefCell<GlesRenderer>>,

// After:
renderer: Option<GlesRenderer>,
```

The constructor body (line 270) already just assigns `renderer,` — no change needed.

Remove `use std::cell::RefCell;` (line 1) and `use std::rc::Rc;` (line 4) from
this file since they are no longer used.

Also add a helper method to `WaylandState` for safely extracting the renderer
while passing `&mut self` to functions that need both:

```rust
impl WaylandState {
    /// Temporarily extracts the renderer, calls `f` with both `&mut self` and
    /// `&mut GlesRenderer`, then puts the renderer back. Restores the renderer
    /// even if `f` panics, so we never leave `self.renderer` as `None`.
    pub fn with_renderer<R>(
        &mut self,
        f: impl FnOnce(&mut WaylandState, &mut GlesRenderer) -> R,
    ) -> R {
        let mut renderer = self.renderer.take().expect("renderer missing (not DRM?)");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f(self, &mut renderer)
        }));
        self.renderer = Some(renderer);
        match result {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}
```

### Step 2: Fix DRM runtime — use `with_renderer` for split borrows

**File: `src/wayland/runtime/drm.rs`**

There are three call sites to fix. The core problem is that `render_outputs`
takes both `state: &mut WaylandState` and `renderer: &mut GlesRenderer`, but
the renderer lives inside state. Rust forbids borrowing both simultaneously.
The `with_renderer` helper handles this cleanly by temporarily moving the
renderer out and restoring it afterward (even on panic).

**Site A — Constructor (line 71):**
```rust
// Before:
let mut state = WaylandState::new(display, &loop_handle, *wm, Rc::new(RefCell::new(renderer)));

// After:
let mut state = WaylandState::new(display, &loop_handle, *wm, Some(renderer));
```

**Site B — dmabuf_formats (lines 78-81):**

The problem is `state.init_dmabuf_global()` takes `&mut self` while
`state.renderer.borrow()` holds an immutable borrow of `state`. Fix by
extracting the formats first:
```rust
// Before:
state.init_dmabuf_global(
    state.renderer.borrow().dmabuf_formats().into_iter().collect(),
    Some(&egl_display),
);

// After:
let dmabuf_formats: Vec<_> = state.renderer.as_ref().unwrap().dmabuf_formats().into_iter().collect();
state.init_dmabuf_global(dmabuf_formats, Some(&egl_display));
```

**Site C — build_output_surfaces (lines 86-89):**
```rust
// Before:
let mut output_surfaces = {
    let mut renderer_ref = state.renderer.borrow_mut();
    build_output_surfaces(&mut drm_device, &mut *renderer_ref, &state, &gbm_device)
};

// After:
let mut output_surfaces = state.with_renderer(|state, renderer| {
    build_output_surfaces(&mut drm_device, renderer, state, &gbm_device)
});
```

Note: `state.renderer.as_mut().unwrap()` + `&state` would NOT compile because
Rust sees both a mutable and immutable borrow of `state` overlapping.
`with_renderer` avoids this by temporarily moving the renderer out.

**Site D — render_outputs in run_event_loop (lines 338-347):**
```rust
// Before:
let mut renderer_ref = state.renderer.borrow_mut();
render_outputs(
    state,
    &mut *renderer_ref,
    output_surfaces,
    cursor_manager,
    shared,
    render_failures,
    start_time,
);

// After:
state.with_renderer(|state, renderer| {
    render_outputs(
        state,
        renderer,
        output_surfaces,
        cursor_manager,
        shared,
        render_failures,
        start_time,
    );
});
```

Also remove the unused imports `use std::cell::RefCell;` (line 2) and
`use std::rc::Rc;` (line 6).

### Step 3: Fix winit runtime — pass `None`, delete dmabuf init

**File: `src/wayland/runtime/winit.rs`**

The winit backend's `WinitGraphicsBackend` owns the `GlesRenderer` — we cannot
move it out. The renderer is obtained each frame from `backend.bind()`.

**Change the constructor call (line 51):**
```rust
// Before:
let mut state = WaylandState::new(display, &loop_handle, *wm, backend.renderer().clone());

// After:
let mut state = WaylandState::new(display, &loop_handle, *wm, None);
```

**Keep the dmabuf init block (lines 58-61), but extract formats before creating state:**
```rust
// Before:
let mut state = WaylandState::new(display, &loop_handle, *wm, backend.renderer().clone());
// ...
state.init_dmabuf_global(
    backend.renderer().dmabuf_formats().into_iter().collect(),
    Some(backend.renderer().egl_context().display()),
);

// After:
let dmabuf_formats: Vec<_> = backend.renderer().dmabuf_formats().into_iter().collect();
let egl_display = backend.renderer().egl_context().display().clone();
let mut state = WaylandState::new(display, &loop_handle, *wm, None);
// ...
state.init_dmabuf_global(dmabuf_formats, Some(&egl_display));
```

Since `renderer` is `None` for winit, `dmabuf_imported` cannot import
immediately. Instead it queues the import (see Step 3b). The queued imports
are drained during `render_frame` where the winit backend's renderer is
available via `backend.bind()`. This preserves full dmabuf support for nested
clients with no unsafe code.

**Remove unused imports (lines 6, 8):**
```rust
// DELETE:
use std::cell::RefCell;
use std::rc::Rc;
```

### Step 3b: Fix dmabuf handler for `Option<GlesRenderer>` + deferred import queue

**File: `src/backend/wayland/compositor/state.rs`**

Add a field to `WaylandState` to hold pending dmabuf imports:
```rust
pub pending_dmabuf_imports: Vec<(
    smithay::backend::allocator::dmabuf::Dmabuf,
    smithay::wayland::dmabuf::ImportNotifier,
)>,
```
Initialize it as `Vec::new()` in the constructor.

**File: `src/backend/wayland/compositor/handlers.rs` (line 135)**

```rust
// Before:
let imported = self.renderer.import_dmabuf(&dmabuf, None).ok().is_some();

// After:
if let Some(renderer) = self.renderer.as_mut() {
    // DRM path: renderer is available, import immediately.
    let imported = renderer.import_dmabuf(&dmabuf, None).ok().is_some();
    if imported {
        let _ = notifier.successful::<Self>();
    } else {
        notifier.failed();
    }
} else {
    // Winit path: renderer is owned by the winit backend, not stored
    // in state. Queue the import — it will be processed during
    // render_frame when the renderer is available via backend.bind().
    self.pending_dmabuf_imports.push((dmabuf, notifier));
}
return; // early return replaces the existing if/else below
```

**File: `src/wayland/render/winit.rs` — drain queue during render**

In `render_frame`, after `backend.bind()` returns the renderer, drain
the pending imports:

```rust
let (renderer, mut framebuffer) = backend.bind().expect("renderer bind");

// Process any deferred dmabuf imports from the handler
for (dmabuf, notifier) in state.pending_dmabuf_imports.drain(..) {
    if renderer.import_dmabuf(&dmabuf, None).is_ok() {
        let _ = notifier.successful::<WaylandState>();
    } else {
        notifier.failed();
    }
}
```

This preserves full dmabuf support for nested winit clients. The only
tradeoff is imports are deferred by at most one frame (16ms at 60Hz).

### Phase 2: Remove NonNull/unsafe backend bridge

**This phase should be done as a separate patch after Phase 1 compiles and is
verified.** The command-queue pattern is a larger architecture change.

### Step 4a: Replace `WaylandBackend` NonNull with command queue (fire-and-forget ops)

**File: `src/backend/wayland/mod.rs`**

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

**Fix**: Replace the `NonNull` back-pointer with a **command queue** for all
fire-and-forget operations. `WaylandBackend` collects pending operations into
a `Vec<WmCommand>`, which are flushed after WM logic returns.

Replace the entire `WaylandBackend` struct and its impls with:

```rust
use std::cell::RefCell;
use crate::backend::BackendOps;
use crate::types::{Rect, WindowId};
use crate::backend::wayland::compositor::WaylandState;

/// Commands queued by WM logic, executed on WaylandState after WM returns.
pub enum WmCommand {
    ResizeWindow(WindowId, Rect),
    RaiseWindow(WindowId),
    Restack(Vec<WindowId>),
    SetFocus(WindowId),
    MapWindow(WindowId),
    UnmapWindow(WindowId),
    Flush,
    WarpPointer(f64, f64),
    SetKeyboardLayout {
        layout: String,
        variant: String,
        options: Option<String>,
        model: Option<String>,
    },
    SetMonitorConfig {
        name: String,
        config: crate::config::config_toml::MonitorConfig,
    },
    CloseWindow(WindowId),
    ClearKeyboardFocus,
    SetCursorIconOverride(Option<smithay::input::pointer::CursorIcon>),
}

pub struct WaylandBackend {
    pending_ops: RefCell<Vec<WmCommand>>,
    // Cached query results, updated during flush
    cached_pointer_location: RefCell<Option<(i32, i32)>>,
    cached_xdisplay: RefCell<Option<u32>>,
}

impl WaylandBackend {
    pub fn new() -> Self {
        Self {
            pending_ops: RefCell::new(Vec::new()),
            cached_pointer_location: RefCell::new(None),
            cached_xdisplay: RefCell::new(None),
        }
    }

    /// Drain all pending commands. Called from the event loop after WM logic.
    pub fn drain_ops(&self) -> Vec<WmCommand> {
        std::mem::take(&mut *self.pending_ops.borrow_mut())
    }

    /// Update cached values from WaylandState. Called from event loop.
    pub fn sync_cache(&self, state: &WaylandState) {
        let loc = state.pointer.current_location();
        *self.cached_pointer_location.borrow_mut() =
            Some((loc.x.round() as i32, loc.y.round() as i32));
        *self.cached_xdisplay.borrow_mut() = state.xdisplay;
    }

    // -- Wayland-specific query methods (use cache, no unsafe) --

    pub fn pointer_location(&self) -> Option<(i32, i32)> {
        *self.cached_pointer_location.borrow()
    }

    pub fn xdisplay(&self) -> Option<u32> {
        *self.cached_xdisplay.borrow()
    }

    pub fn close_window(&self, window: WindowId) -> bool {
        self.pending_ops.borrow_mut().push(WmCommand::CloseWindow(window));
        true // optimistic — actual close happens during flush
    }

    pub fn clear_keyboard_focus(&self) {
        self.pending_ops.borrow_mut().push(WmCommand::ClearKeyboardFocus);
    }

    pub fn set_cursor_icon_override(&self, icon: Option<smithay::input::pointer::CursorIcon>) {
        self.pending_ops.borrow_mut().push(WmCommand::SetCursorIconOverride(icon));
    }

    pub fn warp_pointer(&self, x: f64, y: f64) {
        self.pending_ops.borrow_mut().push(WmCommand::WarpPointer(x, y));
    }
}

impl Default for WaylandBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendOps for WaylandBackend {
    fn resize_window(&self, window: WindowId, rect: Rect) {
        self.pending_ops.borrow_mut().push(WmCommand::ResizeWindow(window, rect));
    }

    fn raise_window(&self, window: WindowId) {
        self.pending_ops.borrow_mut().push(WmCommand::RaiseWindow(window));
    }

    fn restack(&self, windows: &[WindowId]) {
        self.pending_ops.borrow_mut().push(WmCommand::Restack(windows.to_vec()));
    }

    fn set_focus(&self, window: WindowId) {
        self.pending_ops.borrow_mut().push(WmCommand::SetFocus(window));
    }

    fn map_window(&self, window: WindowId) {
        self.pending_ops.borrow_mut().push(WmCommand::MapWindow(window));
    }

    fn unmap_window(&self, window: WindowId) {
        self.pending_ops.borrow_mut().push(WmCommand::UnmapWindow(window));
    }

    fn flush(&self) {
        self.pending_ops.borrow_mut().push(WmCommand::Flush);
    }

    fn warp_pointer(&self, x: f64, y: f64) {
        self.pending_ops.borrow_mut().push(WmCommand::WarpPointer(x, y));
    }

    fn set_keyboard_layout(
        &self,
        layout: &str,
        variant: &str,
        options: Option<&str>,
        model: Option<&str>,
    ) {
        self.pending_ops.borrow_mut().push(WmCommand::SetKeyboardLayout {
            layout: layout.to_owned(),
            variant: variant.to_owned(),
            options: options.map(|s| s.to_owned()),
            model: model.map(|s| s.to_owned()),
        });
    }

    fn set_monitor_config(&self, name: &str, config: &crate::config::config_toml::MonitorConfig) {
        // ⚠️ Ordering concern: if WM calls set_monitor_config() then immediately
        // calls update_geom(), the config won't be applied yet (it's queued).
        // Verify that callers don't depend on synchronous config application.
        self.pending_ops.borrow_mut().push(WmCommand::SetMonitorConfig {
            name: name.to_owned(),
            config: config.clone(),
        });
    }

    // -- Query methods return cached/default values (no unsafe) --

    fn window_exists(&self, _window: WindowId) -> bool {
        // Cannot query WaylandState here. WM already tracks existence in its
        // own client list, so this always returns true. If the window was
        // destroyed, the WM will learn about it from the destroy event.
        true
    }

    fn pointer_location(&self) -> Option<(i32, i32)> {
        *self.cached_pointer_location.borrow()
    }

    fn window_title(&self, _window: WindowId) -> Option<String> {
        // Titles are already tracked in WM's client list on the Wayland path.
        // The Wayland compositor pushes title updates via manage/configure events.
        None
    }

    fn get_outputs(&self) -> Vec<crate::backend::BackendOutputInfo> {
        // Return cached output data. Updated by sync_cache() each tick.
        // Must NOT return empty — monitor::update_geom() depends on this.
        self.cached_outputs.borrow().clone()
    }

    fn get_input_devices(&self) -> Vec<String> {
        // Return cached input device list. Updated by sync_cache() each tick.
        self.cached_input_devices.borrow().clone()
    }
}
```

### Step 4b: Query methods that need WaylandState access

The following methods currently use `with_state()` to query `WaylandState`
and **cannot work with a command queue** because they return values.

**⚠️ Important:** Do NOT return fake/placeholder values for query methods.
Returning `Vec::new()` for `get_outputs()` would break `monitor::update_geom()`
which calls `ctx.backend().get_outputs()` to compute monitor geometry. All
query methods must use **truthful caches** updated at the source of truth
(i.e., when the actual data changes, not just once per tick).

| Method | Callers | Strategy |
|--------|---------|----------|
| `pointer_location()` | `mouse/drag`, `BackendOps` | **Cache** in `WaylandBackend`. Updated by `sync_cache()` each event-loop tick. |
| `xdisplay()` | `ipc/general.rs`, `util.rs` | **Cache** in `WaylandBackend`. Updated by `sync_cache()` each event-loop tick. |
| `window_exists()` | `BackendOps` | **Always return `true`**. The WM already tracks window existence via its client list. Destruction events remove windows reactively. |
| `window_title()` | `BackendOps` | **Return `None`**. Wayland titles are pushed to WM via compositor events, not pulled. |
| `close_window()` | `client/kill.rs` | **Enqueue as command, return `true` optimistically**. Actual close happens during flush. |
| `is_keyboard_focused_on()` | `focus.rs:170` | **Cache** keyboard focus window ID. Updated by `sync_cache()`. |
| `list_displays()` | `ipc/monitor.rs` | See Step 4c. |
| `list_display_modes()` | `ipc/monitor.rs` | See Step 4c. |
| `get_outputs()` | `BackendOps`, `monitor.rs` | **Cache** in `WaylandBackend`. Updated via `sync_cache()`. Must contain real output data — returning empty breaks monitor geometry. |
| `get_input_devices()` | `ipc/input.rs` | See Step 4c. |

### Step 4c: IPC query methods — move to event-loop dispatch

**Complexity: Medium** (requires plumbing changes to IPC dispatch)

`list_displays()`, `list_display_modes()`, and `get_input_devices()` are only
called from IPC handlers. These IPC handlers already run inside the event loop
where `&mut WaylandState` is available. Instead of routing through
`WaylandBackend`, query `WaylandState` directly.

**Challenge:** The current IPC layer signature is:
```rust
IpcServer::process_pending(&mut self, wm: &mut Wm)
```
IPC handlers like `ipc/monitor.rs` only receive `&mut Wm`, not
`&mut WaylandState`. So this step requires either:

1. **Passing `&mut WaylandState`** (or a trait object) into the IPC dispatch path, or
2. **Using the cached values from Step 4b** — if `get_outputs()` and
   `get_input_devices()` return truthful cached data, the IPC handlers can
   continue going through `BackendOps` and the caches will be correct.

Option 2 is simpler and avoids an architecture refactor. `list_displays()` and
`list_display_modes()` would also need to be cached in `WaylandBackend` if
they are called through `BackendOps`.

Similarly for `get_input_devices()` in `src/ipc/input.rs`.

### Step 4d: Add `sync_cache()` and `drain_ops()` calls to event loops

**File: `src/wayland/runtime/drm.rs`**

In `run_event_loop`, inside the `event_loop.run()` closure, add after all
WM logic runs and before rendering.

**⚠️ Borrow-checker note:** You cannot immutably borrow `state.wm.backend`
(to call `drain_ops()`) while also mutably borrowing `state` (to call
`execute_command()`). Split into two phases:

```rust
// Phase 1: drain the command queue (immutable borrow of backend)
let ops: Vec<WmCommand> = if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
    data.backend.drain_ops()
} else {
    Vec::new()
};

// Phase 2: execute commands (mutable borrow of state)
for op in ops {
    state.execute_command(op);
}

// Phase 3: update caches (immutable borrow of backend + shared ref to state)
if let crate::backend::Backend::Wayland(data) = &state.wm.backend {
    data.backend.sync_cache(state);
}
```

Note: `sync_cache` takes `&WaylandState` (immutable) to read current values
into the backend's caches. If it needs `&mut WaylandState`, the same
two-phase pattern applies.

**File: `src/wayland/runtime/winit.rs`**

Same pattern as above.

### Step 4e: Implement `execute_command` on `WaylandState`

**File: `src/backend/wayland/compositor/state.rs`**

Add a method that dispatches each `WmCommand` to the existing methods:

```rust
impl WaylandState {
    pub fn execute_command(&mut self, cmd: WmCommand) {
        use crate::backend::wayland::WmCommand;
        match cmd {
            WmCommand::ResizeWindow(id, rect) => self.resize_window(id, rect),
            WmCommand::RaiseWindow(id) => self.raise_window(id),
            WmCommand::Restack(ids) => self.restack(&ids),
            WmCommand::SetFocus(id) => self.set_focus(id),
            WmCommand::MapWindow(id) => self.map_window(id),
            WmCommand::UnmapWindow(id) => self.unmap_window(id),
            WmCommand::Flush => self.flush(),
            WmCommand::WarpPointer(x, y) => self.request_warp(x, y),
            WmCommand::CloseWindow(id) => { self.close_window(id); },
            WmCommand::ClearKeyboardFocus => self.clear_seat_focus(),
            WmCommand::SetCursorIconOverride(icon) => self.cursor_icon_override = icon,
            WmCommand::SetKeyboardLayout { layout, variant, options, model } => {
                self.set_keyboard_layout(&layout, &variant, options.as_deref(), model.as_deref());
            },
            WmCommand::SetMonitorConfig { name, config } => {
                self.set_output_config(&name, &config);
            },
        }
    }
}
```

### Step 5: Remove `attach_state` unsafe pointer casts

**File: `src/wayland/runtime/drm.rs` (lines 72-74):**
```rust
// DELETE these lines entirely:
if let WmBackend::Wayland(data) = unsafe { &mut *(&mut state.wm.backend as *mut _) } {
    data.backend.attach_state(&mut state);
}
```

**File: `src/wayland/runtime/winit.rs` (lines 52-54):**
```rust
// DELETE these lines entirely:
if let WmBackend::Wayland(data) = unsafe { &mut *(&mut state.wm.backend as *mut _) } {
    data.backend.attach_state(&mut state);
}
```

With the command-queue approach from Step 4, `attach_state` is eliminated
entirely. There's no back-pointer to set up.

### Step 6: Fix `wayland/common.rs:290` — `data.state` field doesn't exist

**File: `src/wayland/common.rs` (line 289-292)**

```rust
// Before:
let _ = data
    .state
    .display_handle
    .insert_client(client, Arc::new(WaylandClientState::default()));

// After:
let _ = data
    .display_handle
    .insert_client(client, Arc::new(WaylandClientState::default()));
```

### Step 7: Remove unused imports

- `src/wayland/runtime/drm.rs`: Remove `use std::cell::RefCell;` (line 2)
  and `use std::rc::Rc;` (line 6).
- `src/wayland/runtime/winit.rs`: Remove `use std::cell::RefCell;` (line 6)
  and `use std::rc::Rc;` (line 8).
  Keep `use smithay::backend::renderer::ImportDma;` — still used for dmabuf init.
- `src/backend/wayland/compositor/state.rs`: Remove `use std::cell::RefCell;`
  (line 1) and `use std::rc::Rc;` (line 4).
- `src/backend/wayland/mod.rs`: Remove `use std::ptr::NonNull;` (line 69).
  Keep `use std::cell::RefCell;` (line 68) — still used by `pending_ops`.

## Summary of Changes

| # | What | Files | Complexity |
|---|------|-------|-----------|
| 1 | `renderer: Rc<RefCell<GlesRenderer>>` → `renderer: Option<GlesRenderer>` | state.rs | Low |
| 2 | Fix DRM runtime — 4 call sites: constructor, dmabuf_formats, build_output_surfaces, render_outputs | runtime/drm.rs | Low |
| 3 | Fix winit runtime — pass `None`, keep dmabuf init (extract formats first) | runtime/winit.rs | Low |
| 3b | Deferred dmabuf import queue for winit path + drain in render_frame | handlers.rs, state.rs, render/winit.rs | Low |
| 4a | Replace `WaylandBackend` NonNull with command queue + cached queries | backend/wayland/mod.rs | Medium |
| 4b | Document query method strategies (cache / optimistic / delegate) | — | N/A (reference) |
| 4c | Move IPC-only queries to event-loop dispatch path | ipc/monitor.rs, ipc/input.rs | Low |
| 4d | Add `drain_ops()` + `sync_cache()` to both event loops | runtime/drm.rs, runtime/winit.rs | Low |
| 4e | Implement `execute_command` on `WaylandState` | state.rs | Low |
| 5 | Delete `attach_state` unsafe casts | runtime/drm.rs, runtime/winit.rs | Trivial (delete) |
| 6 | Fix `data.state.display_handle` → `data.display_handle` | wayland/common.rs | Trivial |
| 7 | Clean up dead imports | Multiple | Trivial |

## Execution Order

### Phase 1: Fix compilation (single patch)

1. **Step 6** first (trivial independent bug fix)
2. **Steps 1 + 2 + 3 + 3b** together (renderer type change + all call sites)
3. **Step 7** (cleanup imports for Phase 1 files)
4. Run `cargo check` — should compile with only `NonNull`-related warnings remaining

### Phase 2: Remove unsafe backend bridge (separate patch)

5. **Steps 4a + 4b + 4d + 4e** together (backend bridge redesign)
6. **Step 5** (delete attach_state calls)
7. **Step 4c** (IPC query migration — medium complexity, may use cached values instead)
8. **Step 7** remainder (cleanup imports for Phase 2 files)

Phase 1 fixes compilation. Phase 2 removes all unsafe code from the backend
bridge. Keep them as separate patches — Phase 2 is a larger architecture
change that should be reviewed independently.

## Verification

After each execution group, run `cargo check` to verify compilation.
After all steps, run `cargo test` if tests exist. The key invariant to
verify is that the DRM event loop correctly takes/returns the renderer
around `render_outputs` — a missing `state.renderer = Some(renderer)`
would cause a panic on the next frame.

## Design Principles

- **No `Rc<RefCell<_>>`**: The renderer has exactly one owner at all times.
  `Option<GlesRenderer>` with temporary extraction is sufficient.
- **No `unsafe`** (Phase 2): The command-queue pattern eliminates the need for
  raw pointers in the backend bridge. Operations are deferred and executed safely.
- **Panic safety**: The `with_renderer` helper restores `self.renderer = Some(...)`
  even on panic, so the invariant (always `Some` outside render calls) is
  maintained. Without the helper, a panic between `take()` and reassignment
  would leave the renderer as `None`.
- **Minimal overhead**: `Option<GlesRenderer>` adds a discriminant byte.
  The command queue adds a `Vec::push` per operation — acceptable for the
  safety gained over `NonNull` + unsafe.
- **Truthful caches**: Query methods in the command-queue backend must return
  real data (especially `get_outputs()`), not placeholder empty values.
