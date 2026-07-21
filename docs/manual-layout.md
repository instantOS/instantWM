# Manual tree layout

instantWM's tiled mode is a persistent, weighted tree. It is shared by the X11
and Wayland backends and stored independently for every monitor and visible tag
mask. Arrange passes do not recreate an automatic layout.

`vertical` splits contain left-to-right children; `horizontal` splits contain
top-to-bottom children. Splits are n-ary and canonical: adjacent splits on the
same axis are folded into one run, weights are positive and normalized, and an
empty or single-child split is collapsed. Newly tiled windows split the least
populated branch, alternating axes.

## Commands and default keys

Most old layout names now describe one-shot transformations:

- `layout_tile` rewrites the current tree as master/stack.
- `layout_grid` and `layout_horiz_grid` rewrite it as column-first and
  row-first grids.
- `layout_bottom_stack` and `layout_bstack_horiz` create bottom-stack trees.
- `layout_maximized` is a persistent presentation mode: every tiled window
  fills the work area and the focused tiled window is stacked on top. The
  underlying manual tree is preserved and reconciled while the mode is active.
  `layout_monocle` remains an input compatibility alias.
- `layout_float` remains a persistent floating mode.

After a transformation, manual swaps, resizes, spawns, and pointer placements
remain in effect. A later arrange does not run the transformation again. The
same applies to `instantwmctl layout <name>`.

The default Super bindings use the tree whenever the focused window is tiled:

- `Super+Arrow` and `Super+H/J/K/L` focus across the first structural seam.
  Geometry only chooses between leaves which share that seam. At the left or
  right boundary (including an empty or single-window tag), the horizontal
  commands switch to the adjacent tag instead.
- `Super+Shift+Arrow` swaps with a visual neighbour without changing split
  topology. `Super+Shift+H/J/K/L` is retained as an alternative.
- `Super+Ctrl+Arrow` resizes an axis run while preserving peer ratios.
  `Super+Alt+H/J/K/L` is retained as an alternative.
- `Super++` and `Super+-` grow or shrink along the most local split.
- `Super+M` enters keyboard placement for a tiled window when its tree has at
  least one other destination. The physical pointer and window focus remain
  unchanged; a thick hollow frame previews the exact final window rectangle on
  both Wayland and X11. Arrow or Vim `h/j/k/l` moves
  geometrically; Shift plus a direction swaps the armed window with its visual
  neighbour, and Ctrl plus a direction resizes it. Tab/Shift+Tab visits every
  candidate, Space selects the current window's centre swap, and Enter applies.
  Escape or any unrelated key cancels without passing that key to the client.
  This deliberately uses Super to enter the mode; the browser prototype could
  not reserve that modifier.

Placement candidates are normalized by their resulting leaf order and the
visible source-window preview. Equivalent descriptions of one seam—such as
“right of A” and “left of B”—therefore appear as one target, and structural
scope choices with substantially overlapping previews do not create visually
redundant navigation steps.

When tiled size hints are enabled, placement candidates are also checked
against every affected client's minimum outer size before they are shown.
Feasible trees redistribute split space to meet those minima; an impossible
candidate is omitted, so its preview can never commit overlapping or
off-monitor tiled geometry. Maximum sizes, aspect constraints, and resize
increments are applied inside the reserved slot. If the current tree itself
becomes impossible to satisfy (for example after moving it to a smaller
monitor), containment wins: instantWM keeps gapless bounded slots and ignores
the contradictory hints for that arrange pass.

Keyboard placement is the built-in `placement` WM mode, not a separate input
state. It is visible in the bar and `instantwmctl mode list`; changing modes by
IPC or a binding cancels placement and removes its preview. IPC cannot enter
`placement` directly because entry needs a validated source window and target
set; use the `begin_tree_placement` action. Floating, maximized, and lone tiled
windows decline the action without starting a pointer drag. Its commands are
normal named actions (`placement_left`, `placement_swap_left`,
`placement_resize_left`, `placement_next`, `placement_center`,
`placement_apply`, `placement_cancel`, and their directional variants), so
defaults can be replaced in TOML:

