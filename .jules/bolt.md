## 2026-01-26 - XSync Overuse in Drawing Paths
**Learning:** The codebase uses `XSync` extensively after drawing operations (e.g., in `drw_map`, `resizeclient`), forcing synchronous round-trips to the X server. This is a significant performance bottleneck, especially for frequent operations like bar updates or animations.
**Action:** When optimizing X11 code, look for `XSync` calls in hot paths and verify if they can be safely removed (relying on implicit flushing or explicit `XFlush`).
