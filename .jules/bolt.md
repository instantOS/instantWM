
## 2026-01-27 - Optimize status bar rendering
**Learning:** In C window managers, `malloc`/`free` in the rendering loop (like `drawstatusbar`) adds unnecessary overhead.
**Action:** Use fixed-size stack buffers (e.g., `char buf[1024]`) when bounds are known (like `stext` global).
