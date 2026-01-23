## 2026-01-23 - XSync vs XFlush in drw_map
**Learning:** In X11 window managers, using XSync in frequent drawing paths (like drw_map) causes blocking round-trips to the X server, significantly degrading performance.
**Action:** Use XFlush for drawing operations where synchronous confirmation is not required.
