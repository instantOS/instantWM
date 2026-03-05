# Context Split Migration Plan

Goal: Make backend-specific code depend only on the minimal context it needs. Keep all shared WM state (tags, layouts, monitors, clients, config) backend-agnostic in `Globals`, and remove scattered runtime `X11`/`Wayland` checks.

This plan introduces a small `CoreCtx` plus backend-specific context types, then migrates modules to depend on only the smallest necessary context. Breaking changes are expected.

---

## 1) Introduce core + backend context types (done)

Create new context types in `src/contexts.rs` (or `src/contexts/core.rs` if you prefer submodules):

- `CoreCtx<'a>`
  - `g: &'a mut Globals`
  - `running: &'a mut bool`
  - `bar: &'a mut BarState`
  - `bar_painter: &'a mut crate::bar::wayland::WaylandBarPainter`
  - `focus: &'a mut FocusState`

- `X11Ctx<'a>`
  - `conn: &'a RustConnection`
  - `screen_num: usize`
  - `root: Window`
  - `atoms: &'a X11RuntimeConfig` (or pass just the fields you need)
  - `drw: Option<&'a mut Drw>` (only where required)

- `WaylandCtx<'a>`
  - `backend: &'a WaylandBackend`
  - `state: Option<&'a mut WaylandState>` if needed outside backend methods

- `XwaylandCtx<'a>` (optional, only if you want XWayland bridging under Wayland)
  - `xdisplay: u32`
  - `xwm: &'a X11Wm` (or whatever is needed)

- `WmCtx<'a>` enum
  - `X11(WmCtxX11<'a>)`
  - `Wayland(WmCtxWayland<'a>)`

- `WmCtxX11<'a>`
  - `core: CoreCtx<'a>`
  - `x11: X11Ctx<'a>`

- `WmCtxWayland<'a>`
  - `core: CoreCtx<'a>`
  - `wayland: WaylandCtx<'a>`
  - `xwayland: Option<XwaylandCtx<'a>>`

Key rule: modules should accept the smallest of `CoreCtx`, `X11Ctx`, `WaylandCtx`, or a combo tuple if needed. Avoid passing `WmCtx` unless the module must branch on backend.

---

## 2) Replace global `backend_kind()` and `x11_conn()` checks

- Remove `WmCtx::backend_kind()` and `WmCtx::x11_conn()`.
- Delete `require_x11!` and `require_x11_ret!` macros in `src/macros.rs`.
- Replace all runtime checks with compile-time context selection.

---

## 3) Create small helper conversions at the boundary (done)

In `Wm::ctx()` (or `Wm::ctx_x11()` / `Wm::ctx_wayland()`):

- Build `CoreCtx` from `Globals` + shared WM state.
- Match on backend to build either `WmCtx::X11` or `WmCtx::Wayland`.
- Keep backend ops (resize/map/etc.) accessible via `BackendOps` or directly on `WaylandCtx`.

This is the only location that needs to branch on backend kind.

---

## 4) Migrate modules by dependency level

### 4.1 Core-only modules
Change function signatures to accept only `&mut CoreCtx` (or `&CoreCtx`) if they only touch shared state:

- tags/* (view, shift, sticky, naming, etc.)
- layouts/*
- monitor manager logic that only reads/writes `Globals`
- client bookkeeping that doesn’t touch X11 properties

### 4.2 Generic window operations
Modules that need to call `map_window`, `resize_window`, etc. should take:

- `&mut CoreCtx` plus `&dyn BackendOps` (or `impl BackendOps`)

Examples:
- general focus/move/resize that is backend-agnostic
- arrange/restack path when it’s only about window geometry

### 4.3 X11-only modules
Change signatures to accept `&X11Ctx` or `&mut X11Ctx`, not `WmCtx`:

- `client/state.rs` (X11 properties, EWMH)
- `systray.rs`
- X11-specific mouse grabs and keyboard grabbing
- `events.rs` (X11 event handlers)
- `xresources.rs`, X11 atoms helpers, etc.

Where these modules also need `Globals`, accept `(&mut CoreCtx, &mut X11Ctx)` explicitly.

### 4.4 Wayland-only modules
Change signatures to accept `&WaylandCtx` or `&mut WaylandCtx`:

- Wayland input handling
- compositor state transitions
- Wayland bar rendering path

### 4.5 XWayland-optional modules
If you want XWayland support under Wayland, accept `Option<&XwaylandCtx>` only in the small set of functions that actually need it. Do not pass it around broadly.

---

## 5) Update call sites to use context matching once

At call boundaries (main event loops / startup), use:

```rust
match wm.ctx() {
    WmCtx::X11(mut ctx) => x11::handle_event(&mut ctx.core, &mut ctx.x11, event),
    WmCtx::Wayland(mut ctx) => wayland::handle_event(&mut ctx.core, &mut ctx.wayland, event),
}
```

From there, only pass the minimal context into deeper modules.

---

## 6) Specific high-impact migrations (first pass)

1. `src/contexts.rs`
   - Introduce `CoreCtx`, `X11Ctx`, `WaylandCtx`, `WmCtx` enum, and `WmCtxX11/Wayland`.
   - Remove old `x11_conn()` and `backend_kind()`.

2. `src/macros.rs`
   - Delete `require_x11!` and `require_x11_ret!`.

3. `src/util.rs`
   - `spawn()` should accept `&CoreCtx` plus optional `WaylandCtx` for XWayland DISPLAY handling.
   - Avoid reading backend kind inside `spawn()`; the caller passes what it needs.

4. `src/client/state.rs`
   - Split into two public entry points:
     - `update_title_x11(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId)`
     - `update_title_wayland(core: &mut CoreCtx, wayland: &WaylandCtx, win: WindowId)`
   - All property access uses `X11Ctx` only.

5. `src/systray.rs`
   - Convert to `(&mut CoreCtx, &mut X11Ctx)` and remove `x11_conn()` checks.
   - Gate entirely at call boundary in Wayland.

6. `src/events.rs`
   - X11 event loop path should only ever call X11 handlers with `X11Ctx`.
   - Wayland path uses Wayland input handlers.

---

## 7) Remove dead helpers and simplify Globals

- If `Globals::x11` is only needed in X11 modules, avoid exposing it in `CoreCtx`.
- Expose only specific X11 fields through `X11Ctx` instead of passing `Globals` directly.

---

## 8) Compile/test cycle

- Run `cargo check` after each migration step.
- Expect many signature mismatches; let the compiler guide the final wiring.
- Only add backend checks at call boundaries; never inside deep modules.

---

## End State (acceptance)

- No `backend_kind()` checks in core modules.
- No `ctx.x11_conn()` `Option` usage outside X11 boundary code.
- X11-only modules compile only with `X11Ctx` inputs.
- Shared state (tags/layouts/monitors/clients/config) remains backend-agnostic and accessible via `CoreCtx` only.
