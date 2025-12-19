# instantWM Architecture

> A guide to the codebase structure for developers and AI agents.

## Quick Reference

```
instantwm.c  [2493 lines] - Entry point, core WM logic, config.h consumer
├── Core Window Management
│   ├── client.c     - Client/window lifecycle (attach, detach, resize, configure)
│   ├── monitors.c   - Multi-monitor handling, monitor creation/cleanup
│   ├── tags.c       - Tag/workspace management, tag switching, swapping
│   └── focus.c      - Focus management, direction-based focus
├── Input Handling
│   ├── events.c     - X11 event handlers (map, unmap, property, motion, etc.)
│   └── mouse.c      - Mouse interactions (move, resize, gestures, drag)
├── User Interface
│   ├── bar.c        - Status bar drawing, click handling
│   ├── drw.c        - Low-level drawing primitives (text, rectangles, fonts)
│   └── systray.c    - System tray implementation
├── Window Features
│   ├── floating.c   - Floating window management, snap positions
│   ├── scratchpad.c - Scratchpad/dropdown terminal functionality
│   ├── overlay.c    - Overlay window (slide-in panels)
│   ├── animation.c  - Window animation effects
│   └── layouts.c    - Tiling layouts (tile, monocle, grid, etc.)
├── Utilities
│   ├── toggles.c    - Toggle functions (floating, fullscreen, sticky, etc.)
│   ├── push.c       - Stack manipulation (push up/down, swap)
│   ├── xresources.c - X resources loading (theme colors)
│   └── util.c       - General utilities (die, ecalloc)
└── Configuration
    └── config.h     - User configuration (keys, rules, colors, commands)
```

## Module Dependency Graph

```
                    ┌─────────────┐
                    │ instantwm.c │ (includes config.h)
                    └──────┬──────┘
                           │
        ┌──────────────────┼──────────────────┐
        │                  │                  │
        ▼                  ▼                  ▼
   ┌─────────┐      ┌───────────┐      ┌───────────┐
   │ events.c│      │  mouse.c  │      │   bar.c   │
   └────┬────┘      └─────┬─────┘      └─────┬─────┘
        │                 │                  │
        └────────────┬────┴──────────────────┘
                     ▼
              ┌─────────────┐
              │  client.c   │
              │  focus.c    │
              │  tags.c     │
              │  monitors.c │
              └──────┬──────┘
                     │
        ┌────────────┼────────────┐
        ▼            ▼            ▼
   ┌─────────┐ ┌──────────┐ ┌─────────┐
   │floating │ │scratchpad│ │ overlay │
   └─────────┘ └──────────┘ └─────────┘
```

## Key Architectural Patterns

### 1. Static Config Arrays (config.h constraint)

Functions that use these `static` arrays **must stay in instantwm.c**:
- `keys[]` - Keyboard bindings → `keypress()`, `grabkeys()`
- `buttons[]` - Mouse bindings → `buttonpress()`, `grabbuttons()`
- `rules[]` - Window rules → `applyrules()`
- `commands[]` - IPC commands → `xcommand()`

### 2. Extern Variable Pattern

Modules access shared state via `extern` declarations:
```c
// In module.c
extern Display *dpy;
extern Monitor *selmon;
extern int bh;  // bar height
```

### 3. Global State (defined in instantwm.c)

| Variable | Type | Purpose |
|----------|------|---------|
| `dpy` | `Display*` | X11 display connection |
| `root` | `Window` | Root window |
| `selmon` | `Monitor*` | Currently selected monitor |
| `mons` | `Monitor*` | Linked list of all monitors |
| `drw` | `Drw*` | Drawing context |
| `bh` | `int` | Bar height in pixels |
| `sw`, `sh` | `int` | Screen width/height |

### 4. Important Enums (instantwm.h)

| Enum | Purpose |
|------|---------|
| `SCRATCHPAD_MASK` | Tag mask for scratchpad windows (1 << 20) |
| `SnapNone..SnapMaximized` | Window snap positions (0-9) |
| `OverlayTop..OverlayLeft` | Overlay slide directions (0-3) |
| `GestureNone..GestureStartMenu` | Bar gesture states |
| `RuleTiled..RuleScratchpad` | Rule floating modes for config.h |

## File Purposes (Detailed)

### instantwm.c (Entry Point)
- `main()` - Entry point, X11 connection, main loop
- `setup()` - Initialize atoms, cursors, colors, bars
- `run()` - Main event loop
- `manage()`/`unmanage()` - Window lifecycle
- `applyrules()` - Apply config.h rules to new windows
- `buttonpress()`/`keypress()` - Input handlers (use static config arrays)

### client.c (Window Operations)
- `attach()`/`detach()` - Add/remove from client list
- `configure()` - Send configure events to clients
- `resize()`/`resizeclient()` - Change window geometry
- `setfullscreen()` - Handle fullscreen state
- `updatetitle()`/`updatewmhints()` - Sync window properties

### events.c (X11 Events)
- `clientmessage()` - EWMH messages (fullscreen, activate, systray)
- `configurerequest()` - Window wants to resize/move
- `enternotify()` - Mouse entered window (focus follows mouse)
- `maprequest()` - New window wants to be shown
- `propertynotify()` - Window property changed
- `motionnotify()` - Mouse movement (hover effects, resize cursors)

### mouse.c (Mouse Interactions)
- `movemouse()` - Drag window to move
- `resizemouse()` - Drag to resize
- `gesturemouse()` - Desktop gestures (volume, onboard)
- `drawwindow()` - Slop-based window drawing
- `dragtag()` - Drag windows between tags

### bar.c (Status Bar)
- `drawbar()` - Main bar rendering
- `drawstatusbar()` - Parse and render status text with markup
- `updatestatus()` - Read root window name for status
- Click helpers: `handle_bar_click()`, `handle_tag_click()`

### tags.c (Workspaces)
- `view()` - Switch to tag
- `toggleview()` - Toggle tag visibility
- `tag()` - Move window to tag
- `swaptags()` - Swap two tags' contents
- `toggle_overview()` - Overview mode toggle

### floating.c (Floating Windows)
- `changesnap()` - Snap window to screen edges/corners
- `applysnap()` - Apply snap position geometry
- `resetsnap()` - Restore original floating position
- `savefloating()`/`restorefloating()` - Save/restore float geometry

### scratchpad.c (Dropdown Terminal)
- `togglescratchpad()` - Show/hide scratchpad
- `makescratchpad()` - Convert window to scratchpad
- `removescratchpad()` - Remove scratchpad status

### layouts.c (Tiling)
- `tile()` - Master/stack layout
- `monocle()` - Fullscreen layout
- `grid()` - Grid layout
- `tcl()` - Three-column layout
- `overviewlayout()` - Overview/expose mode

## Build System

```makefile
# Simple flat compilation
OBJ = drw.o instantwm.o layouts.o util.o overlay.o animation.o \
      floating.o mouse.o scratchpad.o bar.o systray.o tags.o \
      xresources.o toggles.o focus.o monitors.o client.o events.o

instantwm: $(OBJ)
    $(CC) -o $@ $(OBJ) $(LDFLAGS)
```

## For AI Agents

When working on this codebase:

1. **Finding code**: Use `grep -l "pattern" *.c` to find relevant files
2. **Understanding flow**: Start from `instantwm.c` and trace outward
3. **Adding features**: Check if similar functionality exists in a module
4. **Config changes**: Only `instantwm.c` can use `config.h` static arrays
5. **New modules**: Follow the `.c/.h` pair pattern with `extern` declarations
