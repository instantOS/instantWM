## 2024-05-23 - X11 Drawing Optimization
**Learning:** In X11 window managers, replacing `XSync(dpy, False)` with `XFlush(dpy)` in high-frequency paths like drawing (`drw_map`) and resizing (`resizeclient`) can significantly improve perceived responsiveness. `XSync` blocks until the server processes requests, whereas `XFlush` just sends them.
**Action:** Always check `drw_map` and interactive resize functions for unnecessary `XSync` calls in X11-based projects.