```toml
[modes.placement]
description = "place window"

[[modes.placement.keybinds]]
modifiers = []
key = "a"
action = "placement_left"
```

Super is ignored as an entry modifier while this mode is active, allowing it
to remain held after Super+M. Other modifiers remain meaningful. An explicit
`none` action can make an otherwise unrelated key a consumed no-op; an unbound
non-modifier cancels placement.

`Super+W` (`toggle_tiling_maximized`) toggles maximized presentation. Pressing
it again returns to the unchanged manual tree. `Super+J/K` cycles tiled windows
in stable tree-leaf order while maximized. The bar presents those tiled titles
in the same order, so J moves right and K moves left through the visible title
sequence, wrapping at its ends. Floating windows remain separate overlays and
their titles follow the tiled sequence. The broken per-window `Super+Ctrl+F`
maximize binding has been removed.

`Super+T` toggles the default edge overlay (`edge_scratchpad_toggle`), while
`Super+Ctrl+T` creates it from the focused window. The master-stack tree preset
remains available as `layout_tile` and from the layout-symbol menu rather than
occupying the direct `Super+T` binding.

The existing action names (`focus_left`, `key_move_left`, `key_resize_left`,
and their other directions) work in custom TOML bindings. The direction-free
names are `tree_grow` and `tree_shrink`.

## Pointer placement

Dragging a tiled window no longer converts it to floating. The source stays in
the original tree until release, making cancellation lossless. Dropping in
another tiled window's centre swaps slots. Dropping in an edge band reparents
the source on that side. Bands proceed from wide ancestor/aligned-seam scopes
at the outside edge toward local scopes farther inward. Continuous grid seams
and contiguous virtual child ranges are valid; a leaf crossing the seam
invalidates it.

While dragging, a thick hollow frame shows the source window's exact final
outer rectangle for the target under the pointer. The frame disappears over
the tag bar and screen-edge drop zones, whose existing actions take precedence.
Dropping a window on a tag moves it there without changing the current view.
Hold Alt when releasing to move the window and follow it to that tag. The same
release-time rule applies when dragging a tag indicator: plain drag moves the
selected window, while Alt-drag moves and follows. Modifiers may be changed at
any point during the gesture.

The gesture is implemented once in the backend-neutral layout layer. X11's
synchronous drag and Wayland's asynchronous pointer interaction call the same
drop command. Floating windows retain direct movement, and screen-edge/tag-bar
drops retain their existing meanings.

`Super+right-drag` on a tiled window resizes the tree seam nearest the grabbed
edge. Space is transferred between the source and all siblings on the grabbed
edge's side, preserving those siblings' existing ratios. Siblings across the
window's opposite edge stay fixed, so only the physical edge being dragged
moves. The seam stops at the affected branches' minimum-size constraints.
Floating windows retain ordinary free resizing, while a lone tiled window or
maximized presentation still falls back to floating resize behavior.

## Configuration

The `[layout]` section also accepts:

```toml
[layout]
keyboard_resize_step = 0.05
minimum_weight = 0.15
pointer_edge_fraction = 0.34
maximized_gaps = false
```

The resize step is the fraction transferred per command. The minimum bounds
children where the run size permits it. The pointer fraction controls semantic
edge-band depth. Values are clamped to safe ranges when loaded.
`maximized_gaps` controls whether maximized tiled windows retain the configured
outer gap; the old `monocle_gaps` spelling remains accepted.

## Lifecycle rules

Before computing geometry, the tree reconciles against visible tiled clients.
Closing, floating, retagging, or moving a window removes its leaf and collapses
its parent. New or newly tiled windows use balanced insertion. Surviving
topology and weights remain. Exact tag-mask ownership means tag 1, tag 2, and
the combined tag 1+2 view can remember independent arrangements without giving
one window two positions in a single tree.
