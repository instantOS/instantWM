# Abstraction Plan

## Goal / Scope
- Build a dual backend architecture: smithay/niri Wayland backend plus X11 WM backend.
- Preserve current X11 behavior while enabling a new Wayland path.

## Constraints
- No git usage.
- Preserve current behavior at each step.
- Incremental, low-risk refactors only.

## Current Coupling Hotspots
- `rust/src/wm.rs`
- `rust/src/contexts.rs`
- `rust/src/events.rs`
- `rust/src/client/lifecycle.rs`
- `rust/src/monitor.rs`
- `rust/src/bar/x11.rs`
- `rust/src/systray.rs`
- `rust/src/drw/draw.rs`
- Types involving `Window` (x11 types embedded in core data structures)

## Phased Plan
- [x] **Inventory**: map backend-touching flows, list public surfaces, identify ownership/borrowing pain points.
- [ ] **Backend traits**: define minimal trait surfaces for display, input, render, and window ops.
- [ ] **Type decoupling**: replace x11 `Window` in core types with backend-agnostic IDs.
- [ ] **Service interfaces**: add adapters for bar, systray, drawing, monitor plumbing.
- [ ] **X11 implementations**: implement traits/adapters for existing X11 code paths.
- [ ] **Wayland scaffolding**: introduce smithay/niri wiring behind the same traits (no feature parity required yet).

## Progress Log
- 2026-02-26: Inventory complete; hotspot list captured and initial flow map drafted.
- 2026-02-26: Introduced `WindowHandle` alias; core type modules now use it (initial step toward type decoupling).
