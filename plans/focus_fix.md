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

### 1. Keep Smithay `Window` as the window abstraction

Do not add a second broad `ManagedWindow` wrapper for this problem. Smithay's
`desktop::Window` already preserves the protocol-specific object internally:

- `Window::x11_surface()` exposes the `X11Surface`.
- `Window::wl_surface()` exposes the backing Wayland surface.
- `Window::set_activated()` already dispatches by protocol.

The bug was not that the XWayland data was missing. The bug was that a
protocol-sensitive path chose the generic backing `wl_surface` when it needed
the `X11Surface` keyboard target.

Adding another full window wrapper would duplicate Smithay's abstraction and
increase the amount of state future maintainers need to reason about. The
better fix is to make operation-specific routing explicit where the operation
has protocol side effects.

### 2. Add intent-specific helpers on or near focus/input code

Avoid direct `Window::wl_surface()` calls at call sites where the intended
operation matters. Prefer small helpers with names that encode the protocol
contract:

```rust
fn keyboard_focus_target_for(window: &Window) -> KeyboardFocusTarget;
fn root_wl_surface_for_tree_identity(window: &Window) -> Option<WlSurface>;
fn pointer_focus_target_at(window: &Window, point: Point<f64, Logical>) -> Option<PointerFocusTarget>;
```

The important rule: the root `wl_surface` is for tree identity, rendering,
constraints, hit testing, and pointer delivery. It is not automatically the
keyboard focus target.

These helpers should be deliberately narrow. If Smithay already dispatches an
operation correctly, such as `Window::set_activated()`, keep using Smithay
directly rather than wrapping it.

### 3. Centralize keyboard focus routing by protocol

Keep using Smithay's protocol-preserving `Window` wrapper:

```rust
KeyboardFocusTarget::Window(Window)
```

Inside the `KeyboardTarget` implementation, resolve the actual keyboard target
through one local helper. The invariant must be local and obvious:

```rust
if let Some(x11) = window.x11_surface() {
    KeyboardTarget::enter(x11, ...);
} else if let Some(surface) = window.wl_surface() {
    KeyboardTarget::enter(surface.as_ref(), ...);
}
```

This avoids reinventing Smithay's `Window` abstraction while still preventing
keyboard focus from silently taking the wrong route. Splitting
`KeyboardFocusTarget::Window` into protocol-specific variants should only be
reconsidered if the centralized helper proves too easy to bypass.

### 4. Audit other protocol-sensitive operations without preemptive wrappers

Keyboard focus was the confirmed bug, but the same abstraction risk exists
elsewhere. Audit these operations:

- `KeyboardTarget` dispatch
- `PointerTarget` dispatch
- `DndFocus` dispatch
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

Then encode that decision only where it is not already correctly handled by
Smithay. Do not wrap for the sake of wrapping.

The same pattern applies to drag-and-drop: Smithay has a distinct `DndFocus`
implementation for `X11Surface` to bridge XDND. A `PointerFocusTarget::Window`
must therefore route DND through `X11Surface` for XWayland windows, while
ordinary pointer events can still use the backing `wl_surface`.

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
2. Add a comment/invariant near `KeyboardFocusTarget` explaining why XWayland
   keyboard focus must route through `X11Surface`.
3. Centralize the `Window` to actual-keyboard-target decision in one helper.
4. Add targeted helper methods only for protocol-sensitive paths that currently
   need direct branching.
5. Convert pointer routing only after focus behavior is covered by probes.
6. Remove direct `Window::wl_surface()` use from protocol-sensitive code where
   the operation is not actually about Wayland surface identity.

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
