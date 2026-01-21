## 2026-01-21 - XSync vs XFlush in Drawing
**Learning:** `drw_map` currently uses `XSync(drw->dpy, False)`, which forces a round-trip to the X server. This is good for debugging but bad for performance. `XFlush` or relying on the event loop is usually sufficient for drawing operations.
**Action:** In future optimizations, consider replacing `XSync` with `XFlush` in drawing paths, but be aware of synchronization requirements. For now, optimizing memory allocation in `drawstatusbar` is safer and cleaner.
