# Protocol-aware focus/input plan

## Background

The Minecraft/XWayland pointer-lock bug was not caused by missing pointer
constraints. The compositor exported the relevant globals, and a raw X11
`XGrabPointer` test could confine the pointer. The failure was earlier:
GLFW/X11 never attempted the pointer grab because it did not believe its window
was focused.

The root cause was that instantWM represented keyboard focus as
`KeyboardFocusTarget::Window(Window)` and then forwarded focus through
`Window::wl_surface()`. For native Wayland this is fine. For XWayland it skips
Smithay's `KeyboardTarget` implementation for `X11Surface`, which is where X11
`SetInputFocus` and `WM_TAKE_FOCUS` are sent.

So the project was not losing the XWayland data entirely. It was retaining it
on the Smithay `Window`, but input/focus code erased the protocol distinction
too early by treating the backing `wl_surface` as the universal target.

## Design goal

Make protocol-sensitive operations impossible to route through the wrong
surface by accident.

Rendering, hit testing, pointer delivery, keyboard focus, activation, configure,
close, and metadata are related but not identical operations. A native Wayland
window and an XWayland window may both expose a `wl_surface`, but that does not
mean the `wl_surface` is the correct semantic target for every operation.

## Immediate state

The narrow fix is to branch inside `KeyboardFocusTarget::Window`:

- If the `Window` has an `X11Surface`, call the `X11Surface` keyboard target.
- Otherwise, call the native Wayland `wl_surface` keyboard target.

That should remain as the minimal bug fix. The broader plan below is about
making this class of bug harder to reintroduce.

## Plan

### 1. Introduce protocol-aware managed window handles

Create a local abstraction that keeps common window identity separate from the
protocol-specific handle:

```rust
pub enum ManagedWindowKind {
    Wayland {
        window: smithay::desktop::Window,
    },
    XWayland {
        window: smithay::desktop::Window,
        x11: smithay::xwayland::X11Surface,
    },
}

pub struct ManagedWindow {
    pub id: WindowId,
    pub kind: ManagedWindowKind,
}
```

This does not need to replace Smithay `Window` in `Space`. Smithay `Window` can
remain the rendering/layout object. The new wrapper should be the semantic
object used by focus, input routing, activation, configure, and policy code.

### 2. Add intent-specific accessors

Avoid generic helpers at call sites where the intended operation matters.
Prefer methods with names that encode the protocol contract:

```rust
impl ManagedWindow {
    fn root_wl_surface(&self) -> Option<WlSurface>;
    fn keyboard_focus_target(&self) -> KeyboardFocusTarget;
    fn pointer_focus_target_at(&self, point: Point<f64, Logical>) -> Option<PointerFocusTarget>;
    fn set_activated(&self, active: bool) -> bool;
    fn configure(&self, rect: Rectangle<i32, Logical>);
    fn close(&self);
}
```

The important rule: `root_wl_surface()` is for tree identity, rendering,
constraints, and hit testing. It is not automatically the keyboard focus target.

### 3. Split focus target variants by protocol

Replace the protocol-erased variant:

```rust
KeyboardFocusTarget::Window(Window)
```

with protocol-aware variants:

```rust
pub enum KeyboardFocusTarget {
    WaylandWindow(Window),
    XWaylandWindow(X11Surface),
    WlSurface(WlSurface),
    Popup(PopupKind),
}
```

This makes the keyboard focus implementation straightforward and reviewable:

- `WaylandWindow` forwards to the Wayland surface path.
- `XWaylandWindow` forwards to `X11Surface`, preserving X11 focus side effects.
- `WlSurface` remains for layer surfaces and explicit raw Wayland targets.
- `Popup` remains popup-specific.

### 4. Do the same audit for pointer focus, activation, and configure

Keyboard focus was the confirmed bug, but the same abstraction risk exists
elsewhere. Audit these operations:

- `KeyboardTarget` dispatch
- `PointerTarget` dispatch
- `set_activated`
- configure/resize
- close requests
- PID/startup metadata
- transient/parent lookup
- pointer constraints and relative pointer interaction

For each operation, decide whether the semantic target is:

- the Smithay desktop `Window`
- the root `wl_surface`
- a subsurface
- the `X11Surface`
- a protocol-specific shell object

Then encode that decision in helper names and types.

### 5. Add diagnostics at protocol boundaries

Add targeted debug logs where protocol side effects happen:

```text
focus: win=... protocol=xwayland action=set_input_focus
focus: win=... protocol=wayland action=wl_keyboard_enter
activate: win=... protocol=xwayland active=true
```

These logs should be low-noise and only around places where a generic WM action
crosses into Wayland or X11 protocol behavior.

### 6. Add a small XWayland focus probe

Create a debug/test client or documented manual probe that verifies:

- an X11 window maps under instantWM's Wayland backend
- focusing it sends X11 focus
- the client receives `FocusIn`
- `XGetInputFocus` reports the client window
- a GLFW/X11 disabled-cursor probe actually calls `XGrabPointer`

This does not have to be a full CI integration immediately. Even an ignored
test or checked-in `tools/` probe would have shortened this investigation.

### 7. Keep pointer-constraint fixes separate

Do not bundle broad pointer-constraint rewrites with focus fixes. Pointer lock
depends on focus for GLFW/Minecraft, so failures can look like constraint bugs
even when the client never requested a grab.

Future pointer-constraint work should come with a test that proves the client
actually requested a lock/grab first. Otherwise we risk changing compositor
constraint behavior to compensate for an upstream focus/input routing problem.

## Migration strategy

1. Keep the current narrow `X11Surface` keyboard-target fix.
2. Introduce protocol-aware helper methods without changing behavior.
3. Convert focus code to use those helpers.
4. Convert activation/configure/close paths.
5. Convert pointer routing only after focus behavior is covered by probes.
6. Remove direct `Window::wl_surface()` use from protocol-sensitive code.

Direct `Window::wl_surface()` use should remain acceptable in rendering,
surface-tree traversal, and hit testing, where the operation really is about
Wayland surface identity.

## Success criteria

- A reviewer can tell from types whether a focus operation targets Wayland or
  XWayland.
- XWayland keyboard focus cannot silently bypass `X11Surface`.
- Pointer-lock failures can be diagnosed by first checking whether the client
  received X11 focus and attempted a grab.
- The fix for Minecraft remains a small focus/input routing fix, not a broad
  pointer-constraint rewrite.
